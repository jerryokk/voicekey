//! 屏幕底部的胶囊浮层：录音、识别中、出错、提示四态。
//!
//! 它只是个展示窗口，不接收输入（`set_ignore_cursor_events`），
//! 所以显隐全靠后端推事件。`HUD_TOKEN` 解决的是「上一条延时隐藏」和
//! 「刚又弹出一条」打架的问题：每次显示都换票号，旧定时器看到票号变了就放弃。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::settings::Channel;
use crate::state::AppState;

use super::geometry::cursor_area;
use super::windows::place_hud;

static HUD_TOKEN: AtomicU64 = AtomicU64::new(0);

fn current_channel<R: Runtime>(app: &AppHandle<R>) -> Channel {
    app.state::<AppState>().settings.read().unwrap().channel
}

/// 显示胶囊。state = listen | wait | error | info
pub fn hud<R: Runtime>(app: &AppHandle<R>, state: &str, text: &str, ch: Channel) {
    HUD_TOKEN.fetch_add(1, Ordering::Relaxed);
    let _ = app.emit_to("hud", "hud", json!({ "state": state, "text": text, "channel": ch }));
    let Some(w) = app.get_webview_window("hud") else { return };
    // 只在从隐藏到显示时定位：之后用户可能换了显示器，但一句话通常不会跨屏
    if !w.is_visible().unwrap_or(false) {
        if let Some(a) = cursor_area(app) {
            place_hud(&w, &a);
        }
        let _ = w.show();
    }
}

/// 出错：红底胶囊，2.5 秒后自动收起
pub fn hud_error<R: Runtime>(app: &AppHandle<R>, msg: &str) {
    hud(app, "error", msg, current_channel(app));
    hud_hide(app, 2500);
}

/// 延时收起。显示时票号已经变了的话，这次收起就作废。
pub fn hud_hide<R: Runtime>(app: &AppHandle<R>, delay_ms: u64) {
    let token = HUD_TOKEN.load(Ordering::Relaxed);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        if HUD_TOKEN.load(Ordering::Relaxed) != token {
            return;
        }
        let _ = app.emit_to("hud", "hud", json!({ "state": "hidden" }));
        // 等淡出动画走完再真正隐藏，否则会硬切
        tokio::time::sleep(Duration::from_millis(160)).await;
        if HUD_TOKEN.load(Ordering::Relaxed) == token {
            if let Some(w) = app.get_webview_window("hud") {
                let _ = w.hide();
            }
        }
    });
}
