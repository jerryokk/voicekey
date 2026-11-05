//! 屏幕底部的胶囊浮层：录音、识别中、出错、提示四态；电脑本机说话时还承担多渠道候选。
//!
//! 平时它只是个展示窗口，不接收输入（`set_ignore_cursor_events`），显隐全靠后端推事件。
//! 只有本机说话、多渠道候选可以挑的时候才临时接收点击：用户手里没有手机，候选只能在这里点。
//! 弹出时不抢焦点；点候选会激活它，上屏前先把焦点还给原来的输入框。
//!
//! `HUD_TOKEN` 解决的是「上一条延时隐藏」和「刚又弹出一条」打架的问题：
//! 每次显示都换票号，旧定时器看到票号变了就放弃。

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use voicekey_platform::{self as pf, hotkey::Hotkey};

use crate::settings::Channel;
use crate::state::AppState;

use super::geometry::cursor_area;
use super::windows::{hide_quietly, place_hud, show_quietly, shown, HUD_H, HUD_ROWS_H};

static HUD_TOKEN: AtomicU64 = AtomicU64::new(0);
/// 浮层此刻在展示候选：尺寸是高的那一版
static ROWS: AtomicBool = AtomicBool::new(false);

fn current_channel<R: Runtime>(app: &AppHandle<R>) -> Channel {
    app.state::<AppState>().settings.read().unwrap().channel
}

/// 摆好并显示。只在从隐藏到显示、或换了尺寸时定位：之后用户可能换了显示器，但一句话通常不会跨屏
fn show<R: Runtime>(app: &AppHandle<R>, rows: bool) {
    let Some(w) = app.get_webview_window("hud") else { return };
    let resized = ROWS.swap(rows, Ordering::Relaxed) != rows;
    if resized || !shown(&w) {
        if let Some(a) = cursor_area(app) {
            place_hud(&w, &a, if rows { HUD_ROWS_H } else { HUD_H });
        }
        show_quietly(&w);
    }
}

/// 候选可以点时，用户正在输入的那个窗口。点候选会把浮层激活，上屏前要把焦点还给它
static TARGET: AtomicIsize = AtomicIsize::new(0);

/// 候选可点时接收鼠标，其余时候让点击穿过去。
///
/// 可点的这段时间也允许激活：WebView2 在不可激活的窗口里会把第一下点击吞掉，
/// 用户得点两下才选得中。代价是点完前台变成了浮层，所以先记下原来的前台，见 [`hud_return_focus`]
fn clickable<R: Runtime>(app: &AppHandle<R>, on: bool) {
    if on && !Hotkey::picking() {
        TARGET.store(pf::foreground(), Ordering::Relaxed);
    }
    if let Some(w) = app.get_webview_window("hud") {
        let _ = w.set_ignore_cursor_events(!on);
        let _ = w.set_focusable(on);
    }
    Hotkey::set_picking(on);
}

/// 点了浮层候选：上屏之前把焦点还给用户原来的窗口，字才会打进那里
pub fn hud_return_focus<R: Runtime>(app: &AppHandle<R>) {
    clickable(app, false);
    pf::set_foreground(TARGET.swap(0, Ordering::Relaxed));
}

/// 显示胶囊。state = listen | wait | error | info
pub fn hud<R: Runtime>(app: &AppHandle<R>, state: &str, text: &str, ch: Channel) {
    HUD_TOKEN.fetch_add(1, Ordering::Relaxed);
    if ROWS.load(Ordering::Relaxed) {
        clickable(app, false);
    }
    let _ = app.emit_to("hud", "hud", json!({ "state": state, "text": text, "channel": ch }));
    show(app, false);
}

/// 电脑本机说话时的多渠道候选。`pickable`：已经松手，可以点了
pub fn hud_rows<R: Runtime>(app: &AppHandle<R>, model: Value, pickable: bool) {
    HUD_TOKEN.fetch_add(1, Ordering::Relaxed);
    let state = if pickable { "pick" } else { "listen" };
    let _ = app.emit_to("hud", "hud", json!({ "state": state, "rows": model, "channel": Channel::All }));
    show(app, true);
    clickable(app, pickable);
}

/// 候选里某一行不可选：抖一下
pub fn hud_shake<R: Runtime>(app: &AppHandle<R>, row: usize) {
    let _ = app.emit_to("hud", "shake", row);
}

/// 候选收起了（已上屏、取消、打断）：浮层不再接收点击并收起。
/// 紧接着若有提示（如「已复制到剪贴板」），它换了票号，这次收起自然作废
pub fn hud_rows_closed<R: Runtime>(app: &AppHandle<R>) {
    if ROWS.load(Ordering::Relaxed) {
        clickable(app, false);
        hud_hide(app, 0);
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
                hide_quietly(&w);
            }
            if ROWS.swap(false, Ordering::Relaxed) {
                clickable(&app, false);
            }
        }
    });
}
