//! 会话编排：手机按下 → 音频流 → 识别 → 上屏。
//!
//! 状态机本身没变（Mode / gen / Segments / 超时 / 33ms 节流都是原来那套），
//! 变的只是「谁开始一句话」：以前是全局快捷键加电脑麦克风，现在是手机端的 Press/Release，
//! 音频由 `voicekey-remote` 交过来。用户松手就是结束，所以静音自动结束那套判定没有了。
use crate::session::engines::Engines;
use crate::session::machine::{Choice, Picker, Tickets};
use crate::session::types::{Mode, Model, Msg, Segments};
use crate::settings::{Channel, Settings};
use crate::ui;
use crate::AppState;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use voicekey_core::{Audio, QwenEngine};
use voicekey_platform::{self as pf, Typer};
use voicekey_remote::protocol;
use voicekey_remote::{Event as RemoteEvent, Handle as RemoteHandle};

pub struct Ctl {
    app: AppHandle,
    settings: Arc<RwLock<Settings>>,
    tx: UnboundedSender<Msg>,
    remote: Arc<RwLock<Option<RemoteHandle>>>,
    /// 引擎注册表 + 本句的任务句柄（取消时一起停）
    engines: Engines,
    /// 代际与超时票根：迟到的结果靠它作废
    tickets: Tickets,
    mode: Mode,
    /// 手机正按着
    recording: bool,
    typer: Option<Typer>,
    /// 单渠道会话的分段文本
    segs: Segments,
    /// 多渠道候选（含每行的分段、排序、选中项）
    picker: Picker,
    last_emit: Instant,
    flush_pending: bool,
    released_at: Option<Instant>,
}

pub fn spawn(app: AppHandle, qwen: Arc<QwenEngine>, rx: UnboundedReceiver<Msg>) {
    let st = app.state::<AppState>();
    let ctl = Ctl {
        settings: st.settings.clone(),
        remote: st.remote.clone(),
        tx: st.tx.clone(),
        app: app.clone(),
        engines: Engines::new(qwen),
        tickets: Tickets::default(),
        mode: Mode::Idle,
        recording: false,
        typer: None,
        segs: Segments::default(),
        picker: Picker::new(Vec::new(), |_| false, None, &std::collections::HashMap::new()),
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

    /// 上屏，并把「被高权限窗口拦住」如实说出来。
    /// 被拦下的文字留在剪贴板里，用户按一次 Ctrl+V 就能拿到，不算白说。
    fn output(&mut self, text: &str, ch: Channel) {
        if pf::paste(text) != pf::Pasted::Blocked {
            ui::hud_hide(&self.app, 0);
            return;
        }
        let msg = "目标窗口以管理员权限运行，文字已复制到剪贴板，请按 Ctrl+V";
        self.publish(protocol::notice(msg));
        ui::hud(&self.app, "info", "已复制到剪贴板 · 请手动粘贴", ch);
        ui::hud_hide(&self.app, 2400);
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
            Msg::Partial { gen, ch, seg, text } if self.tickets.accepts(gen) => self.partial(ch, seg, text),
            Msg::Done { gen, ch, seg, res } if self.tickets.accepts(gen) => self.done(ch, seg, res),
            Msg::Timeout { token } if self.tickets.timeout_is_current(token) && self.mode != Mode::Idle => {
                self.abort("识别超时".into())
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
            RemoteEvent::Cancel => self.cancel_current(),
            RemoteEvent::Abort(reason) => {
                if self.mode != Mode::Idle {
                    self.abort(reason)
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
        self.engines.prewarm(&list);
    }

    fn begin_single(&mut self, ch: Channel, audio: Audio) {
        self.tickets.next_generation();
        self.mode = Mode::Single(ch);
        self.recording = true;
        // 高权限窗口收不到任何模拟输入，流式上屏会「看着在录、实际一个字没进」，
        // 这种时候直接走剪贴板路径，并且明说一句
        let blocked = pf::input_blocked();
        self.typer = (self.settings().streaming && !blocked).then(Typer::default);
        if blocked {
            self.publish(protocol::notice("当前窗口以管理员权限运行，将改用剪贴板粘贴"));
        }
        self.segs = Segments::default();
        let seg = self.segs.start();
        ui::hud(&self.app, "listen", "", ch);
        let (gen, tx) = (self.tickets.generation(), self.tx.clone());
        self.engines.spawn(ch, seg, gen, audio, tx);
    }

    fn begin_multi(&mut self, channels: Vec<Channel>, audio: Audio) {
        let s = self.settings();
        self.tickets.next_generation();
        self.mode = Mode::Pick;
        self.recording = true;
        self.released_at = None;
        // 行的排序、跳过、默认焦点都在 Picker 里，这里只负责把音频扇出去
        self.picker = Picker::new(channels, crate::model_ready, s.last_pick, &s.pick_counts);
        let targets = self.picker.start_round();
        let (gen, tx) = (self.tickets.generation(), self.tx.clone());
        self.engines.fan_out(audio, targets, gen, tx);
        self.emit_rows();
    }

    /// 识别中或候选未消失时再按：新开一段，结果接在前面几段后面
    fn resume(&mut self, audio: Audio) {
        if self.recording {
            return;
        }
        self.recording = true;
        self.tickets.next_timeout();
        if let Mode::Single(ch) = self.mode {
            let seg = self.segs.start();
            self.hud_single(ch);
            let (gen, tx) = (self.tickets.generation(), self.tx.clone());
            return self.engines.spawn(ch, seg, gen, audio, tx);
        }
        self.released_at = None;
        let targets = self.picker.start_round();
        let (gen, tx) = (self.tickets.generation(), self.tx.clone());
        self.engines.fan_out(audio, targets, gen, tx);
        self.emit_rows();
    }

    fn hud_single(&self, ch: Channel) {
        let text = if self.settings.read().unwrap().live_text { self.segs.text() } else { String::new() };
        ui::hud(&self.app, if self.recording { "listen" } else { "wait" }, &text, ch);
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
                self.publish(protocol::partial(text));
                self.hud_single(ch);
            }
            Mode::Pick => {
                self.picker.on_partial(ch, seg, text);
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
                self.tickets.next_timeout();
                self.engines.abort_all();
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
                        self.output(&text, ch);
                    }
                    let msg = if text.is_empty() {
                        format!("识别失败：{e}")
                    } else {
                        format!("识别失败：{e}，已保留已识别的文字")
                    };
                    self.publish(protocol::error("asr", msg.as_str()));
                    ui::hud_error(&self.app, &msg);
                } else if streamed {
                    ui::hud_hide(&self.app, 0);
                    self.publish(protocol::fin(text));
                } else if text.is_empty() {
                    // 桌面上只有一行"没有识别到内容"，手机上什么原因都看不到，
                    // 所以给一句能立刻照做的话
                    self.publish(protocol::notice("没听清：靠近手机再说一次，或在电脑上换个渠道试试"));
                    ui::hud(&self.app, "info", "没有识别到内容", ch);
                    ui::hud_hide(&self.app, 1000);
                    self.publish(protocol::fin(""));
                } else {
                    self.output(&text, ch);
                    self.record(&text, ch, None);
                    self.publish(protocol::fin(text));
                }
            }
            Mode::Pick => {
                let ms = self.released_at.map(|t| t.elapsed().as_millis() as u64);
                self.picker.on_done(ch, seg, res, ms);
                // 全都不在识别中了：把「等定稿」的超时撤掉，免得误报
                if self.picker.all_settled() {
                    self.tickets.next_timeout();
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
                self.picker.release();
                self.emit_rows();
            }
            Mode::Idle => return,
        }
        let token = self.tickets.next_timeout();
        let tx = self.tx.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let _ = tx.send(Msg::Timeout { token });
        });
    }

    /// 打断：定稿超时、手机断线。已经识别出的文字尽量留下
    fn abort(&mut self, message: String) {
        if self.mode == Mode::Idle {
            return;
        }
        let streamed = self.typer.is_some();
        let (partial, ch) = match self.mode {
            Mode::Single(ch) => (self.segs.text(), ch),
            Mode::Pick => {
                self.picker.selected().unwrap_or_else(|| (String::new(), self.settings().channel))
            }
            Mode::Idle => (String::new(), self.settings().channel),
        };
        self.cancel(false);
        // 非流式时中间结果还没写进输入框，补上；流式已经打出去了，不重复写
        if !streamed && !partial.is_empty() {
            self.output(&partial, ch);
        }
        let text = if partial.is_empty() { message } else { format!("{message}，已保留已识别的文字") };
        self.publish(protocol::error("session", text.as_str()));
        ui::hud_error(&self.app, &text);
    }

    fn cancel(&mut self, revert: bool) {
        self.tickets.next_generation();
        self.tickets.next_timeout();
        self.engines.abort_all();
        self.recording = false;
        if let Some(mut t) = self.typer.take() {
            // 用户取消时把已经流式打出去的字退回来，输入框里不留半句
            if revert {
                t.discard();
            } else {
                t.finish();
            }
        }
        self.mode = Mode::Idle;
    }

    /// 手机上滑取消：这一句不要了，什么都不上屏
    fn cancel_current(&mut self) {
        if self.mode == Mode::Idle && !self.recording {
            return;
        }
        self.cancel(true);
        ui::hud_hide(&self.app, 0);
    }

    /// 手机点了某条候选
    fn commit(&mut self, channel: &str) {
        if self.mode != Mode::Pick {
            return;
        }
        let Ok(ch) = serde_json::from_value::<Channel>(serde_json::json!(channel)) else { return };
        let Some(i) = self.picker.index_of(ch) else { return };
        self.choose(i);
    }

    /// 选中一行上屏；还在监听的行等它定稿再说，失败的行提示重选
    fn choose(&mut self, i: usize) {
        match self.picker.choose(i) {
            Choice::Select => self.emit_rows(),
            Choice::Shake => {
                self.emit_rows();
                self.publish(protocol::shake(i));
            }
            Choice::Commit => {
                let Some((text, ch)) = self.picker.take(i) else { return };
                crate::update_settings(&self.app, |s| {
                    s.last_pick = Some(ch);
                    *s.pick_counts.entry(ch).or_default() += 1;
                });
                self.cancel(false);
                self.output(&text, ch);
                self.record(&text, ch, None);
                self.publish(protocol::fin(text));
            }
        }
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
        let model = Model { recording: self.recording, rows: self.picker.rows(), sel: self.picker.sel(), text };
        if let Ok(v) = serde_json::to_value(&model) {
            self.publish(protocol::rows(v));
        }
    }
}

