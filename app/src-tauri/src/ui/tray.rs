//! 托盘图标与菜单。
//!
//! 左键：弹出扫码面板（二维码 + 渠道选择），这是产品的主入口。
//! 右键：只有「打开」「退出」两项。
//! 有手机在线时图标换成高亮版，悬停提示里写着几台。

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Runtime};

const TRAY: &[u8] = include_bytes!("../../icons/tray.png");
const TRAY_ACTIVE: &[u8] = include_bytes!("../../icons/tray-active.png");

fn tooltip(devices: usize) -> String {
    let base = format!("VoiceKey {}", env!("CARGO_PKG_VERSION"));
    if devices == 0 {
        base
    } else {
        format!("{base} · {devices} 台手机在线")
    }
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
        .tooltip(tooltip(0))
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
    let Some(t) = app.tray_by_id("main") else { return };
    if let Ok(i) = Image::from_bytes(if devices > 0 { TRAY_ACTIVE } else { TRAY }) {
        let _ = t.set_icon(Some(i));
    }
    let _ = t.set_tooltip(Some(tooltip(devices)));
}
