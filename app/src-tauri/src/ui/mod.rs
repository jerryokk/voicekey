//! 界面层：设置窗、扫码面板、胶囊浮层，加上托盘。
//!
//! 拆成四块，各自只做一件事：
//!
//! - `geometry`：屏幕坐标与缩放换算
//! - `windows`：窗口的创建、摆放、显隐
//! - `overlay`：胶囊浮层的三态与延时收起
//! - `tray`：托盘图标、左键弹面板、右键菜单
//!
//! 对外只暴露下面这几个函数，调用方（controller / ipc / main）不需要知道窗口的细节。

mod geometry;
mod overlay;
mod tray;
mod windows;

pub use overlay::{hud, hud_error, hud_hide};
pub use tray::{build_tray, tray_devices};
pub use windows::{create_overlays, qr_blurred, show_settings, toggle_qr};
