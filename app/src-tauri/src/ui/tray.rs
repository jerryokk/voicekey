//! 托盘图标与菜单。
//!
//! 点击托盘出菜单（左键右键一致），菜单项就是产品的主入口：
//! 显示二维码 / 渠道（二级菜单）/ 打开 / 退出。第一行是不可点的版本号——
//! 单实例机制下旧进程会吞掉新启动，这行是判断「我现在点的是哪一版」的唯一依据。

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Runtime};

use crate::settings::{Channel, Settings};

const TRAY: &[u8] = include_bytes!("../../icons/tray.png");
const TRAY_ACTIVE: &[u8] = include_bytes!("../../icons/tray-active.png");

fn tray_menu<R: Runtime>(app: &AppHandle<R>, s: &Settings) -> tauri::Result<Menu<R>> {
    let mut channels: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
    for c in Channel::available() {
        let id = format!("ch:{}", serde_json::to_value(c)?.as_str().unwrap_or(""));
        channels.push(Box::new(CheckMenuItem::with_id(app, id, c.title(), true, s.channel == c, None::<&str>)?));
    }
    let refs: Vec<&dyn IsMenuItem<R>> = channels.iter().map(|b| b.as_ref()).collect();
    let sub = Submenu::with_items(app, "渠道", true, &refs)?;

    let items: Vec<Box<dyn IsMenuItem<R>>> = vec![
        Box::new(MenuItem::with_id(
            app,
            "ver",
            format!("VoiceKey {} · {}", env!("CARGO_PKG_VERSION"), env!("VK_BUILD")),
            false,
            None::<&str>,
        )?),
        Box::new(PredefinedMenuItem::separator(app)?),
        Box::new(MenuItem::with_id(app, "qr", "显示二维码", true, None::<&str>)?),
        Box::new(sub),
        Box::new(PredefinedMenuItem::separator(app)?),
        Box::new(MenuItem::with_id(app, "open", "打开 VoiceKey", true, None::<&str>)?),
        Box::new(MenuItem::with_id(app, "quit", "退出 VoiceKey", true, None::<&str>)?),
    ];

    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|b| b.as_ref()).collect();
    Menu::with_items(app, &refs)
}

/// 托盘专用图（透明底、纯字形、笔画加粗）。不能拿窗口图标代替：
/// 窗口图标是带深色底的徽章，缩到托盘尺寸只剩一块黑方块。
fn tray_icon<R: Runtime>(_app: &AppHandle<R>, active: bool) -> Option<Image<'static>> {
    Image::from_bytes(if active { TRAY_ACTIVE } else { TRAY }).ok()
}

pub fn build_tray<R: Runtime>(app: &AppHandle<R>, s: &Settings) -> tauri::Result<()> {
    let mut b = TrayIconBuilder::with_id("main")
        .menu(&tray_menu(app, s)?)
        .show_menu_on_left_click(true)
        .tooltip(format!("VoiceKey {} · {}", env!("CARGO_PKG_VERSION"), env!("VK_BUILD")))
        .on_menu_event(|app, ev| crate::tray_event(app, ev.id().as_ref()));
    if cfg!(target_os = "macos") {
        b = b.icon_as_template(true);
    }
    if let Some(i) = tray_icon(app, false) {
        b = b.icon(i);
    }
    b.build(app)?;
    Ok(())
}

/// 渠道变了之后重建菜单（勾选项跟着变）
pub fn refresh_tray<R: Runtime>(app: &AppHandle<R>, s: &Settings) {
    if let (Some(t), Ok(m)) = (app.tray_by_id("main"), tray_menu(app, s)) {
        let _ = t.set_menu(Some(m));
    }
}
