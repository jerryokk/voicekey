//! 会话的纯数据类型：不依赖 Tauri，可以被单独测试。
//! 这些结构是 controller 与前端之间的契约（序列化成 rows/partial 消息），
//! 也是「接着说」「多渠道候选」这些状态的基础。

use crate::settings::Channel;
use serde::Serialize;
use voicekey_core::Audio;
use voicekey_remote::Event as RemoteEvent;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum RowState {
    Listen,
    Wait,
    Final,
    Error,
    Skip,
}

#[derive(Clone, Serialize)]
pub struct Row {
    pub channel: Channel,
    pub text: String,
    pub state: RowState,
    pub ms: Option<u64>,
}

#[derive(Clone, Serialize)]
pub struct Model<'a> {
    pub recording: bool,
    pub rows: &'a [Row],
    pub sel: usize,
    /// 单渠道模式下这一句的实时文字（多渠道走 rows）
    pub text: String,
}

/// 会话收到的全部输入。控制器只有一个入口：`while let Some(m) = rx.recv()`。
pub enum Msg {
    /// 手机端事件，Press 时附带本句的音频流
    Remote(RemoteEvent, Option<Audio>),
    /// 引擎的中间结果，带发出时的代际
    Partial { gen: u64, ch: Channel, seg: usize, text: String },
    /// 引擎的定稿结果，带发出时的代际
    Done { gen: u64, ch: Channel, seg: usize, res: Result<String, String> },
    /// 等定稿的超时，带票号
    Timeout { token: u64 },
    /// `emit_rows` 的节流兑现
    Flush,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Idle,
    Single(Channel),
    Pick,
}

/// 单渠道会话的分段：识别中再触发继续说，各段结果按顺序拼成一句
#[derive(Default)]
pub struct Segments {
    parts: Vec<(String, bool)>,
    pub error: Option<String>,
}

impl Segments {
    pub fn start(&mut self) -> usize {
        self.parts.push((String::new(), false));
        self.parts.len() - 1
    }

    /// None：该段已定稿，丢弃；Some(changed)：文字是否有变化
    pub fn partial(&mut self, i: usize, text: String) -> Option<bool> {
        match self.parts.get_mut(i) {
            Some((t, false)) => {
                let changed = *t != text;
                *t = text;
                Some(changed)
            }
            _ => None,
        }
    }

    /// keep_partial：流式上屏时，定稿为空也保留已打出的片段。失败一律留下已显示的中间结果
    pub fn done(&mut self, i: usize, res: Result<String, String>, keep_partial: bool) {
        let Some((t, done)) = self.parts.get_mut(i) else { return };
        if *done {
            return;
        }
        *done = true;
        match res {
            Ok(s) if !s.is_empty() => *t = s,
            Ok(_) => {
                if !keep_partial {
                    t.clear()
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn finished(&self) -> bool {
        self.parts.iter().all(|p| p.1)
    }

    pub fn text(&self) -> String {
        self.parts.iter().map(|p| p.0.as_str()).collect()
    }
}


#[cfg(test)]
mod tests {
    use super::Segments;

    #[test]
    fn resumed_segments_join_in_order_and_wait_for_all() {
        let mut s = Segments::default();
        let a = s.start();
        assert_eq!(s.partial(a, "今天".into()), Some(true));
        let b = s.start();
        assert_eq!(s.partial(b, "天气".into()), Some(true));
        assert_eq!(s.partial(b, "天气".into()), Some(false));
        s.done(b, Ok("天气不错。".into()), false);
        assert!(!s.finished());
        s.done(a, Ok("今天，".into()), false);
        assert!(s.finished());
        assert_eq!(s.text(), "今天，天气不错。");
        assert_eq!(s.partial(a, "迟到".into()), None);
    }

    #[test]
    fn failed_segment_keeps_partial_text_and_reports_error() {
        let mut s = Segments::default();
        let a = s.start();
        let b = s.start();
        s.partial(b, "半句".into());
        s.done(a, Ok("第一句".into()), false);
        s.done(b, Err("网络".into()), false);
        assert_eq!(s.text(), "第一句半句");
        assert_eq!(s.error.as_deref(), Some("网络"));

        let mut timed_out = Segments::default();
        let d = timed_out.start();
        timed_out.partial(d, "已经打出来的字".into());
        timed_out.done(d, Err("识别超时".into()), false);
        assert_eq!(timed_out.text(), "已经打出来的字");

        let mut streamed = Segments::default();
        let c = streamed.start();
        streamed.partial(c, "已上屏".into());
        streamed.done(c, Ok(String::new()), true);
        assert_eq!(streamed.text(), "已上屏");
    }
}
