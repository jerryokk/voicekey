// GUI 子系统：双击 exe 不带出黑色控制台；从终端启动时 stderr 日志仍写入该终端
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

// 这些名字在 crate 各处被引用（controller / remote / ipc / ui），集中从这里转出
pub use state::{model_ready, update_settings, AppState, RemoteStatus};

use controller::Msg;
use serde_json::json;
use settings::{Channel, Settings};
use std::sync::Arc;
use tauri::{AppHandle, Manager, RunEvent, Runtime, WindowEvent};
use tokio::sync::mpsc;
use voicekey_core::QwenEngine;

/// 托盘菜单点一项。id 在 `ui::tray_menu` 里生成，这里是唯一的分支处。
fn tray_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        // 第一项：直接弹扫码小窗，不用先开设置页
        "qr" => ui::show_qr(app),
        "open" => ui::show_settings(app),
        "quit" => app.exit(0),
        _ => {
            if let Some(ch) = id.strip_prefix("ch:").and_then(|c| serde_json::from_value::<Channel>(json!(c)).ok()) {
                update_settings(app, |s| s.channel = ch);
            }
        }
    }
}

fn main() {
    log::init();
    let settings = Settings::load();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| ui::show_settings(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .invoke_handler(tauri::generate_handler![
            ipc::get_state,
            ipc::set_settings,
            ipc::model_download,
            ipc::disconnect_phone,
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
            ui::build_tray(&handle, &settings)?;
            // 托盘菜单与开机自启动都以磁盘上的设置为准对齐一次
            state::apply(&handle, &settings);
            // 手动双击 exe：弹出设置窗（用户是特意来操作的）；
            // 开机自启动带 --minimized 参数，保持安静，入口在托盘
            if !std::env::args().any(|a| a == "--minimized") {
                ui::show_settings(&handle);
            }
            Ok(())
        })
        .on_window_event(|w, e| {
            if w.label() == "main" && matches!(e, WindowEvent::Destroyed) {
                let _ = w.state::<AppState>().tx.send(Msg::Flush);
            }
            // 二维码小窗按原生托盘弹层的行为：点窗口外面（失焦）就收起
            if w.label() == "qr" {
                if let WindowEvent::Focused(false) = e {
                    if ui::qr_armed() {
                        let _ = w.hide();
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build app")
        .run(|_app, e| if let RunEvent::ExitRequested { api, code: None, .. } = e { api.prevent_exit() });
}
