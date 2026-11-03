//! 三个窗口的创建与摆放：设置窗、扫码小窗、透明浮层。
//!
//! 都做成无系统标题栏——顶栏由界面自己画，和内容同色，不再是 Windows 那层灰条。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::geometry::{cursor_area, set_client, set_pos, units};

const HUD_W: f64 = 680.0;
const HUD_H: f64 = 120.0;
const QR_W: f64 = 320.0;
const QR_H: f64 = 378.0;

/// 扫码窗刚 show 出来时会有一次瞬时失焦（焦点还没落定），
/// 用这个标记把那次抖动挡掉，否则窗口会闪一下自己关掉
static QR_ARMED: AtomicBool = AtomicBool::new(false);

/// 扫码窗是否已经稳定显示（可以响应「点外面自动关」）
pub fn qr_armed() -> bool {
    QR_ARMED.load(Ordering::Relaxed)
}

/// 透明浮层：不抢焦点、不上任务栏、不进 Alt+Tab
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
    Ok(())
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
    #[cfg(target_os = "macos")]
    let b = b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    if let Ok(w) = b.build() {
        let _ = w.set_focus();
    }
}

/// 扫码小窗贴着托盘那一角弹出（光标所在显示器的工作区右下角），
/// 而不是屏幕正中——它是个从托盘飞出来的临时面板
pub fn show_qr<R: Runtime>(app: &AppHandle<R>) {
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
        .title("扫码连接手机")
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

/// 右下角定位：用外框尺寸算（含边框），留 12pt 边距。
/// 工作区已经扣掉了任务栏，所以不用再避让它。
fn place_qr<R: Runtime>(w: &WebviewWindow<R>) {
    let Some(a) = cursor_area(w.app_handle()) else { return };
    let (mut ww, mut wh) = w
        .outer_size()
        .map(|s| (s.width as f64, s.height as f64))
        .unwrap_or((units(&a, QR_W), units(&a, QR_H)));
    if cfg!(target_os = "macos") {
        ww /= a.scale;
        wh /= a.scale;
    }
    let margin = units(&a, 12.0);
    set_pos(w, a.x + a.w - ww - margin, a.y + a.h - wh - margin);
}

/// 胶囊浮层的位置：底部居中，离工作区底边 24pt
pub(super) fn place_hud<R: Runtime>(w: &WebviewWindow<R>, a: &super::geometry::Area) {
    let (ww, hh) = (units(a, HUD_W), units(a, HUD_H));
    set_client(w, a, HUD_W, HUD_H);
    set_pos(w, a.x + (a.w - ww) / 2.0, a.y + a.h - hh - units(a, 24.0));
}
