//! 窗口与托盘：设置窗口、胶囊浮层。
//!
//! 候选面板没有了——挑哪一条在手机上做，所以这里不再需要光标定位，也不会再抢焦点。
use crate::settings::{Channel, Settings};
use crate::AppState;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, Monitor, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const TRAY: &[u8] = include_bytes!("../icons/tray.png");
const TRAY_ACTIVE: &[u8] = include_bytes!("../icons/tray-active.png");
const HUD_W: f64 = 680.0;
const HUD_H: f64 = 120.0;

static HUD_TOKEN: AtomicU64 = AtomicU64::new(0);
/// 二维码窗口刚 show 出来时会有一次瞬时失焦（焦点还没落定），
/// 用这个标记把那次抖动挡掉，否则窗口会闪一下自己关掉
static QR_ARMED: AtomicBool = AtomicBool::new(false);

/// 二维码窗口是否已经稳定显示（可以响应「点外面自动关」）
pub fn qr_armed() -> bool {
    QR_ARMED.load(Ordering::Relaxed)
}

#[derive(Clone, Copy)]
struct Area {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale: f64,
}

fn work_area(m: &Monitor) -> Area {
    let wa = m.work_area();
    let s = m.scale_factor();
    let k = if cfg!(target_os = "macos") { s } else { 1.0 };
    Area {
        x: wa.position.x as f64 / k,
        y: wa.position.y as f64 / k,
        w: wa.size.width as f64 / k,
        h: wa.size.height as f64 / k,
        scale: s,
    }
}

/// 统一坐标空间下的尺寸换算：macOS 用逻辑点，Windows 用物理像素
fn units(a: &Area, logical: f64) -> f64 {
    if cfg!(target_os = "macos") {
        logical
    } else {
        logical * a.scale
    }
}

fn set_pos<R: Runtime>(w: &WebviewWindow<R>, x: f64, y: f64) {
    let _ = if cfg!(target_os = "macos") {
        w.set_position(tauri::LogicalPosition::new(x, y))
    } else {
        w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32))
    };
}

fn set_client<R: Runtime>(w: &WebviewWindow<R>, a: &Area, logical_w: f64, logical_h: f64) {
    let _ = if cfg!(target_os = "macos") {
        w.set_size(LogicalSize::new(logical_w, logical_h))
    } else {
        w.set_size(tauri::PhysicalSize::new(
            (logical_w * a.scale).ceil() as u32,
            (logical_h * a.scale).ceil() as u32,
        ))
    };
}

fn cursor_area<R: Runtime>(app: &AppHandle<R>) -> Option<Area> {
    let p = app.cursor_position().ok();
    let m = p
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())?;
    Some(work_area(&m))
}

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
        .center();
    #[cfg(target_os = "macos")]
    let b = b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    if let Ok(w) = b.build() {
        let _ = w.set_focus();
    }
}

/// 二维码小窗贴着托盘那一角弹出（光标所在显示器的工作区右下角），
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
        .inner_size(320.0, 344.0)
        .resizable(false)
        .maximizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
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

/// 右下角定位：用外框尺寸算，留 12pt 边距，避免贴边（任务栏工作区已经扣掉了任务栏本身）
fn place_qr<R: Runtime>(w: &WebviewWindow<R>) {
    let Some(a) = cursor_area(w.app_handle()) else { return };
    let (mut ww, mut wh) = w
        .outer_size()
        .map(|s| (s.width as f64, s.height as f64))
        .unwrap_or((units(&a, 320.0), units(&a, 344.0)));
    if cfg!(target_os = "macos") {
        ww /= a.scale;
        wh /= a.scale;
    }
    let margin = units(&a, 12.0);
    set_pos(w, a.x + a.w - ww - margin, a.y + a.h - wh - margin);
}

fn current_channel<R: Runtime>(app: &AppHandle<R>) -> Channel {
    app.state::<AppState>().settings.read().unwrap().channel
}

/// 胶囊浮层：state = listen | wait | error | info
pub fn hud<R: Runtime>(app: &AppHandle<R>, state: &str, text: &str, ch: Channel) {
    HUD_TOKEN.fetch_add(1, Ordering::Relaxed);
    let _ = app.emit_to("hud", "hud", json!({ "state": state, "text": text, "channel": ch }));
    let Some(w) = app.get_webview_window("hud") else { return };
    if !w.is_visible().unwrap_or(false) {
        if let Some(a) = cursor_area(app) {
            let (ww, hh) = (units(&a, HUD_W), units(&a, HUD_H));
            set_client(&w, &a, HUD_W, HUD_H);
            set_pos(&w, a.x + (a.w - ww) / 2.0, a.y + a.h - hh - units(&a, 24.0));
        }
        let _ = w.show();
    }
}

pub fn hud_error<R: Runtime>(app: &AppHandle<R>, msg: &str) {
    hud(app, "error", msg, current_channel(app));
    hud_hide(app, 2500);
}

pub fn hud_hide<R: Runtime>(app: &AppHandle<R>, delay_ms: u64) {
    let token = HUD_TOKEN.load(Ordering::Relaxed);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if HUD_TOKEN.load(Ordering::Relaxed) != token {
            return;
        }
        let _ = app.emit_to("hud", "hud", json!({ "state": "hidden" }));
        tokio::time::sleep(Duration::from_millis(160)).await;
        if HUD_TOKEN.load(Ordering::Relaxed) == token {
            if let Some(w) = app.get_webview_window("hud") {
                let _ = w.hide();
            }
        }
    });
}

fn tray_menu<R: Runtime>(app: &AppHandle<R>, s: &Settings) -> tauri::Result<Menu<R>> {
    let mut channels: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
    for c in Channel::available() {
        let id = format!("ch:{}", serde_json::to_value(c)?.as_str().unwrap_or(""));
        channels.push(Box::new(CheckMenuItem::with_id(app, id, c.title(), true, s.channel == c, None::<&str>)?));
    }
    let refs: Vec<&dyn IsMenuItem<R>> = channels.iter().map(|b| b.as_ref()).collect();
    let sub = Submenu::with_items(app, "渠道", true, &refs)?;

    let mut items: Vec<Box<dyn IsMenuItem<R>>> = vec![
        Box::new(MenuItem::with_id(app, "qr", "显示二维码", true, None::<&str>)?),
        Box::new(sub),
        Box::new(PredefinedMenuItem::separator(app)?),
        Box::new(MenuItem::with_id(app, "open", "打开 VoiceKey", true, None::<&str>)?),
    ];
    if let Some(v) = crate::update::available() {
        items.push(Box::new(MenuItem::with_id(app, "update", format!("更新到 v{v} 并重启"), true, None::<&str>)?));
    } else if crate::update::downloading() {
        items.push(Box::new(MenuItem::with_id(app, "update", "正在下载更新…", false, None::<&str>)?));
    }
    items.push(Box::new(MenuItem::with_id(app, "quit", "退出 VoiceKey", true, None::<&str>)?));

    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

fn tray_icon<R: Runtime>(_app: &AppHandle<R>, active: bool) -> Option<Image<'static>> {
    // 托盘专用图（透明底、纯字形、笔画加粗），不能拿窗口图标代替：
    // 窗口图标是带深色底的徽章，缩到托盘尺寸只剩一块黑方块
    Image::from_bytes(if active { TRAY_ACTIVE } else { TRAY }).ok()
}

pub fn build_tray<R: Runtime>(app: &AppHandle<R>, s: &Settings) -> tauri::Result<()> {
    let mut b = TrayIconBuilder::with_id("main")
        .menu(&tray_menu(app, s)?)
        // 左键直接弹二维码（配手机时的高频动作），右键才是菜单
        .show_menu_on_left_click(false)
        .tooltip("VoiceKey")
        .on_menu_event(|app, ev| crate::tray_event(app, ev.id().as_ref()))
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show_qr(tray.app_handle());
            }
        });
    if cfg!(target_os = "macos") {
        b = b.icon_as_template(true);
    }
    if let Some(i) = tray_icon(app, false) {
        b = b.icon(i);
    }
    b.build(app)?;
    Ok(())
}

pub fn refresh_tray<R: Runtime>(app: &AppHandle<R>, s: &Settings) {
    if let (Some(t), Ok(m)) = (app.tray_by_id("main"), tray_menu(app, s)) {
        let _ = t.set_menu(Some(m));
    }
}
