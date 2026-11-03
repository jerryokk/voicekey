//! 前端能调用的全部命令，集中一处只做参数校验与转发。
//!
//! 放在一起的好处很直接：这是前后端唯一的接口面，审阅「前端到底能做什么」
//! 只要读这一个文件，不用在整个 `main.rs` 里翻。

use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::settings::{Channel, Settings};
use crate::state::{download_model, update_settings, AppState};
use crate::wtoffline;

/// 首屏要的全部状态：一次拿完，避免前端开局发一串请求
#[tauri::command]
pub fn get_state(app: AppHandle) -> serde_json::Value {
    let st = app.state::<AppState>();
    let s = st.settings.read().unwrap().clone();
    let url = st.remote_url.read().unwrap().clone();
    let rs = st.remote_status.read().unwrap().clone();
    // 二维码只对公网 https 地址生成。本机 127.0.0.1 的码手机扫了必然连不上，
    // 给出来只会误导——托盘弹出的扫码窗口里尤其明显（用户以为能连，其实不能）。
    let qr = url.as_deref().filter(|u| u.starts_with("https://")).and_then(qr_svg);
    json!({
        "settings": s,
        "platform": "win",
        "version": app.package_info().version.to_string(),
        "channels": Channel::available(),
        "models": { "wetypeoffline": wtoffline::status() },
        "last": st.last.read().unwrap().clone(),
        "remote": {
            "url": url,
            "qr": qr,
            "state": rs.state,
            "error": rs.error,
            "lanOnly": rs.lan_only,
        },
    })
}

#[tauri::command]
pub fn set_settings(app: AppHandle, settings: Settings) {
    update_settings(&app, |s| *s = settings);
}

#[tauri::command]
pub fn model_download(app: AppHandle, ch: Channel) {
    download_model(&app, ch);
}

/// 渠道自检：用内置测试语音真跑一次，能拿到非空文字就算通
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
            json!({ "ok": ok, "ms": p.ms, "text": p.text, "error": if ok { None } else { Some("没有识别结果") } })
        }
        Err(e) => {
            crate::log_warn!("自检 {ch:?} 失败：{e:#}");
            json!({ "ok": false, "error": format!("{e:#}") })
        }
    }
}

/// 断开手机：轮换 token，旧二维码立即失效；返回新的扫码地址
#[tauri::command]
pub fn disconnect_phone(app: AppHandle) -> Option<String> {
    let st = app.state::<AppState>();
    let handle = st.remote.read().unwrap().clone()?;
    handle.kick();
    let origin = st.remote_origin.read().unwrap().clone()?;
    let url = format!("{origin}/?t={}", handle.token());
    *st.remote_url.write().unwrap() = Some(url.clone());
    Some(url)
}

/// 二维码用 SVG 直接给前端：省掉一个前端依赖，也不怕离线加载不到库
fn qr_svg(url: &str) -> Option<String> {
    let code = qrcode::QrCode::new(url.as_bytes()).ok()?;
    Some(code.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).quiet_zone(true).build())
}
