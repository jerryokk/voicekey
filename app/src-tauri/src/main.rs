#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod controller;
mod remote;
mod settings;
mod ui;
mod update;
mod wtoffline;

use controller::Msg;
use serde_json::json;
use settings::{Channel, Settings};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tokio::sync::mpsc::{self, UnboundedSender};
use voicekey_core::QwenEngine;
use voicekey_remote::Handle as RemoteHandle;

pub struct AppState {
    pub settings: Arc<RwLock<Settings>>,
    pub tx: UnboundedSender<Msg>,
    pub qwen: Arc<QwenEngine>,
    /// 托盘「暂停监听」：暂停后手机怎么按都不响应
    pub paused: AtomicBool,
    /// 手机接入服务起来之后才有值
    pub remote: Arc<RwLock<Option<RemoteHandle>>>,
    pub remote_url: Arc<RwLock<Option<String>>>,
}

fn apply<R: Runtime>(app: &AppHandle<R>, s: &Settings) {
    let st = app.state::<AppState>();
    ui::refresh_tray(app, s, st.paused.load(Ordering::Relaxed));
    let al = app.autolaunch();
    if al.is_enabled().unwrap_or(false) != s.autostart {
        let _ = if s.autostart { al.enable() } else { al.disable() };
    }
}

pub fn update_settings<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&mut Settings)) {
    let s = {
        let st = app.state::<AppState>();
        let mut g = st.settings.write().unwrap();
        f(&mut g);
        g.sanitize();
        g.save();
        g.clone()
    };
    apply(app, &s);
    download_model(app, s.channel);
    let _ = app.emit("settings", &s);
}

/// 本地模型渠道：开始下载（已下载或下载中则忽略）
fn download_model<R: Runtime>(app: &AppHandle<R>, ch: Channel) {
    if ch == Channel::WetypeOffline {
        wtoffline::download(app);
    }
}

pub fn model_ready(ch: Channel) -> bool {
    match ch {
        Channel::WetypeOffline => wtoffline::installed(),
        _ => true,
    }
}

pub fn tray_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        "settings" => ui::show_settings(app),
        "update" => update::install(app),
        "quit" => app.exit(0),
        "pause" => {
            let st = app.state::<AppState>();
            let p = !st.paused.load(Ordering::Relaxed);
            st.paused.store(p, Ordering::Relaxed);
            let s = st.settings.read().unwrap().clone();
            ui::refresh_tray(app, &s, p);
        }
        _ => {
            if let Some(ch) = id.strip_prefix("ch:").and_then(|c| serde_json::from_value::<Channel>(json!(c)).ok()) {
                update_settings(app, |s| s.channel = ch);
            }
        }
    }
}

/// 二维码用 SVG 直接给前端，省掉一个前端依赖，也不怕离线加载不到库
fn qr_svg(url: &str) -> Option<String> {
    let code = qrcode::QrCode::new(url.as_bytes()).ok()?;
    Some(code.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).quiet_zone(true).build())
}

#[tauri::command]
fn get_state(app: AppHandle) -> serde_json::Value {
    let st = app.state::<AppState>();
    let s = st.settings.read().unwrap().clone();
    let url = st.remote_url.read().unwrap().clone();
    json!({
        "settings": s,
        "platform": if cfg!(target_os = "macos") { "mac" } else { "win" },
        "version": app.package_info().version.to_string(),
        "channels": Channel::available(),
        "models": { "wetypeoffline": wtoffline::status() },
        "paused": st.paused.load(Ordering::Relaxed),
        "remote": { "url": url, "qr": url.as_deref().and_then(qr_svg) },
        "update": update::status(),
    })
}

#[tauri::command]
fn set_settings(app: AppHandle, settings: Settings) {
    update_settings(&app, |s| *s = settings);
}

#[tauri::command]
fn model_download(app: AppHandle, ch: Channel) {
    download_model(&app, ch);
}

#[tauri::command]
fn set_paused(app: AppHandle, on: bool) {
    let st = app.state::<AppState>();
    st.paused.store(on, Ordering::Relaxed);
    let s = st.settings.read().unwrap().clone();
    ui::refresh_tray(&app, &s, on);
}

#[tauri::command]
fn open_url(url: String) {
    let cmd = if cfg!(target_os = "macos") { "open" } else { "explorer" };
    let _ = std::process::Command::new(cmd).arg(url).spawn();
}

#[tauri::command]
fn update_check(app: AppHandle) {
    update::check(&app, true);
}

#[tauri::command]
fn update_install(app: AppHandle) {
    update::install(&app);
}

fn main() {
    let settings = Settings::load();
    let first_run = !settings.onboarded;
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| ui::show_settings(app)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_state, set_settings, model_download, set_paused, open_url, update_check, update_install
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let (tx, rx) = mpsc::unbounded_channel();
            let qwen = Arc::new(QwenEngine::default());
            let remote_url = Arc::new(RwLock::new(None));
            app.manage(AppState {
                settings: Arc::new(RwLock::new(settings.clone())),
                tx: tx.clone(),
                qwen: qwen.clone(),
                paused: AtomicBool::new(false),
                remote: Arc::new(RwLock::new(None)),
                remote_url: remote_url.clone(),
            });

            let handle = app.handle().clone();
            controller::spawn(handle.clone(), qwen, rx);
            remote::spawn(handle.clone());
            ui::create_overlays(&handle)?;
            ui::build_tray(&handle, &settings)?;
            apply(&handle, &settings);
            if first_run {
                ui::show_settings(&handle);
            }
            update::schedule(&handle);
            Ok(())
        })
        .on_window_event(|w, e| {
            if w.label() == "main" && matches!(e, WindowEvent::Destroyed) {
                let _ = w.state::<AppState>().tx.send(Msg::Flush);
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build app")
        .run(|_app, e| match e {
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => ui::show_settings(_app),
            RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
            _ => {}
        });
}
