// GUI 子系统：双击 exe 不带出黑色控制台。没有控制台，诊断信息写进文件日志（见 log.rs）
#![windows_subsystem = "windows"]

mod controller;
mod ipc;
mod log;
mod remote;
mod session;
mod settings;
mod state;
mod ui;
mod wtoffline;

use settings::Settings;
use std::sync::Arc;
use tauri::{Manager, RunEvent, WindowEvent};
use tokio::sync::mpsc;
use voicekey_core::QwenEngine;

fn main() {
    log::init();
    let settings = Settings::load();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| ui::show_settings(app)))
        .plugin(tauri_plugin_autostart::Builder::new().args(["--minimized"]).build())
        .invoke_handler(tauri::generate_handler![
            ipc::get_state,
            ipc::set_settings,
            ipc::model_download,
            ipc::disconnect_phone,
            ipc::retry_remote,
            ipc::open_settings,
            ipc::probe_channel
        ])
        .setup(move |app| {
            let (tx, rx) = mpsc::unbounded_channel();
            let qwen = Arc::new(QwenEngine::default());
            state::manage(app.handle(), settings.clone(), tx, qwen.clone());

            let handle = app.handle().clone();
            controller::spawn(handle.clone(), qwen, rx);
            remote::spawn(handle.clone());
            ui::create_overlays(&handle)?;
            ui::build_tray(&handle)?;
            // 开机自启动以磁盘上的设置为准对齐一次
            if let Err(e) = state::apply(&handle, &settings) {
                log_warn!("{e}");
            }
            // 手动双击 exe：弹出设置窗（用户是特意来操作的）；
            // 开机自启动带 --minimized 参数，保持安静，入口在托盘
            if !std::env::args().any(|a| a == "--minimized") {
                ui::show_settings(&handle);
            }
            Ok(())
        })
        .on_window_event(|w, e| {
            // 扫码面板按原生托盘弹层的行为：点窗口外面（失焦）就收起
            if w.label() == "qr" && matches!(e, WindowEvent::Focused(false)) {
                if let Some(w) = w.app_handle().get_webview_window("qr") {
                    ui::qr_blurred(&w);
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build app")
        .run(|_app, e| if let RunEvent::ExitRequested { api, code: None, .. } = e { api.prevent_exit() });
}
