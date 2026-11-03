//! 应用级状态与它的读写路径。
//!
//! 所有「改设置」的入口都收在 `update_settings` 一处：写盘、刷新托盘、应用自启动、
//! 触发模型下载、广播给前端，顺序固定，避免某个入口漏掉其中一步。

use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;
use tokio::sync::mpsc::UnboundedSender;
use voicekey_core::QwenEngine;
use voicekey_remote::Handle as RemoteHandle;

use crate::session::types::Msg;
use crate::settings::{Channel, Settings};
use crate::{ui, wtoffline};

/// 手机接入的当前状态，喂给「连接」页展示
#[derive(Clone)]
pub struct RemoteStatus {
    /// starting | ready | failed
    pub state: &'static str,
    pub error: Option<String>,
    /// 隧道没起来，只有本机地址可用——手机连不上，要在页面上讲清楚
    pub lan_only: bool,
}

pub struct AppState {
    pub settings: Arc<RwLock<Settings>>,
    pub tx: UnboundedSender<Msg>,
    pub qwen: Arc<QwenEngine>,
    /// 最近一次成功上屏的结果（连接页展示）
    pub last: Arc<RwLock<Option<serde_json::Value>>>,
    /// 隧道 origin（不带 token），断开后可重新拼二维码
    pub remote_origin: Arc<RwLock<Option<String>>>,
    /// 手机接入服务起来之后才有值
    pub remote: Arc<RwLock<Option<RemoteHandle>>>,
    pub remote_url: Arc<RwLock<Option<String>>>,
    pub remote_status: Arc<RwLock<RemoteStatus>>,
}

/// 把这些状态装进 Tauri 的全局表
pub fn manage<R: Runtime>(app: &AppHandle<R>, settings: Settings, tx: UnboundedSender<Msg>, qwen: Arc<QwenEngine>) {
    app.manage(AppState {
        settings: Arc::new(RwLock::new(settings)),
        tx,
        qwen,
        last: Arc::new(RwLock::new(None)),
        remote_origin: Arc::new(RwLock::new(None)),
        remote: Arc::new(RwLock::new(None)),
        remote_url: Arc::new(RwLock::new(None)),
        remote_status: Arc::new(RwLock::new(RemoteStatus { state: "starting", error: None, lan_only: false })),
    });
}

/// 改设置的唯一入口：先落盘，再让改动生效，最后广播
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

/// 让设置生效：托盘菜单、开机自启动。启动时与改动后都要走一遍
pub fn apply<R: Runtime>(app: &AppHandle<R>, s: &Settings) {
    ui::refresh_tray(app, s);
    let al = app.autolaunch();
    if al.is_enabled().unwrap_or(false) != s.autostart {
        let _ = if s.autostart { al.enable() } else { al.disable() };
    }
}

/// 本地模型渠道：开始下载（已下载或下载中则忽略）
pub fn download_model<R: Runtime>(app: &AppHandle<R>, ch: Channel) {
    if ch == Channel::WetypeOffline {
        wtoffline::download(app);
    }
}

/// 这个渠道此刻能不能用（离线模型没下载就不能）
pub fn model_ready(ch: Channel) -> bool {
    match ch {
        Channel::WetypeOffline => wtoffline::installed(),
        _ => true,
    }
}
