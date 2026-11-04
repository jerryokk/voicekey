//! 屏幕坐标与尺寸换算。
//!
//! Windows 下窗口位置与尺寸都按物理像素算，150% 缩放时逻辑尺寸要乘 1.5。
//! 所有摆窗口的代码都从这里取数，免得每个调用点各自乘一次缩放系数。

use tauri::{AppHandle, Monitor, Runtime, WebviewWindow};

/// 一块工作区（已扣掉任务栏），物理像素
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
    Area {
        x: wa.position.x as f64,
        y: wa.position.y as f64,
        w: wa.size.width as f64,
        h: wa.size.height as f64,
        scale: m.scale_factor(),
    }
}

/// 逻辑尺寸换算成物理像素
pub(super) fn units(a: &Area, logical: f64) -> f64 {
    logical * a.scale
}

pub(super) fn set_pos<R: Runtime>(w: &WebviewWindow<R>, x: f64, y: f64) {
    let _ = w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

pub(super) fn set_client<R: Runtime>(w: &WebviewWindow<R>, a: &Area, logical_w: f64, logical_h: f64) {
    let _ = w.set_size(tauri::PhysicalSize::new(
        (logical_w * a.scale).ceil() as u32,
        (logical_h * a.scale).ceil() as u32,
    ));
}

/// 光标所在显示器的工作区：浮层和扫码面板要贴着用户正在看的那块屏幕
pub(super) fn cursor_area<R: Runtime>(app: &AppHandle<R>) -> Option<Area> {
    let p = app.cursor_position().ok()?;
    let m = app.monitor_from_point(p.x, p.y).ok().flatten()?;
    Some(work_area(&m))
}
