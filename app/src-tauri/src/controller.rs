//! 会话编排：手机按下 → 音频流 → 识别 → 上屏。
//!
//! 状态机本身没变（Mode / gen / Segments / 超时 / 33ms 节流都是原来那套），
//! 变的只是「谁开始一句话」：以前是全局快捷键加电脑麦克风，现在是手机端的 Press/Release，
//! 音频由 `voicekey-remote` 交过来。用户松手就是结束，所以静音自动结束那套判定没有了。
use crate::settings::{Channel, Settings};
use crate::ui;
use crate::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use voicekey_core::{
    Audio, BaiduEngine, ChatterflyEngine, DoubaoEngine, Engine, IflyEngine, QwenEngine, SogouEngine, WeTypeEngine,
};
use voicekey_platform::{self as pf, Typer};
use voicekey_remote::{Event as RemoteEvent, Handle as RemoteHandle};

pub enum Msg {
    /// 手机端事件，Press 时附带本句的音频流
    Remote(RemoteEvent, Option<Audio>),
    Partial { gen: u64, ch: Channel, seg: usize, text: String },
    Done { gen: u64, ch: Channel, seg: usize, res: Result<String, String> },
    Timeout { token: u64 },
    Flush,
}

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
    channel: Channel,
    text: String,
    state: RowState,
    ms: Option<u64>,
}

#[derive(Clone, Serialize)]
struct Model<'a> {
    recording: bool,
    rows: &'a [Row],
    sel: usize,
    /// 单渠道模式下这一句的实时文字（多渠道走 rows）
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Idle,
    Single(Channel),
    Pick,
}

/// 单渠道会话的分段：识别中再触发继续说，各段结果按顺序拼成一句
#[derive(Default)]
struct Segments {
    parts: Vec<(String, bool)>,
    error: Option<String>,
}

impl Segments {
    fn start(&mut self) -> usize {
        self.parts.push((String::new(), false));
        self.parts.len() - 1
    }

    /// None：该段已定稿，丢弃；Some(changed)：文字是否有变化
    fn partial(&mut self, i: usize, text: String) -> Option<bool> {
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
    fn done(&mut self, i: usize, res: Result<String, String>, keep_partial: bool) {
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

    fn finished(&self) -> bool {
        self.parts.iter().all(|p| p.1)
    }

    fn text(&self) -> String {
        self.parts.iter().map(|p| p.0.as_str()).collect()
    }
}

pub struct Ctl {
    app: AppHandle,
    settings: Arc<RwLock<Settings>>,
    tx: UnboundedSender<Msg>,
    remote: Arc<RwLock<Option<RemoteHandle>>>,
    engines: HashMap<Channel, Arc<dyn Engine>>,
    tasks: Vec<JoinHandle<()>>,
    gen: u64,
    timeout_token: u64,
    mode: Mode,
    /// 手机正按着
    recording: bool,
    typer: Option<Typer>,
    segs: Segments,
    rows: Vec<Row>,
    row_segs: Vec<Segments>,
    sel: usize,
    user_picked: bool,
    last_emit: Instant,
    flush_pending: bool,
    released_at: Option<Instant>,
}

pub fn spawn(app: AppHandle, qwen: Arc<QwenEngine>, rx: UnboundedReceiver<Msg>) {
    let st = app.state::<AppState>();
    let mut engines: HashMap<Channel, Arc<dyn Engine>> = HashMap::new();
    engines.insert(Channel::Doubao, Arc::new(DoubaoEngine));
    engines.insert(Channel::Wetype, Arc::new(WeTypeEngine::default()));
    engines.insert(Channel::Qwen, qwen);
    engines.insert(Channel::Baidu, Arc::new(BaiduEngine));
    engines.insert(Channel::Sogou, Arc::new(SogouEngine));
    engines.insert(Channel::Iflytek, Arc::new(IflyEngine));
    engines.insert(Channel::Chatterfly, Arc::new(ChatterflyEngine));
    engines.insert(Channel::WetypeOffline, crate::wtoffline::engine());
    let ctl = Ctl {
        settings: st.settings.clone(),
        remote: st.remote.clone(),
        tx: st.tx.clone(),
        app: app.clone(),
        engines,
        tasks: Vec::new(),
        gen: 0,
        timeout_token: 0,
        mode: Mode::Idle,
        recording: false,
        typer: None,
        segs: Segments::default(),
        rows: Vec::new(),
        row_segs: Vec::new(),
        sel: 0,
        user_picked: false,
        last_emit: Instant::now(),
        flush_pending: false,
        released_at: None,
    };
    tauri::async_runtime::spawn(ctl.run(rx));
}

impl Ctl {
    async fn run(mut self, mut rx: UnboundedReceiver<Msg>) {
        while let Some(m) = rx.recv().await {
            self.handle(m);
        }
    }

    /// 记下这一句：连接页要展示「最近一句」，也让失败后能复查
    fn record(&self, text: &str, ch: Channel, ms: Option<u64>) {
        let value = serde_json::json!({
            "text": text,
            "channel": ch,
            "at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            "ms": ms,
        });
        let st = self.app.state::<AppState>();
        *st.last.write().unwrap() = Some(value.clone());
        let _ = self.app.emit("last", value);
    }

    fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    fn publish(&self, value: serde_json::Value) {
        if let Some(h) = self.remote.read().unwrap().as_ref() {
            h.publish(value);
        }
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Remote(e, audio) => self.remote_event(e, audio),
            Msg::Partial { gen, ch, seg, text } if gen == self.gen => self.partial(ch, seg, text),
            Msg::Done { gen, ch, seg, res } if gen == self.gen => self.done(ch, seg, res),
            Msg::Timeout { token } if token == self.timeout_token && self.mode != Mode::Idle => {
                self.abort(Some("识别超时".into()))
            }
            Msg::Flush => {
                self.flush_pending = false;
                self.emit_rows();
            }
            _ => {}
        }
    }

    fn remote_event(&mut self, e: RemoteEvent, audio: Option<Audio>) {
        match e {
            RemoteEvent::Press => self.press(audio),
            RemoteEvent::Release => self.release(),
            RemoteEvent::Abort(reason) => {
                if self.mode != Mode::Idle {
                    self.abort(Some(reason))
                }
            }
            RemoteEvent::Commit(channel) => self.commit(&channel),
            // 说到一半不给删：流式打字机记着已打的字数，外部退格会让它失准
            RemoteEvent::Erase(n) if !self.recording => pf::backspace(n as usize),
            RemoteEvent::Erase(_) => {}
        }
    }

    fn press(&mut self, audio: Option<Audio>) {
        let Some(audio) = audio else {
            ui::hud_error(&self.app, "没有收到音频");
            return;
        };
        match self.mode {
            Mode::Idle => {
                self.prewarm();
                let s = self.settings();
                if s.channel == Channel::All {
                    self.begin_multi(s.multi, audio);
                } else {
                    self.begin_single(s.channel, audio);
                }
            }
            // 上一句还在定稿或候选里：接着说，结果接在后面
            Mode::Single(_) | Mode::Pick => self.resume(audio),
        }
    }

    fn prewarm(&self) {
        let ch = self.settings().channel;
        let list = if ch == Channel::All { self.settings().multi } else { vec![ch] };
        for c in list {
            if let Some(e) = self.engines.get(&c).cloned() {
                tauri::async_runtime::spawn(async move { e.prewarm().await });
            }
        }
    }

    fn begin_single(&mut self, ch: Channel, audio: Audio) {
        self.gen += 1;
        self.mode = Mode::Single(ch);
        self.recording = true;
        self.typer = self.settings().streaming.then(Typer::default);
        self.segs = Segments::default();
        let seg = self.segs.start();
        ui::hud(&self.app, "listen", "", ch);
        self.spawn_engine(ch, seg, audio);
    }

    fn begin_multi(&mut self, mut channels: Vec<Channel>, audio: Audio) {
        let n = self.settings().pick_counts;
        channels.sort_by_key(|c| std::cmp::Reverse(n.get(&c).copied().unwrap_or(0)));
        self.gen += 1;
        self.mode = Mode::Pick;
        self.recording = true;
        self.sel = 0;
        self.user_picked = false;
        self.released_at = None;
        self.rows = channels
            .into_iter()
            .map(|ch| {
                let skip = !crate::model_ready(ch);
                Row {
                    channel: ch,
                    text: if skip { "离线模型未下载，已跳过".into() } else { String::new() },
                    state: if skip { RowState::Skip } else { RowState::Listen },
                    ms: None,
                }
            })
            .collect();
        self.row_segs = self.rows.iter().map(|_| Segments::default()).collect();
        let targets = self
            .rows
            .iter()
            .zip(&mut self.row_segs)
            .filter(|(r, _)| r.state == RowState::Listen)
            .map(|(r, segs)| (r.channel, segs.start()))
            .collect();
        self.fan_out(audio, targets);
        // 默认选中上次用的渠道
        let last = self.settings().last_pick;
        if let Some(i) = self.rows.iter().position(|r| Some(r.channel) == last && r.state == RowState::Listen) {
            self.sel = i;
            self.user_picked = true;
        }
        self.emit_rows();
    }

    /// 识别中或候选未消失时再按：新开一段，结果接在前面几段后面
    fn resume(&mut self, audio: Audio) {
        if self.recording {
            return;
        }
        self.recording = true;
        self.timeout_token += 1;
        if let Mode::Single(ch) = self.mode {
            let seg = self.segs.start();
            self.hud_single(ch);
            return self.spawn_engine(ch, seg, audio);
        }
        self.released_at = None;
        let mut targets = Vec::new();
        for (r, segs) in self.rows.iter_mut().zip(&mut self.row_segs) {
            if r.state == RowState::Skip {
                continue;
            }
            targets.push((r.channel, segs.start()));
            r.state = RowState::Listen;
            r.text = segs.text();
        }
        self.fan_out(audio, targets);
        self.emit_rows();
    }

    fn hud_single(&self, ch: Channel) {
        let text = if self.settings.read().unwrap().live_text { self.segs.text() } else { String::new() };
        ui::hud(&self.app, if self.recording { "listen" } else { "wait" }, &text, ch);
    }

    fn fan_out(&mut self, mut audio: Audio, targets: Vec<(Channel, usize)>) {
        let mut senders = Vec::new();
        for (ch, seg) in targets {
            let (t, r) = mpsc::unbounded_channel();
            senders.push(t);
            self.spawn_engine(ch, seg, r);
        }
        self.tasks.push(tauri::async_runtime::spawn(async move {
            while let Some(f) = audio.recv().await {
                for s in &senders {
                    let _ = s.send(f.clone());
                }
            }
        }));
    }

    fn spawn_engine(&mut self, ch: Channel, seg: usize, audio: Audio) {
        let Some(engine) = self.engines.get(&ch).cloned() else { return };
        let (gen, tx) = (self.gen, self.tx.clone());
        self.tasks.push(tauri::async_runtime::spawn(async move {
            let ptx = tx.clone();
            let partial = Box::new(move |t: &str| {
                let _ = ptx.send(Msg::Partial { gen, ch, seg, text: t.to_string() });
            });
            let res = engine.run(audio, partial).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Done { gen, ch, seg, res });
        }));
    }

    fn partial(&mut self, ch: Channel, seg: usize, text: String) {
        match self.mode {
            Mode::Single(_) => {
                let Some(changed) = self.segs.partial(seg, text) else { return };
                if !changed {
                    return;
                }
                if let Some(t) = self.typer.as_mut() {
                    t.update(&self.segs.text());
                }
                let text = self.segs.text();
                // 人盯着手机看，实时文字必须发到手机上去
                self.publish(serde_json::json!({ "type": "partial", "text": text }));
                self.hud_single(ch);
            }
            Mode::Pick => {
                if let Some(i) = self.rows.iter().position(|r| r.channel == ch) {
                    if self.row_segs[i].partial(seg, text).is_some() {
                        self.rows[i].text = self.row_segs[i].text();
                    }
                }
                self.emit_rows();
            }
            Mode::Idle => {}
        }
    }

    fn done(&mut self, ch: Channel, seg: usize, res: Result<String, String>) {
        match self.mode {
            Mode::Single(_) => {
                self.segs.done(seg, res, self.typer.is_some());
                let text = self.segs.text();
                if !self.segs.finished() {
                    if let Some(t) = self.typer.as_mut().filter(|_| !text.is_empty()) {
                        t.update(&text);
                    }
                    return;
                }
                self.recording = false;
                self.timeout_token += 1;
                self.tasks.clear();
                self.mode = Mode::Idle;
                let typer = self.typer.take();
                let error = self.segs.error.take();
                let streamed = typer.is_some();
                if let Some(mut t) = typer {
                    // 定稿可能与流式结果不同（数字/标点整理），按差异修正；之后到达的迟到片段丢弃
                    if !text.is_empty() {
                        t.update(&text);
                    }
                    t.finish();
                }
                if let Some(e) = error {
                    if !streamed && !text.is_empty() {
                        pf::paste(&text);
                    }
                    let msg = if text.is_empty() {
                        format!("识别失败：{e}")
                    } else {
                        format!("识别失败：{e}，已保留已识别的文字")
                    };
                    self.publish(serde_json::json!({ "type": "error", "stage": "asr", "message": msg }));
                    ui::hud_error(&self.app, &msg);
                } else if streamed {
                    ui::hud_hide(&self.app, 0);
                    self.publish(serde_json::json!({ "type": "final", "text": text }));
                } else if text.is_empty() {
                    ui::hud(&self.app, "info", "没有识别到内容", ch);
                    ui::hud_hide(&self.app, 1000);
                    self.publish(serde_json::json!({ "type": "final", "text": "" }));
                } else {
                    ui::hud_hide(&self.app, 0);
                    pf::paste(&text);
                    self.record(&text, ch, None);
                    self.publish(serde_json::json!({ "type": "final", "text": text }));
                }
            }
            Mode::Pick => {
                let ms = self.released_at.map(|t| t.elapsed().as_millis() as u64);
                let Some(i) = self.rows.iter().position(|r| r.channel == ch) else { return };
                let segs = &mut self.row_segs[i];
                segs.done(seg, res, false);
                let text = segs.text();
                let r = &mut self.rows[i];
                if !segs.finished() {
                    r.text = text;
                } else if !text.is_empty() {
                    r.text = text;
                    r.state = RowState::Final;
                    r.ms = Some(ms.unwrap_or(0));
                } else {
                    r.text = segs.error.clone().unwrap_or_else(|| "没有识别到内容".into());
                    r.state = RowState::Error;
                }
                let cur = self.rows[self.sel].state;
                let fallback = !self.user_picked || matches!(cur, RowState::Error | RowState::Skip);
                if self.rows[i].state == RowState::Final && fallback && cur != RowState::Final {
                    self.sel = i;
                }
                if !self.rows.iter().any(|r| matches!(r.state, RowState::Listen | RowState::Wait)) {
                    self.timeout_token += 1;
                }
                self.emit_rows();
            }
            Mode::Idle => {}
        }
    }

    /// 手机松手。音频流这时已经结束（服务端在发 Release 前就停了泵），
    /// 所以这里只更新界面状态并开始等定稿。
    fn release(&mut self) {
        if !self.recording {
            return;
        }
        self.recording = false;
        match self.mode {
            Mode::Single(ch) => ui::hud(&self.app, "wait", "", ch),
            Mode::Pick => {
                self.released_at = Some(Instant::now());
                for r in &mut self.rows {
                    if r.state == RowState::Listen {
                        r.state = RowState::Wait;
                    }
                }
                self.emit_rows();
            }
            Mode::Idle => return,
        }
        self.timeout_token += 1;
        let (token, tx) = (self.timeout_token, self.tx.clone());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let _ = tx.send(Msg::Timeout { token });
        });
    }

    /// 打断：定稿超时、手机断线
    fn abort(&mut self, message: Option<String>) {
        if self.mode == Mode::Idle {
            return;
        }
        let streamed = self.typer.is_some();
        let partial = match self.mode {
            Mode::Single(_) => self.segs.text(),
            Mode::Pick => self.rows.get(self.sel).map(|r| r.text.clone()).unwrap_or_default(),
            Mode::Idle => String::new(),
        };
        self.cancel();
        // 超时打断时，非流式还没写进输入框的中间结果补上；用户主动取消不补写
        if message.is_some() && !streamed && !partial.is_empty() {
            pf::paste(&partial);
        }
        match message {
            Some(m) => {
                let text = if partial.is_empty() { m } else { format!("{m}，已保留已识别的文字") };
                self.publish(serde_json::json!({ "type": "error", "stage": "session", "message": text }));
                ui::hud_error(&self.app, &text);
            }
            None => ui::hud_hide(&self.app, 0),
        }
    }

    fn cancel(&mut self) {
        self.gen += 1;
        self.timeout_token += 1;
        for t in self.tasks.drain(..) {
            t.abort();
        }
        self.recording = false;
        if let Some(mut t) = self.typer.take() {
            t.finish();
        }
        self.mode = Mode::Idle;
    }

    /// 手机点了某条候选
    fn commit(&mut self, channel: &str) {
        if self.mode != Mode::Pick {
            return;
        }
        let Ok(ch) = serde_json::from_value::<Channel>(serde_json::json!(channel)) else { return };
        let Some(i) = self.rows.iter().position(|r| r.channel == ch) else { return };
        self.choose(i);
    }

    /// 选中一行上屏；还在监听的行等它定稿再说，失败的行提示重选
    fn choose(&mut self, i: usize) {
        let Some(r) = self.rows.get(i) else { return };
        if matches!(r.state, RowState::Listen | RowState::Wait) {
            self.sel = i;
            self.user_picked = true;
            self.emit_rows();
            return;
        }
        if r.state != RowState::Final {
            self.sel = i;
            self.user_picked = true;
            self.emit_rows();
            self.publish(serde_json::json!({ "type": "shake", "row": i }));
            return;
        }
        let text = r.text.clone();
        let ch = r.channel;
        crate::update_settings(&self.app, |s| {
            s.last_pick = Some(ch);
            *s.pick_counts.entry(ch).or_default() += 1;
        });
        self.cancel();
        pf::paste(&text);
        self.record(&text, ch, None);
        self.publish(serde_json::json!({ "type": "final", "text": text }));
    }

    /// 最多每 33ms 推送一次，避免文字跳动
    fn emit_rows(&mut self) {
        let wait = Duration::from_millis(33).saturating_sub(self.last_emit.elapsed());
        if !wait.is_zero() {
            if !self.flush_pending {
                self.flush_pending = true;
                let tx = self.tx.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(wait).await;
                    let _ = tx.send(Msg::Flush);
                });
            }
            return;
        }
        self.last_emit = Instant::now();
        let text = match self.mode {
            Mode::Single(_) => self.segs.text(),
            _ => String::new(),
        };
        let model = Model { recording: self.recording, rows: &self.rows, sel: self.sel, text };
        if let Ok(v) = serde_json::to_value(&model) {
            self.publish(serde_json::json!({ "type": "rows", "model": v }));
        }
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
