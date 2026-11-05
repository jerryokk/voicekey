//! 前端能调用的全部命令，集中一处只做参数校验与转发。
//!
//! 这是前后端唯一的接口面，审阅「前端到底能做什么」只要读这一个文件。

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::inbox;
use crate::session::types::Msg;
use crate::settings::{Channel, Patch, POINTER_SPEED};
use crate::state::{download_model, update_settings, AppState};
use crate::wtoffline;

/// 首屏要的全部状态：一次拿完，避免前端开局发一串请求
#[tauri::command]
pub fn get_state(app: AppHandle) -> serde_json::Value {
    let st = app.state::<AppState>();
    let s = st.settings.read().unwrap().clone();
    let url = st.remote.read().unwrap().as_ref().map(|h| h.url());
    let rs = st.remote_status.read().unwrap().clone();
    let qr = url.as_deref().and_then(qr_svg);
    json!({
        "settings": s,
        "version": app.package_info().version.to_string(),
        "channels": Channel::available(),
        "pointerSpeed": POINTER_SPEED,
        "models": { "wetypeoffline": wtoffline::status() },
        "last": st.last.read().unwrap().clone(),
        "local": st.local_status.read().unwrap().clone(),
        "inbox": inbox::list(),
        "remote": {
            "url": url,
            "qr": qr,
            "phase": rs.phase,
            "error": rs.error,
            "devices": rs.devices,
        },
    })
}

/// 只收改了的字段，见 [`Patch`]
#[tauri::command]
pub fn set_settings(app: AppHandle, patch: Patch) -> Result<(), String> {
    update_settings(&app, |s| patch.apply(s))
}

#[tauri::command]
pub fn model_download(app: AppHandle, ch: Channel) {
    download_model(&app, ch);
}

/// 渠道自检：用内置测试语音真跑一次，识别出至少两个字才算通
#[tauri::command]
pub async fn probe_channel(app: AppHandle, ch: Channel) -> serde_json::Value {
    let engines = crate::session::engines::Engines::new(app.state::<AppState>().qwen.clone());
    let Some(engine) = engines.get(&ch) else {
        return json!({ "ok": false, "error": "未知渠道" });
    };
    match engine.probe().await {
        Ok(p) => {
            let chars = p.text.chars().count();
            let ok = chars >= 2;
            crate::log_info!("自检 {ch:?}：{}ms，{chars} 字", p.ms);
            json!({ "ok": ok, "ms": p.ms, "error": if ok { None } else { Some("没有识别结果") } })
        }
        Err(e) => {
            crate::log_warn!("自检 {ch:?} 失败：{e:#}");
            json!({ "ok": false, "error": format!("{e:#}") })
        }
    }
}

/// 断开手机：轮换 token 并断开所有在线设备，旧二维码立即失效
#[tauri::command]
pub fn disconnect_phone(app: AppHandle) -> Result<(), String> {
    let handle = app.state::<AppState>().remote.read().unwrap().clone().ok_or("手机接入还没就绪")?;
    handle.kick()?;
    // 地址里的 token 换了，二维码要重画
    let _ = app.emit("remote", ());
    Ok(())
}

/// 隧道失败后的重试：整套服务重新拉起
#[tauri::command]
pub fn retry_remote() {
    crate::remote::restart();
}

/// 扫码面板里的「设置」。必须是 async：同步命令跑在主线程上，
/// 在那里建 WebView2 窗口会和窗口创建互相等待而死锁（wry#583），表现为一个卡死的空白窗
#[tauri::command]
pub async fn open_settings(app: AppHandle) {
    crate::ui::show_settings(&app);
}

/// 设置页的麦克风下拉：可用设备与系统默认那一个
#[tauri::command]
pub async fn list_mics() -> Result<serde_json::Value, String> {
    let (mics, default) = tauri::async_runtime::spawn_blocking(voicekey_platform::mic::list)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.message)?;
    let mics: Vec<_> = mics.into_iter().map(|m| json!({ "id": m.id, "name": m.name })).collect();
    Ok(json!({ "mics": mics, "default": default }))
}

/// 设置页的「试听」：电平经 mic-level 事件推回来
#[tauri::command]
pub fn mic_test() {
    crate::local::test();
}

/// 本机麦克风出错后的「重试」：结果经 local 事件推回来
#[tauri::command]
pub fn mic_retry() {
    crate::local::retry();
}

/// Windows 隐私设置里「允许桌面应用访问麦克风」那一页
#[tauri::command]
pub fn open_mic_privacy() -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg("ms-settings:privacy-microphone")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打不开系统设置：{e}"))
}

/// 浮层候选（电脑本机说话 + 多渠道）里点了一行
#[tauri::command]
pub fn hud_choose(app: AppHandle, row: usize) {
    let _ = app.state::<AppState>().tx.send(Msg::Choose(row));
}

#[tauri::command]
pub fn inbox_items() -> Vec<inbox::Card> {
    inbox::list()
}

#[tauri::command]
pub fn inbox_copy(app: AppHandle, id: u64) -> Result<(), String> {
    inbox::copy(&app, id)
}

#[tauri::command]
pub fn inbox_reveal(app: AppHandle, id: u64) -> Result<(), String> {
    inbox::reveal(&app, id)
}

#[tauri::command]
pub fn inbox_dismiss(app: AppHandle, id: u64) {
    inbox::dismiss(&app, id);
}

/// 收件面板量好了自己的内容高度：贴着右下角重摆
#[tauri::command]
pub fn inbox_resize(app: AppHandle, height: f64) {
    if inbox::list().iter().any(|c| c.open) {
        crate::ui::show_inbox(&app, height);
    }
}

/// 二维码用 SVG 直接给前端：省掉一个前端依赖，也不怕离线加载不到库
fn qr_svg(url: &str) -> Option<String> {
    let code = qrcode::QrCode::new(url.as_bytes()).ok()?;
    Some(code.render::<qrcode::render::svg::Color>().min_dimensions(200, 200).quiet_zone(false).build())
}
