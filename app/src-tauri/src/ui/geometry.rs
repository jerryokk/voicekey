//! 屏幕坐标与尺寸换算。
//!
//! 这是整个界面层唯一需要区分平台的地方：macOS 用逻辑点、Windows 用物理像素，
//! 而 150% 缩放下两者相差 1.5 倍。所有摆窗口的代码都从这里取数，
//! 免得每个调用点各自乘一次缩放系数。

use tauri::{AppHandle, LogicalSize, Monitor, Runtime, WebviewWindow};

/// 一块工作区（已扣掉任务栏/程序坞）
#[derive(Clone, Copy)]
pub(super) struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
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

/// 把逻辑尺寸换算成当前平台使用的单位
pub(super) fn units(a: &Area, logical: f64) -> f64 {
    if cfg!(target_os = "macos") {
        logical
    } else {
        logical * a.scale
    }
}

pub(super) fn set_pos<R: Runtime>(w: &WebviewWindow<R>, x: f64, y: f64) {
    let _ = if cfg!(target_os = "macos") {
        w.set_position(tauri::LogicalPosition::new(x, y))
    } else {
        w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32))
    };
}

pub(super) fn set_client<R: Runtime>(w: &WebviewWindow<R>, a: &Area, logical_w: f64, logical_h: f64) {
    let _ = if cfg!(target_os = "macos") {
        w.set_size(LogicalSize::new(logical_w, logical_h))
    } else {
        w.set_size(tauri::PhysicalSize::new(
            (logical_w * a.scale).ceil() as u32,
            (logical_h * a.scale).ceil() as u32,
        ))
    };
}

/// 光标所在显示器的工作区。取不到光标位置时退回主显示器——
/// 浮层要贴着用户正在看的那块屏幕，而不是固定主屏。
pub(super) fn cursor_area<R: Runtime>(app: &AppHandle<R>) -> Option<Area> {
    let p = app.cursor_position().ok();
    let m = p
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())?;
    Some(work_area(&m))
}
