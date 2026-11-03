//! 界面层：三个窗口 + 托盘。
//!
//! 拆成四块，各自只做一件事：
//!
//! - `geometry`：屏幕坐标与缩放换算（唯一需要分平台的地方）
//! - `windows`：窗口的创建、摆放、显隐
//! - `overlay`：胶囊浮层的三态与延时收起
//! - `tray`：托盘图标与菜单
//!
//! 对外只暴露下面这几个函数，调用方（controller / ipc / main）不需要知道窗口的细节。

mod geometry;
mod overlay;
mod tray;
mod windows;

pub use overlay::{hud, hud_error, hud_hide};
pub use tray::{build_tray, refresh_tray};
pub use windows::{create_overlays, qr_armed, show_qr, show_settings};
