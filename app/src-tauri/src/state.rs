//! 应用级状态与它的读写路径。
//!
//! 所有「改设置」的入口都收在 `update_settings` 一处：应用自启动、写盘、
//! 触发模型下载、广播给前端，顺序固定，避免某个入口漏掉其中一步。

use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;
use tokio::sync::mpsc::UnboundedSender;
use voicekey_core::QwenEngine;
use voicekey_remote::Handle as RemoteHandle;

use crate::session::types::Msg;
use crate::settings::{Channel, Settings};
use crate::wtoffline;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Starting,
    Ready,
    Failed,
}

/// 手机接入的当前状态，喂给连接页与扫码面板
#[derive(Clone)]
pub struct RemoteStatus {
    pub phase: Phase,
    /// 仅 Failed 时有值
    pub error: Option<String>,
    /// 在线设备数
    pub devices: usize,
}

impl RemoteStatus {
    pub const STARTING: Self = Self { phase: Phase::Starting, error: None, devices: 0 };
}

pub struct AppState {
    pub settings: Arc<RwLock<Settings>>,
    pub tx: UnboundedSender<Msg>,
    pub qwen: Arc<QwenEngine>,
    /// 最近一次成功上屏的结果（连接页展示）
    pub last: Arc<RwLock<Option<serde_json::Value>>>,
    /// 手机接入服务起来之后才有值；扫码地址也从它拿（token 轮换后随之变化）
    pub remote: Arc<RwLock<Option<RemoteHandle>>>,
    pub remote_status: Arc<RwLock<RemoteStatus>>,
}

/// 把这些状态装进 Tauri 的全局表
pub fn manage<R: Runtime>(app: &AppHandle<R>, settings: Settings, tx: UnboundedSender<Msg>, qwen: Arc<QwenEngine>) {
    app.manage(AppState {
        settings: Arc::new(RwLock::new(settings)),
        tx,
        qwen,
        last: Arc::new(RwLock::new(None)),
        remote: Arc::new(RwLock::new(None)),
        remote_status: Arc::new(RwLock::new(RemoteStatus::STARTING)),
    });
}

/// 改设置的串行锁：并发的两次修改（界面改设置、会话记上屏历史）按改内存的先后
/// 落盘和广播，后改的不会被先改的旧快照盖掉。另用一把锁而不是一直持着设置的写锁：
/// 落盘、设自启动慢时，会话读设置（触摸板每次位移都读）不被卡住
static UPDATE: Mutex<()> = Mutex::new(());

/// 改设置的唯一入口：先改内存，再让改动生效、落盘、广播。
/// 自启动设不上就把这一项退回原值，界面看到的是真实状态；
/// 落盘失败则照常生效（本次运行里用户看到的就是改过的）。两种原因都交给调用方
pub fn update_settings<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let _order = UPDATE.lock().unwrap();
    let st = app.state::<AppState>();
    let (before, mut s) = {
        let mut g = st.settings.write().unwrap();
        let before = g.clone();
        f(&mut g);
        g.sanitize();
        (before, g.clone())
    };
    let applied = apply(app, &s);
    if applied.is_err() && s.autostart != before.autostart {
        s.autostart = before.autostart;
        st.settings.write().unwrap().autostart = before.autostart;
    }
    let saved = s.save();
    // 只在切到离线渠道的那一刻触发下载：别的设置改动不该把失败过的下载又拉起来
    if s.channel != before.channel {
        download_model(app, s.channel);
    }
    let _ = app.emit("settings", &s);
    applied.and(saved)
}

/// 让设置生效：开机自启动。启动时与改动后都要走一遍
pub fn apply<R: Runtime>(app: &AppHandle<R>, s: &Settings) -> Result<(), String> {
    let al = app.autolaunch();
    let enabled = al.is_enabled().map_err(|e| format!("读取开机自启动状态失败：{e}"))?;
    if enabled == s.autostart {
        return Ok(());
    }
    if s.autostart { al.enable() } else { al.disable() }.map_err(|e| format!("设置开机自启动失败：{e}"))
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
