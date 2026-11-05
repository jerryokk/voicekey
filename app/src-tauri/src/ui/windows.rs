//! 窗口的创建与摆放：设置窗、扫码面板、收件面板、透明浮层。
//!
//! 都做成无系统标题栏——顶栏由界面自己画，和内容同色，不再是 Windows 那层灰条。
//! 扫码面板和收件面板共用托盘那一角：同一时间只显示其中一个。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::geometry::{cursor_area, set_client, set_pos, units, Area};

const HUD_W: f64 = 680.0;
pub(super) const HUD_H: f64 = 120.0;
/// 浮层展示候选时的高度：候选最多八行
pub(super) const HUD_ROWS_H: f64 = 420.0;
/// 扫码面板：二维码在上，3×3 渠道格在下
const QR_W: f64 = 340.0;
const QR_H: f64 = 586.0;
/// 收件面板与扫码面板同宽；高度由内容决定，这里是上限
const INBOX_W: f64 = 340.0;
const INBOX_MAX_H: f64 = 560.0;

/// 扫码窗刚 show 出来时会有一次瞬时失焦（焦点还没落定），
/// 用这个标记把那次抖动挡掉，否则窗口会闪一下自己关掉
static QR_ARMED: AtomicBool = AtomicBool::new(false);

/// 上一次因失焦收起的时刻。点托盘图标时窗口先失焦收起、再收到点击，
/// 不看这个时间的话「再点一下关掉」会变成「关掉又弹出来」
static QR_HIDDEN_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// 失焦：面板像原生托盘弹层一样收起（刚弹出时的那次瞬时失焦除外）
pub fn qr_blurred<R: Runtime>(w: &WebviewWindow<R>) {
    if QR_ARMED.load(Ordering::Relaxed) {
        let _ = w.hide();
        *QR_HIDDEN_AT.lock().unwrap() = Some(Instant::now());
    }
}

/// 托盘左键：没显示就弹出，显示着就收起。收件面板开着的话先让位
pub fn toggle_qr<R: Runtime>(app: &AppHandle<R>) {
    let just_hidden = QR_HIDDEN_AT.lock().unwrap().take().is_some_and(|t| t.elapsed() < Duration::from_millis(300));
    let visible = app.get_webview_window("qr").and_then(|w| w.is_visible().ok()).unwrap_or(false);
    if visible {
        if let Some(w) = app.get_webview_window("qr") {
            let _ = w.hide();
        }
    } else if !just_hidden {
        hide_inbox(app);
        show_qr(app);
    }
}

/// 透明浮层：不抢焦点、不上任务栏、不进 Alt+Tab。平时点击直接穿过它
fn overlay<R: Runtime>(app: &AppHandle<R>, label: &str, w: f64, h: f64) -> tauri::Result<WebviewWindow<R>> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .initialization_script(format!("window.__VK_VIEW = {label:?};"))
        .title(label)
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .visible(false)
        .focused(false)
        .focusable(false)
        .visible_on_all_workspaces(true)
        .inner_size(w, h)
        .build()
}

pub fn create_overlays<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    overlay(app, "hud", HUD_W, HUD_H)?.set_ignore_cursor_events(true)?;
    inbox(app)?;
    Ok(())
}

/// 显示一个浮在别人上面的窗口但不激活它，`show()` 会激活
pub(super) fn show_quietly<R: Runtime>(w: &WebviewWindow<R>) {
    match w.hwnd() {
        Ok(h) => voicekey_platform::show_no_activate(h.0 as isize),
        Err(_) => {
            let _ = w.show();
        }
    }
}

/// 收件面板：弹出时不激活（用户可能正在打字，焦点不能被拿走，见 [`show_inbox`]），
/// 但它本身可以激活——WebView2 在不可激活的窗口里会把每次的第一下点击吞掉，按钮要点两下才有反应
fn inbox<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    WebviewWindowBuilder::new(app, "inbox", WebviewUrl::App("index.html".into()))
        .initialization_script("window.__VK_VIEW = \"inbox\";")
        .title("VoiceKey 收件")
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .shadow(true)
        .visible(false)
        .focused(false)
        .inner_size(INBOX_W, 200.0)
        .build()
}

pub fn show_settings<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let b = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .initialization_script("window.__VK_VIEW = \"settings\";")
        .title("VoiceKey 设置")
        .inner_size(860.0, 600.0)
        .resizable(false)
        .maximizable(false)
        .decorations(false)
        .shadow(true)
        .center();
    if let Ok(w) = b.build() {
        let _ = w.set_focus();
    }
}

/// 扫码小窗贴着托盘那一角弹出（光标所在显示器的工作区右下角），
/// 而不是屏幕正中——它是个从托盘飞出来的临时面板
fn show_qr<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("qr") {
        place_qr(&w);
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        arm_qr();
        return;
    }
    let b = WebviewWindowBuilder::new(app, "qr", WebviewUrl::App("index.html".into()))
        .initialization_script("window.__VK_VIEW = \"qr\";")
        .title("VoiceKey")
        .inner_size(QR_W, QR_H)
        .resizable(false)
        .maximizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .decorations(false)
        .shadow(true)
        .visible(false);
    if let Ok(w) = b.build() {
        place_qr(&w);
        let _ = w.show();
        let _ = w.set_focus();
        arm_qr();
    }
}

/// 显示后短暂屏蔽失焦事件，之后才允许「点外面自动关」
fn arm_qr() {
    QR_ARMED.store(false, Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(450)).await;
        QR_ARMED.store(true, Ordering::Relaxed);
    });
}

/// 贴着工作区右下角放：用外框尺寸算（含边框），留 12pt 边距。
/// 工作区已经扣掉了任务栏，所以不用再避让它。
fn corner<R: Runtime>(w: &WebviewWindow<R>, a: &Area, logical_w: f64, logical_h: f64) {
    set_client(w, a, logical_w, logical_h);
    let (ww, wh) = w
        .outer_size()
        .map(|s| (s.width as f64, s.height as f64))
        .unwrap_or((units(a, logical_w), units(a, logical_h)));
    let margin = units(a, 12.0);
    set_pos(w, a.x + a.w - ww - margin, a.y + a.h - wh - margin);
}

fn place_qr<R: Runtime>(w: &WebviewWindow<R>) {
    if let Some(a) = cursor_area(w.app_handle()) {
        corner(w, &a, QR_W, QR_H);
    }
}

/// 收件面板：内容变高变矮都贴着右下角重摆。扫码面板开着就收起它，两个面板不叠在一起
pub fn show_inbox<R: Runtime>(app: &AppHandle<R>, logical_h: f64) {
    let Some(w) = app.get_webview_window("inbox") else { return };
    if let Some(qr) = app.get_webview_window("qr") {
        let _ = qr.hide();
    }
    if let Some(a) = cursor_area(app) {
        corner(&w, &a, INBOX_W, logical_h.clamp(80.0, INBOX_MAX_H));
    }
    if !shown(&w) {
        show_quietly(&w);
    }
}

pub fn hide_inbox<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("inbox") {
        hide_quietly(&w);
    }
}

/// 与 [`show_quietly`] 配对：窗口是绕过 tao 显示的，tao 记的可见状态不准，
/// 它的 `hide()` 会以为本来就藏着而什么都不做
pub(super) fn hide_quietly<R: Runtime>(w: &WebviewWindow<R>) {
    match w.hwnd() {
        Ok(h) => voicekey_platform::hide_window(h.0 as isize),
        Err(_) => {
            let _ = w.hide();
        }
    }
}

/// 窗口此刻是不是真的显示着（不问 tao，理由同上）
pub(super) fn shown<R: Runtime>(w: &WebviewWindow<R>) -> bool {
    w.hwnd().map(|h| voicekey_platform::window_visible(h.0 as isize)).unwrap_or(false)
}

/// 胶囊浮层的位置：底部居中，离工作区底边 24pt
pub(super) fn place_hud<R: Runtime>(w: &WebviewWindow<R>, a: &Area, logical_h: f64) {
    let (ww, hh) = (units(a, HUD_W), units(a, logical_h));
    set_client(w, a, HUD_W, logical_h);
    set_pos(w, a.x + (a.w - ww) / 2.0, a.y + a.h - hh - units(a, 24.0));
}
