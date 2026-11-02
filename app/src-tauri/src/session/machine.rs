//! 会话里**不碰 I/O** 的那一部分：候选行的构建与排序、选中项怎么变、
//! 迟到的结果怎么作废。
//!
//! `Ctl` 负责「调用谁」（引擎、HUD、上屏、广播），这里负责「判断什么」。
//! 拆开的原因是：这些判断最容易回归，却最不容易手测——多台设备、渠道失败、
//! 用户手选与自动切换打架，靠人工按是测不完的。

use std::collections::HashMap;

use crate::session::types::{Row, RowState, Segments};
use crate::settings::Channel;

/// 代际与超时的票根。
///
/// 一句话的识别是并发的：引擎可能在用户已经开了下一句之后才回结果。
/// 每个结果带着发出时的代号，代号对不上就作废——这就是「迟到的结果不会串到新句子里」。
#[derive(Default)]
pub struct Tickets {
    generation: u64,
    timeout: u64,
}

impl Tickets {
    /// 开新的一句：旧的在途结果全部作废
    pub fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 结果还属于当前这一句吗
    pub fn accepts(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// 重新计时：返回新的票号，旧的超时定时器随之失效
    pub fn next_timeout(&mut self) -> u64 {
        self.timeout += 1;
        self.timeout
    }

    pub fn timeout_is_current(&self, token: u64) -> bool {
        token == self.timeout
    }
}

/// 手机上点了某条候选之后该做什么
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// 已定稿，可以上屏
    Commit,
    /// 还在识别：只切换焦点，等它定稿
    Select,
    /// 不可用（失败或未下载）：抖动提示，让用户重选
    Shake,
}

/// 多渠道候选面板的全部状态
pub struct Picker {
    rows: Vec<Row>,
    segs: Vec<Segments>,
    sel: usize,
    /// 用户自己点过某一行：此后不再自动把焦点挪走
    user_picked: bool,
}

impl Picker {
    /// 按「上次用的渠道」优先、其次按历史使用次数排序建行。
    /// `ready` 用来判断渠道此刻能不能用（离线模型没下载就不该参与）。
    pub fn new(
        mut channels: Vec<Channel>,
        ready: impl Fn(Channel) -> bool,
        last_pick: Option<Channel>,
        counts: &HashMap<Channel, u32>,
    ) -> Self {
        channels.sort_by_key(|c| std::cmp::Reverse(counts.get(c).copied().unwrap_or(0)));
        let rows: Vec<Row> = channels
            .into_iter()
            .map(|ch| {
                let skip = !ready(ch);
                Row {
                    channel: ch,
                    text: if skip { "离线模型未下载，已跳过".into() } else { String::new() },
                    state: if skip { RowState::Skip } else { RowState::Listen },
                    ms: None,
                }
            })
            .collect();
        let segs = rows.iter().map(|_| Segments::default()).collect();
        let mut picker = Self { rows, segs, sel: 0, user_picked: false };
        // 默认焦点停在「上次用的那一个」上，减少一次点击
        if let Some(i) = picker.rows.iter().position(|r| Some(r.channel) == last_pick && r.state == RowState::Listen)
        {
            picker.sel = i;
            picker.user_picked = true;
        }
        picker
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn sel(&self) -> usize {
        self.sel
    }

    pub fn index_of(&self, ch: Channel) -> Option<usize> {
        self.rows.iter().position(|r| r.channel == ch)
    }

    /// 可参与识别的行（跳过未下载模型的），每行开一段新音频
    pub fn start_round(&mut self) -> Vec<(Channel, usize)> {
        let mut targets = Vec::new();
        for (row, segs) in self.rows.iter_mut().zip(&mut self.segs) {
            if row.state == RowState::Skip {
                continue;
            }
            targets.push((row.channel, segs.start()));
            row.state = RowState::Listen;
            row.text = segs.text();
        }
        targets
    }

    pub fn on_partial(&mut self, ch: Channel, seg: usize, text: String) {
        let Some(i) = self.index_of(ch) else { return };
        if self.segs[i].partial(seg, text).is_some() {
            self.rows[i].text = self.segs[i].text();
        }
    }

    /// 某一行定稿。返回需要把焦点挪过去的目标（没有就 None）。
    ///
    /// 规则：用户没手选过、或当前焦点那条已经废了，就自动落到第一条成功的行上。
    pub fn on_done(&mut self, ch: Channel, seg: usize, res: Result<String, String>, ms: Option<u64>) -> Option<usize> {
        let i = self.index_of(ch)?;
        self.segs[i].done(seg, res, false);
        let text = self.segs[i].text();
        if !self.segs[i].finished() {
            self.rows[i].text = text;
            return None;
        }
        if !text.is_empty() {
            self.rows[i].text = text;
            self.rows[i].state = RowState::Final;
            self.rows[i].ms = Some(ms.unwrap_or(0));
        } else {
            self.rows[i].text = self.segs[i].error.clone().unwrap_or_else(|| "没有识别到内容".into());
            self.rows[i].state = RowState::Error;
        }

        let current = self.rows[self.sel].state;
        let stale = !self.user_picked || matches!(current, RowState::Error | RowState::Skip);
        if self.rows[i].state == RowState::Final && stale && current != RowState::Final {
            self.sel = i;
            return Some(i);
        }
        None
    }

    /// 松手：所有还在听的行转入等待定稿
    pub fn release(&mut self) {
        for row in &mut self.rows {
            if row.state == RowState::Listen {
                row.state = RowState::Wait;
            }
        }
    }

    /// 点了一条候选。识别中的只切焦点，失败的要抖一下，定稿的才算数。
    pub fn choose(&mut self, i: usize) -> Choice {
        let Some(row) = self.rows.get(i) else { return Choice::Shake };
        let choice = match row.state {
            RowState::Final => Choice::Commit,
            RowState::Listen | RowState::Wait => Choice::Select,
            RowState::Error | RowState::Skip => Choice::Shake,
        };
        // 没到定稿的点击也算「用户表过态」：之后不再自动把焦点挪走
        if choice != Choice::Commit {
            self.sel = i;
            self.user_picked = true;
        }
        choice
    }

    /// 取出某一行的正文与渠道（提交时用）
    pub fn take(&self, i: usize) -> Option<(String, Channel)> {
        self.rows.get(i).map(|r| (r.text.clone(), r.channel))
    }

    /// 焦点所在行的正文与渠道（超时打断时补写用）
    pub fn selected(&self) -> Option<(String, Channel)> {
        self.take(self.sel)
    }

    /// 所有行都不再「在识别中」——超时定时器可以撤了
    pub fn all_settled(&self) -> bool {
        !self.rows.iter().any(|r| matches!(r.state, RowState::Listen | RowState::Wait))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(pairs: &[(Channel, u32)]) -> HashMap<Channel, u32> {
        pairs.iter().copied().collect()
    }

    fn picker(chs: Vec<Channel>) -> Picker {
        Picker::new(chs, |_| true, None, &HashMap::new())
    }

    #[test]
    fn orders_by_history() {
        let p = Picker::new(
            vec![Channel::Doubao, Channel::Qwen, Channel::Wetype],
            |_| true,
            None,
            &counts(&[(Channel::Qwen, 9), (Channel::Wetype, 3)]),
        );
        let order: Vec<Channel> = p.rows().iter().map(|r| r.channel).collect();
        assert_eq!(order, vec![Channel::Qwen, Channel::Wetype, Channel::Doubao]);
    }

    #[test]
    fn skips_channels_whose_model_is_missing() {
        let p = Picker::new(vec![Channel::Doubao, Channel::WetypeOffline], |_| false, None, &HashMap::new());
        assert!(p.rows().iter().all(|r| r.state == RowState::Skip));
        // 全都不可用时也不该有可识别的目标
        let mut p = p;
        assert!(p.start_round().is_empty());
    }

    #[test]
    fn last_pick_moves_the_focus_and_resists_stealing() {
        let mut p = Picker::new(vec![Channel::Doubao, Channel::Qwen], |_| true, Some(Channel::Qwen), &HashMap::new());
        assert_eq!(p.sel(), 1);
        // 沿用上次选择也算「表过态」：别的行定稿不该把焦点抢走
        let seg = p.start_round()[0].1;
        p.on_done(Channel::Doubao, seg, Ok("别的渠道".into()), None);
        assert_eq!(p.sel(), 1, "上次选择的行不该被自动切换顶掉");
    }

    #[test]
    fn first_success_takes_focus_when_nobody_picked() {
        let mut p = picker(vec![Channel::Doubao, Channel::Qwen]);
        let targets = p.start_round();
        let seg = targets[0].1;
        // 第二行先成功：焦点应当跟着走
        assert_eq!(p.on_done(Channel::Qwen, seg, Ok("你好".into()), Some(120)), Some(1));
        assert_eq!(p.sel(), 1);
    }

    #[test]
    fn keeps_user_selection() {
        let mut p = picker(vec![Channel::Doubao, Channel::Qwen]);
        let targets = p.start_round();
        let seg = targets[0].1;
        p.on_done(Channel::Doubao, seg, Ok("第一行".into()), None);
        assert_eq!(p.sel(), 0);
        // 用户手点第一行之后，第二行定稿不该把焦点抢走
        assert_eq!(p.choose(0), Choice::Commit);
        p.on_done(Channel::Qwen, seg, Ok("第二行".into()), None);
        assert_eq!(p.sel(), 0, "用户选过的行不该被自动切换顶掉");
    }

    #[test]
    fn failed_row_shakes_instead_of_committing() {
        let mut p = picker(vec![Channel::Doubao]);
        let targets = p.start_round();
        p.on_done(Channel::Doubao, targets[0].1, Err("超时".into()), None);
        assert_eq!(p.rows()[0].state, RowState::Error);
        assert_eq!(p.choose(0), Choice::Shake);
        assert_eq!(p.take(0).unwrap().0, "超时");
    }

    #[test]
    fn listening_row_only_moves_focus() {
        let mut p = picker(vec![Channel::Doubao, Channel::Qwen]);
        p.start_round();
        assert_eq!(p.choose(1), Choice::Select);
        assert_eq!(p.sel(), 1);
    }

    #[test]
    fn release_turns_listening_into_waiting() {
        let mut p = picker(vec![Channel::Doubao]);
        p.start_round();
        p.release();
        assert_eq!(p.rows()[0].state, RowState::Wait);
        assert!(!p.all_settled());
        p.on_done(Channel::Doubao, 0, Ok("好".into()), None);
        assert!(p.all_settled());
    }

    #[test]
    fn stale_generation_is_rejected() {
        let mut t = Tickets::default();
        let first = t.next_generation();
        assert!(t.accepts(first));
        let second = t.next_generation();
        assert!(!t.accepts(first), "上一句的结果不能串进新句子");
        assert!(t.accepts(second));
    }

    #[test]
    fn stale_timeout_is_rejected() {
        let mut t = Tickets::default();
        let old = t.next_timeout();
        assert!(t.timeout_is_current(old));
        let new = t.next_timeout();
        assert!(!t.timeout_is_current(old), "旧的超时定时器必须失效");
        assert!(t.timeout_is_current(new));
    }
}
