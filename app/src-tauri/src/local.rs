//! 电脑本机麦克风：按住快捷键说话，和手机按住说话走同一个会话。
//!
//! 这里只做声源该做的事——拿发言权、开麦克风、把样本泵成本句的音频流、按顺序发出
//! Press / Release / Cancel。之后的识别、上屏、候选与手机那条路径完全一样。
//!
//! 麦克风只在按住时开着：按下打开、这一句收尾即关，启动时不碰设备。
//! 全部状态由一个线程独占，快捷键钩子、采集线程、界面都只往它的通道里发命令。

use crate::session::types::Msg;
use crate::settings::LocalMic;
use crate::state::{AppState, LocalStatus};
use crate::ui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use voicekey_core::audio::{pump, Inbox};
use voicekey_platform::hotkey::{Hotkey, Signal, TAP_MS};
use voicekey_platform::mic::{self, Capture, MicError};
use voicekey_remote::{Event, Floor, LOCAL};

/// 设置页「试听」的时长
const TEST: Duration = Duration::from_secs(5);
/// 电平推给界面的间隔
const LEVEL_EVERY: Duration = Duration::from_millis(50);

enum Cmd {
    Key(Signal),
    Configure(LocalMic),
    Lost(MicError),
    Test,
    Retry,
}

/// 配置生效时要不要试开一次麦克风（随即在 tick 里关掉），让隐私、独占这类问题当场在设置页报出来
#[derive(PartialEq)]
enum Probe {
    /// 启动时：不碰设备
    No,
    /// 用户刚开启或换了设备
    IfChanged,
    /// 用户点了「重试」
    Always,
}

static CMD: OnceLock<Sender<Cmd>> = OnceLock::new();

fn send(c: Cmd) {
    if let Some(tx) = CMD.get() {
        let _ = tx.send(c);
    }
}

/// 应用启动时调用一次：按设置装上快捷键，不打开麦克风
pub fn spawn(app: AppHandle, floor: Arc<Floor>, initial: LocalMic) {
    let (tx, rx) = mpsc::channel();
    if CMD.set(tx.clone()).is_err() {
        return;
    }
    let worker = Worker {
        app,
        floor,
        tx,
        config: LocalMic::default(),
        hotkey: None,
        capture: None,
        inbox: Arc::new(Inbox::default()),
        meter: Arc::new(Meter::default()),
        hold: None,
        pending: None,
        test_until: None,
    };
    let spawned = std::thread::Builder::new().name("voicekey-local".into()).spawn(move || {
        let mut w = worker;
        w.configure(initial, Probe::No);
        loop {
            let wait = match w.pending {
                Some(at) => Duration::from_millis(TAP_MS).saturating_sub(at.elapsed()),
                None if w.test_until.is_some() => Duration::from_millis(200),
                None => Duration::from_secs(1),
            };
            match rx.recv_timeout(wait) {
                Ok(cmd) => w.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            w.tick();
        }
    });
    if let Err(e) = spawned {
        crate::log_error!("电脑麦克风线程启动失败：{e}");
    }
}

/// 设置改了
pub fn configure(c: &LocalMic) {
    send(Cmd::Configure(c.clone()));
}

/// 设置页的「试听」：打开麦克风，把电平推给设置窗几秒
pub fn test() {
    send(Cmd::Test);
}

/// 设置页的「重试」：出错后按当前设置重装快捷键、试开麦克风
pub fn retry() {
    send(Cmd::Retry);
}

/// 采集线程往哪里报电平
#[derive(Default)]
struct Meter {
    /// 正在说话：浮层
    hud: AtomicBool,
    /// 正在试听：设置窗
    main: AtomicBool,
}

struct Hold {
    stop: Arc<AtomicBool>,
    pump: tauri::async_runtime::JoinHandle<()>,
}

struct Worker {
    app: AppHandle,
    floor: Arc<Floor>,
    tx: Sender<Cmd>,
    config: LocalMic,
    hotkey: Option<Hotkey>,
    capture: Option<Capture>,
    inbox: Arc<Inbox>,
    meter: Arc<Meter>,
    hold: Option<Hold>,
    /// 已经按下、还没按满 [`TAP_MS`] 的时刻
    pending: Option<Instant>,
    test_until: Option<Instant>,
}

impl Worker {
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Key(Signal::Press) => self.press(),
            Cmd::Key(Signal::Release) => self.end(Event::Release(LOCAL)),
            Cmd::Key(Signal::Cancel) => self.end(Event::Cancel(LOCAL)),
            Cmd::Key(Signal::Dismiss) => {
                let _ = self.app.state::<AppState>().tx.send(Msg::Dismiss);
            }
            Cmd::Configure(c) => self.configure(c, Probe::IfChanged),
            Cmd::Lost(e) => self.lost(e),
            Cmd::Test => self.test(),
            Cmd::Retry => self.configure(self.config.clone(), Probe::Always),
        }
    }

    fn tick(&mut self) {
        if self.pending.is_some_and(|at| at.elapsed() >= Duration::from_millis(TAP_MS)) {
            self.begin();
        }
        if self.test_until.is_some_and(|t| Instant::now() >= t) {
            self.test_until = None;
            self.meter.main.store(false, Ordering::Relaxed);
            let _ = self.app.emit_to("main", "mic-level", 0.0f32);
        }
        // 没人在用就关：说完、误触、没抢到发言权、试听结束、试开
        let idle = self.hold.is_none() && self.pending.is_none() && self.test_until.is_none();
        if idle && self.capture.is_some() {
            self.close_capture();
        }
    }

    fn set_status(&self, s: LocalStatus) {
        *self.app.state::<AppState>().local_status.write().unwrap() = s.clone();
        let _ = self.app.emit("local", s);
        ui::tray_refresh(&self.app);
    }

    fn fail(&self, e: &MicError) {
        crate::log_warn!("电脑麦克风：{e}");
        self.set_status(LocalStatus { state: "error", error: Some(e.message.clone()), privacy: e.privacy, note: None });
    }

    fn channel(&self) -> crate::settings::Channel {
        self.app.state::<AppState>().settings.read().unwrap().channel
    }

    /// 麦克风没开就打开。成功后状态回到 ready（之前出过错的话一并清掉）
    fn ensure_capture(&mut self) -> Result<(), MicError> {
        if self.capture.is_some() {
            return Ok(());
        }
        let (inbox, meter, app) = (self.inbox.clone(), self.meter.clone(), self.app.clone());
        let (mut peak, mut last) = (0f32, Instant::now());
        let sink = Box::new(move |s: &[i16]| {
            inbox.push(s);
            let (hud, main) = (meter.hud.load(Ordering::Relaxed), meter.main.load(Ordering::Relaxed));
            if !hud && !main {
                return;
            }
            peak = peak.max(mic::level(s));
            if last.elapsed() >= LEVEL_EVERY {
                if hud {
                    let _ = app.emit_to("hud", "level", peak);
                }
                if main {
                    let _ = app.emit_to("main", "mic-level", peak);
                }
                (peak, last) = (0.0, Instant::now());
            }
        });
        let tx = self.tx.clone();
        let lost = Box::new(move |e| {
            let _ = tx.send(Cmd::Lost(e));
        });
        let (capture, fell_back) = Capture::open(self.config.device.clone(), sink, lost)?;
        self.capture = Some(capture);
        let note = fell_back.then(|| "选择的麦克风不在了，暂时改用系统默认".to_string());
        if fell_back {
            crate::log_warn!("电脑麦克风：所选设备不可用，改用系统默认");
        }
        self.set_status(LocalStatus { note, ..LocalStatus::READY });
        Ok(())
    }

    /// 关掉麦克风。残留的样本一并丢掉：它们不属于下一句
    fn close_capture(&mut self) {
        self.capture = None;
        self.inbox.clear();
    }

    /// 按下：先打开麦克风，按满 [`TAP_MS`] 才开始这一句（打开以来的声音由预录接上，字头不丢）。
    /// 误触的短按不会开始一句：不打断正在等定稿的那一句，也不冒出「没有识别到内容」
    fn press(&mut self) {
        if self.hold.is_some() || self.pending.is_some() {
            return;
        }
        self.pending = Some(Instant::now());
        if let Err(e) = self.ensure_capture() {
            self.pending = None;
            self.fail(&e);
            ui::hud_error(&self.app, &e.message);
        }
    }

    /// 按满了：这一句正式开始
    fn begin(&mut self) {
        if self.pending.take().is_none() || self.capture.is_none() {
            return;
        }
        if !self.floor.take(LOCAL) {
            ui::hud(&self.app, "info", "手机正在说话，等它说完再按", self.channel());
            ui::hud_hide(&self.app, 1500);
            return;
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        self.inbox.set_live();
        let pump = tauri::async_runtime::spawn(pump(self.inbox.clone(), tx, stop.clone()));
        self.meter.hud.store(true, Ordering::Relaxed);
        self.hold = Some(Hold { stop, pump });
        self.floor.emit(Event::Press(LOCAL, rx));
    }

    /// 结束这一句：停泵（引擎随即收到流结束）、发出结束事件、最后交还发言权。
    /// 顺序与手机那边一致，见 `Floor::give_back`
    fn end(&mut self, ev: Event) {
        // 还没按满就结束：取消就当没按过；松开（钩子按它的时钟已经算按满）就补上开始再结束
        if self.pending.is_some() {
            if !matches!(ev, Event::Release(_)) {
                self.pending = None;
                return;
            }
            self.begin();
        }
        let Some(h) = self.hold.take() else { return };
        self.meter.hud.store(false, Ordering::Relaxed);
        h.stop.store(true, Ordering::Relaxed);
        let _ = tauri::async_runtime::block_on(h.pump);
        self.floor.emit(ev);
        self.floor.give_back(LOCAL);
    }

    /// 采集中途失败（拔了 USB 麦克风、被别的程序独占）。说到一半的按断线处理，已识别的文字留下
    fn lost(&mut self, e: MicError) {
        self.close_capture();
        self.end(Event::Abort(LOCAL, e.message.clone()));
        self.fail(&e);
    }

    fn test(&mut self) {
        if !self.config.enabled {
            return;
        }
        match self.ensure_capture() {
            Ok(()) => {
                self.test_until = Some(Instant::now() + TEST);
                self.meter.main.store(true, Ordering::Relaxed);
            }
            Err(e) => self.fail(&e),
        }
    }

    fn configure(&mut self, c: LocalMic, probe: Probe) {
        let was = std::mem::replace(&mut self.config, c);
        let device_changed = was.device != self.config.device;
        if !self.config.enabled {
            self.end(Event::Cancel(LOCAL));
            self.hotkey = None;
            self.close_capture();
            self.test_until = None;
            self.meter.main.store(false, Ordering::Relaxed);
            self.floor.leave(LOCAL);
            self.set_status(LocalStatus::OFF);
            return;
        }
        if device_changed {
            self.end(Event::Cancel(LOCAL));
            self.close_capture();
        }
        // 快捷键之前没装上，这次装上了：那个错误随之作废
        let mut installed = false;
        match &self.hotkey {
            Some(h) if was.hotkey != self.config.hotkey => h.set_key(self.config.hotkey.key()),
            Some(_) => {}
            None => {
                let tx = self.tx.clone();
                let emit = Box::new(move |s: Signal| {
                    let _ = tx.send(Cmd::Key(s));
                });
                match Hotkey::start(self.config.hotkey.key(), emit) {
                    Ok(h) => (self.hotkey, installed) = (Some(h), true),
                    Err(e) => {
                        crate::log_error!("快捷键：{e}");
                        let error = Some(format!("快捷键装不上：{e}"));
                        self.set_status(LocalStatus { state: "error", error, ..LocalStatus::OFF });
                        return;
                    }
                }
            }
        }
        let changed = !was.enabled || device_changed;
        if probe == Probe::Always || (probe == Probe::IfChanged && changed) {
            if let Err(e) = self.ensure_capture() {
                self.fail(&e);
            }
        } else if installed || self.app.state::<AppState>().local_status.read().unwrap().state != "error" {
            self.set_status(LocalStatus::READY);
        }
    }
}
