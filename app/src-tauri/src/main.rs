// GUI 子系统：双击 exe 不带出黑色控制台。没有控制台，诊断信息写进文件日志（见 log.rs）
#![windows_subsystem = "windows"]

mod controller;
mod inbox;
mod ipc;
mod local;
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
use voicekey_remote::Floor;

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
            ipc::probe_channel,
            ipc::list_mics,
            ipc::mic_test,
            ipc::mic_retry,
            ipc::open_mic_privacy,
            ipc::hud_choose,
            ipc::inbox_items,
            ipc::inbox_copy,
            ipc::inbox_reveal,
            ipc::inbox_dismiss,
            ipc::inbox_resize
        ])
        .setup(move |app| {
            let (tx, rx) = mpsc::unbounded_channel();
            let qwen = Arc::new(QwenEngine::default());
            // 发言权建一次：手机接入整套重启时它不动，电脑本机麦克风也挂在它上面
            let (floor, events) = Floor::new();
            state::manage(app.handle(), settings.clone(), tx, qwen.clone());

            let handle = app.handle().clone();
            controller::spawn(handle.clone(), qwen, rx);
            // 两个声源的事件汇进同一条通道交给会话。这条通道不经过手机接入：
            // 隧道没起来、起不来、重启中，电脑本机麦克风照样能用
            let to_session = handle.state::<state::AppState>().tx.clone();
            let mut events = events;
            tauri::async_runtime::spawn(async move {
                while let Some(e) = events.recv().await {
                    let _ = to_session.send(session::types::Msg::Remote(e));
                }
            });
            ui::create_overlays(&handle)?;
            ui::build_tray(&handle)?;
            local::spawn(handle.clone(), floor.clone(), settings.local.clone());
            remote::spawn(handle.clone(), floor);
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
