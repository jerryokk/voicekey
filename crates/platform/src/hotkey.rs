//! 按住说话的全局快捷键。
//!
//! 用低级键盘钩子而不是 `RegisterHotKey`：后者只有按下没有松开，也注册不了单独一个修饰键。
//! 选中的那个键被**吞掉**，前台程序看不到它——否则单按 Alt 松开会激活菜单栏，
//! 边说边上屏时补打的字和 Ctrl+V 也会变成 Alt 组合键。
//!
//! 钩子回调只更新状态、发出信号就返回：超时的钩子会被系统悄悄摘掉。
//! 判断逻辑在 [`Machine`] 里，不碰 Win32，可以单独测。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;
use windows::Win32::Foundation::{HANDLE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, UOI_NAME,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_LCONTROL, VK_RCONTROL, VK_RMENU};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetTimer, SetWindowsHookExW,
    UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_UP, MSG, WH_KEYBOARD_LL, WM_QUIT, WM_TIMER,
};

/// 可选的快捷键：只有右边的 Alt 和 Ctrl。拇指 / 小指够得着、按住不累，
/// 而且很少有程序单独用右边那一个，吞掉它不影响左边的同名键
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    RightAlt,
    RightCtrl,
}

impl Key {
    fn vk(self) -> u32 {
        match self {
            Key::RightAlt => VK_RMENU.0 as u32,
            Key::RightCtrl => VK_RCONTROL.0 as u32,
        }
    }
}

/// 钩子发给上层的信号
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// 按下：开始一句
    Press,
    /// 松开：进入定稿
    Release,
    /// 不要这一句：按住时按了 Esc，或者按得太短（误触）
    Cancel,
    /// 浮层上有候选等着挑时按了 Esc：放弃这组候选
    Dismiss,
}

/// 按住不到这么久就松开算误触：静默取消，不出结果
pub const TAP_MS: u64 = 250;
/// 按住超过这么久还没等到松开，多半是松开事件丢了（远程桌面、驱动问题）：按松开处理
pub const MAX_HOLD_MS: u64 = 120_000;
/// AltGr 布局下按右 Alt，系统会先合成一个左 Ctrl，扫描码带这个标记
const ALTGR_FAKE_CTRL_SCAN: u32 = 0x21D;

/// 一次按键要不要吞掉、要不要发信号
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub swallow: bool,
    pub signal: Option<Signal>,
}

const PASS: Outcome = Outcome { swallow: false, signal: None };

fn swallow(signal: Option<Signal>) -> Outcome {
    Outcome { swallow: true, signal }
}

/// 快捷键的状态机。时间以毫秒传进来，测试里可以随意快进
pub struct Machine {
    key: Key,
    /// 按下的时刻；None 表示没按着
    down_at: Option<u64>,
    /// 这一次按住已经取消过了（按了 Esc），松开时不再发信号
    cancelled: bool,
    /// Esc 也被这次按住吞了按下，松开也得吞，否则前台只看到半个 Esc
    eat_escape_up: bool,
}

impl Machine {
    pub fn new(key: Key) -> Self {
        Self { key, down_at: None, cancelled: false, eat_escape_up: false }
    }

    pub fn holding(&self) -> bool {
        self.down_at.is_some()
    }

    /// 换键。按着的时候换，当作松开处理
    pub fn set_key(&mut self, key: Key) -> Option<Signal> {
        let signal = self.force_release();
        self.key = key;
        signal
    }

    /// `picking`：浮层上有候选在等着挑，此时 Esc 归我们
    pub fn on_key(&mut self, vk: u32, scan: u32, up: bool, injected: bool, now: u64, picking: bool) -> Outcome {
        // 我们自己（以及别的软件）注入的键不参与：边说边上屏会注入大量按键
        if injected {
            return PASS;
        }
        if vk == self.key.vk() {
            return self.on_hotkey(up, now);
        }
        if self.key == Key::RightAlt && vk == VK_LCONTROL.0 as u32 && scan == ALTGR_FAKE_CTRL_SCAN {
            return swallow(None);
        }
        if vk == VK_ESCAPE.0 as u32 {
            if up {
                return if std::mem::take(&mut self.eat_escape_up) { swallow(None) } else { PASS };
            }
            // 这次 Esc 已经归我们：自动重复也吞掉，否则前台只看到按下、看不到松开
            if self.eat_escape_up {
                return swallow(None);
            }
            if self.holding() && !self.cancelled {
                self.cancelled = true;
                self.eat_escape_up = true;
                return swallow(Some(Signal::Cancel));
            }
            if !self.holding() && picking {
                self.eat_escape_up = true;
                return swallow(Some(Signal::Dismiss));
            }
        }
        PASS
    }

    fn on_hotkey(&mut self, up: bool, now: u64) -> Outcome {
        match (up, self.down_at) {
            // 按住时的自动重复：吞掉，不再发信号
            (false, Some(_)) => swallow(None),
            (false, None) => {
                self.down_at = Some(now);
                self.cancelled = false;
                swallow(Some(Signal::Press))
            }
            (true, Some(at)) => {
                self.down_at = None;
                let signal = if self.cancelled {
                    None
                } else if now.saturating_sub(at) < TAP_MS {
                    Some(Signal::Cancel)
                } else {
                    Some(Signal::Release)
                };
                swallow(signal)
            }
            // 没见过按下的松开（钩子装上之前就按着）：放行，免得前台的键卡住
            (true, None) => PASS,
        }
    }

    /// 定时检查：`elsewhere` 表示输入桌面已经不是用户桌面（锁屏、UAC 弹窗），
    /// 那里的按键钩子收不到，松开事件再也等不来了
    pub fn on_tick(&mut self, now: u64, elsewhere: bool) -> Option<Signal> {
        let at = self.down_at?;
        if elsewhere || now.saturating_sub(at) >= MAX_HOLD_MS {
            return self.force_release();
        }
        None
    }

    fn force_release(&mut self) -> Option<Signal> {
        self.down_at.take()?;
        (!std::mem::take(&mut self.cancelled)).then_some(Signal::Release)
    }
}

type Emit = Box<dyn Fn(Signal) + Send + Sync>;

struct Shared {
    machine: Machine,
    emit: Emit,
    epoch: Instant,
}

/// 钩子回调是裸函数，状态只能放全局。同一时间只会有一个快捷键
static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
/// 浮层上有候选在等着挑：Esc 归我们
static PICKING: AtomicBool = AtomicBool::new(false);

/// 一个装好的全局快捷键。丢弃即卸载
pub struct Hotkey {
    thread: u32,
    join: Option<JoinHandle<()>>,
}

impl Hotkey {
    /// 装上钩子。`emit` 在钩子线程里调用，必须立刻返回（比如往通道里塞一条）
    pub fn start(key: Key, emit: Emit) -> Result<Self, String> {
        {
            let mut shared = SHARED.lock().unwrap();
            if shared.is_some() {
                return Err("快捷键已经装上了".into());
            }
            *shared = Some(Shared { machine: Machine::new(key), emit, epoch: Instant::now() });
        }
        let (ready_tx, ready_rx) = mpsc::channel();
        let join = std::thread::Builder::new().name("voicekey-hotkey".into()).spawn(move || unsafe {
            let module = GetModuleHandleW(None).ok().map(|m| m.into());
            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module, 0) {
                Ok(h) => h,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("无法安装键盘钩子：{e}")));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(GetCurrentThreadId()));
            // 按住期间每 200ms 看一眼输入桌面有没有被切走
            SetTimer(None, 0, 200, None);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_TIMER {
                    tick();
                }
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(hook);
        });
        let fail = |e: String| {
            *SHARED.lock().unwrap() = None;
            e
        };
        let join = join.map_err(|e| fail(format!("无法启动快捷键线程：{e}")))?;
        match ready_rx.recv() {
            Ok(Ok(thread)) => Ok(Self { thread, join: Some(join) }),
            Ok(Err(e)) => Err(fail(e)),
            Err(_) => Err(fail("快捷键线程意外退出".into())),
        }
    }

    pub fn set_key(&self, key: Key) {
        with(|s| {
            if let Some(signal) = s.machine.set_key(key) {
                (s.emit)(signal);
            }
        });
    }

    /// 浮层候选出现 / 收起时调用
    pub fn set_picking(on: bool) {
        PICKING.store(on, Ordering::Relaxed);
    }

    pub fn picking() -> bool {
        PICKING.load(Ordering::Relaxed)
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        // 卸载时还按着：替它松开，免得会话一直在录
        if let Some(mut s) = SHARED.lock().unwrap().take() {
            if let Some(signal) = s.machine.force_release() {
                (s.emit)(signal);
            }
        }
    }
}

fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> Option<R> {
    SHARED.lock().unwrap().as_mut().map(f)
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let up = k.flags.contains(LLKHF_UP);
        let injected = k.flags.contains(LLKHF_INJECTED);
        let picking = PICKING.load(Ordering::Relaxed);
        let swallowed = with(|s| {
            let now = s.epoch.elapsed().as_millis() as u64;
            let o = s.machine.on_key(k.vkCode, k.scanCode, up, injected, now, picking);
            if let Some(signal) = o.signal {
                (s.emit)(signal);
            }
            o.swallow
        });
        if swallowed == Some(true) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn tick() {
    with(|s| {
        if !s.machine.holding() {
            return;
        }
        let now = s.epoch.elapsed().as_millis() as u64;
        if let Some(signal) = s.machine.on_tick(now, !on_user_desktop()) {
            (s.emit)(signal);
        }
    });
}

/// 输入桌面还是不是用户桌面。锁屏、UAC 时是 Winlogon；打不开（权限不够）也说明被切走了
fn on_user_desktop() -> bool {
    unsafe {
        let Ok(desk) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) else { return false };
        let mut name = [0u16; 64];
        let ok = GetUserObjectInformationW(
            HANDLE(desk.0),
            UOI_NAME,
            Some(name.as_mut_ptr() as *mut _),
            (name.len() * 2) as u32,
            None,
        )
        .is_ok();
        let _ = CloseDesktop(desk);
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        ok && String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case("Default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RALT: u32 = 0xA5;
    const ESC: u32 = 0x1B;
    const A: u32 = 0x41;

    fn key(m: &mut Machine, vk: u32, up: bool, t: u64) -> Outcome {
        m.on_key(vk, 0, up, false, t, false)
    }

    #[test]
    fn hold_then_release() {
        let mut m = Machine::new(Key::RightAlt);
        assert_eq!(key(&mut m, RALT, false, 0), swallow(Some(Signal::Press)));
        // 自动重复只吞不发
        assert_eq!(key(&mut m, RALT, false, 500), swallow(None));
        assert_eq!(key(&mut m, RALT, true, 900), swallow(Some(Signal::Release)));
        assert!(!m.holding());
    }

    #[test]
    fn short_tap_is_a_silent_cancel() {
        let mut m = Machine::new(Key::RightAlt);
        key(&mut m, RALT, false, 0);
        assert_eq!(key(&mut m, RALT, true, TAP_MS - 1), swallow(Some(Signal::Cancel)));
    }

    #[test]
    fn escape_while_holding_cancels_once() {
        let mut m = Machine::new(Key::RightAlt);
        key(&mut m, RALT, false, 0);
        assert_eq!(key(&mut m, ESC, false, 400), swallow(Some(Signal::Cancel)));
        assert_eq!(key(&mut m, ESC, false, 450), swallow(None), "自动重复也吞掉");
        assert_eq!(key(&mut m, ESC, true, 500), swallow(None), "吞了按下就得吞松开");
        assert_eq!(key(&mut m, ESC, false, 600), PASS, "第二下 Esc 不归我们");
        assert_eq!(key(&mut m, ESC, true, 650), PASS);
        assert_eq!(key(&mut m, RALT, true, 900), swallow(None), "取消过了，松开不再发");
        assert_eq!(key(&mut m, ESC, false, 1000), PASS, "没按着时 Esc 照常给前台");
    }

    #[test]
    fn escape_dismisses_candidates_only_when_picking() {
        let mut m = Machine::new(Key::RightAlt);
        assert_eq!(m.on_key(ESC, 0, false, false, 0, true), swallow(Some(Signal::Dismiss)));
        assert_eq!(m.on_key(ESC, 0, true, false, 10, true), swallow(None));
        assert_eq!(m.on_key(ESC, 0, false, false, 20, false), PASS);
    }

    #[test]
    fn injected_and_other_keys_pass_through() {
        let mut m = Machine::new(Key::RightAlt);
        assert_eq!(m.on_key(RALT, 0, false, true, 0, false), PASS, "注入的不算");
        key(&mut m, RALT, false, 0);
        assert_eq!(key(&mut m, A, false, 10), PASS);
        assert_eq!(key(&mut m, A, true, 20), PASS);
    }

    #[test]
    fn altgr_fake_ctrl_is_swallowed_only_for_right_alt() {
        let lctrl = VK_LCONTROL.0 as u32;
        let mut m = Machine::new(Key::RightAlt);
        assert_eq!(m.on_key(lctrl, ALTGR_FAKE_CTRL_SCAN, false, false, 0, false), swallow(None));
        assert_eq!(m.on_key(lctrl, 0x1D, false, false, 0, false), PASS, "真的左 Ctrl 不动");
        let mut m = Machine::new(Key::RightCtrl);
        assert_eq!(m.on_key(lctrl, ALTGR_FAKE_CTRL_SCAN, false, false, 0, false), PASS);
    }

    #[test]
    fn unseen_release_passes_through() {
        let mut m = Machine::new(Key::RightAlt);
        assert_eq!(key(&mut m, RALT, true, 0), PASS);
    }

    #[test]
    fn lost_release_is_recovered() {
        let mut m = Machine::new(Key::RightAlt);
        key(&mut m, RALT, false, 0);
        assert_eq!(m.on_tick(1_000, false), None);
        assert_eq!(m.on_tick(1_200, true), Some(Signal::Release), "锁屏后松开等不来了");
        assert!(!m.holding());

        key(&mut m, RALT, false, 0);
        assert_eq!(m.on_tick(MAX_HOLD_MS, false), Some(Signal::Release));

        key(&mut m, RALT, false, 0);
        key(&mut m, ESC, false, 300);
        assert_eq!(m.on_tick(1_000, true), None, "已经取消过的不再发松开");
    }

    #[test]
    fn switching_key_while_holding_releases() {
        let mut m = Machine::new(Key::RightAlt);
        key(&mut m, RALT, false, 0);
        assert_eq!(m.set_key(Key::RightCtrl), Some(Signal::Release));
        assert_eq!(key(&mut m, RALT, false, 10), PASS, "旧键不再归我们");
        assert_eq!(m.on_key(VK_RCONTROL.0 as u32, 0, false, false, 20, false), swallow(Some(Signal::Press)));
    }
}
