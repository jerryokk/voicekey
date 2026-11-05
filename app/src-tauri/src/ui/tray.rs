//! 托盘图标与菜单。
//!
//! 左键：弹出扫码面板（二维码 + 渠道选择），这是产品的主入口。
//! 右键：只有「打开」「退出」两项。
//! 有手机在线时图标换成高亮版，悬停提示里写着几台；开了电脑麦克风的话也写上按哪个键。

use std::sync::atomic::{AtomicUsize, Ordering};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::state::AppState;

const TRAY: &[u8] = include_bytes!("../../icons/tray.png");
const TRAY_ACTIVE: &[u8] = include_bytes!("../../icons/tray-active.png");

/// 在线手机数：设置变了重写提示时要用
static DEVICES: AtomicUsize = AtomicUsize::new(0);

fn tooltip<R: Runtime>(app: &AppHandle<R>, devices: usize) -> String {
    let mut s = format!("VoiceKey {}", env!("CARGO_PKG_VERSION"));
    if devices > 0 {
        s += &format!(" · {devices} 台手机在线");
    }
    let st = app.state::<AppState>();
    let local = st.settings.read().unwrap().local.clone();
    if local.enabled && st.local_status.read().unwrap().state == "ready" {
        s += &format!(" · 按住{}说话", local.hotkey.title());
    }
    s
}

pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "打开 VoiceKey", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(TRAY)?)
        .tooltip(tooltip(app, 0))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => super::show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                super::toggle_qr(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// 在线设备数变了：换图标、改提示
pub fn tray_devices<R: Runtime>(app: &AppHandle<R>, devices: usize) {
    DEVICES.store(devices, Ordering::Relaxed);
    let Some(t) = app.tray_by_id("main") else { return };
    if let Ok(i) = Image::from_bytes(if devices > 0 { TRAY_ACTIVE } else { TRAY }) {
        let _ = t.set_icon(Some(i));
    }
    let _ = t.set_tooltip(Some(tooltip(app, devices)));
}

/// 设置或电脑麦克风状态变了：只改提示
pub fn tray_refresh<R: Runtime>(app: &AppHandle<R>) {
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_tooltip(Some(tooltip(app, DEVICES.load(Ordering::Relaxed))));
    }
}
