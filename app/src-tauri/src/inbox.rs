//! 手机发到电脑的东西：剪贴板文字直接写进剪贴板，文字等用户复制，文件已经在下载文件夹里。
//! 每一项（一次选的多个文件算一项）在右下角的收件面板里出一张卡片，
//! 同时记进「最近收到」——卡片关掉了，在设置窗的连接页还能找回来。
//!
//! 只在内存里留最近几项：文字可能是密码、验证码，不落盘。

use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
use voicekey_platform as pf;
use voicekey_remote::{Conn, Received, XferItem};

use crate::ui;

/// 「最近收到」留几项
const KEEP: usize = 10;
/// 面板上同时最多几张卡片，更早的收起（仍在「最近收到」里）
const SHOWN: usize = 3;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    /// 手机端生成的缩略图（data URL），只有图片有
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// 用户后来把它删了、挪了
    pub gone: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Body {
    /// 已经写进剪贴板
    Clip { text: String },
    Text { text: String },
    /// `of`：这一组一共几个；`files` 是已经收到的
    Files { files: Vec<File>, of: u32 },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: u64,
    pub at: u64,
    /// 还在面板上
    pub open: bool,
    #[serde(flatten)]
    pub body: Body,
}

struct State {
    seq: u64,
    /// 新的在前
    cards: Vec<Card>,
    /// 正在收的一组文件属于哪张卡片：(手机, 组号) → 卡片 id
    batches: Vec<(Conn, u32, u64)>,
}

static STATE: Mutex<State> = Mutex::new(State { seq: 0, cards: Vec::new(), batches: Vec::new() });

fn emit(app: &AppHandle) {
    let _ = app.emit("inbox", list());
}

/// 当前全部卡片（新的在前），给面板和连接页
pub fn list() -> Vec<Card> {
    let mut st = STATE.lock().unwrap();
    for c in &mut st.cards {
        if let Body::Files { files, .. } = &mut c.body {
            for f in files {
                f.gone = !f.path.exists();
            }
        }
    }
    st.cards.clone()
}

/// 估一个面板高度先摆上去，界面量好真实高度后会再调整
fn estimate(open: &[&Card]) -> f64 {
    let one = |c: &&Card| match &c.body {
        Body::Clip { .. } => 116.0,
        Body::Text { .. } => 196.0,
        Body::Files { files, .. } if files.iter().any(|f| f.thumb.is_some()) && files.len() == 1 => 150.0,
        Body::Files { files, .. } => 106.0 + 26.0 * files.len().min(5) as f64,
    };
    16.0 + open.iter().map(one).sum::<f64>() + 8.0 * open.len().saturating_sub(1) as f64
}

/// 手机发来了一项
pub fn receive(app: &AppHandle, from: Conn, r: Received) {
    let now = voicekey_core::util::now_ms() / 1000;
    let mut st = STATE.lock().unwrap();
    let body = match r.item {
        XferItem::Clip { text } => {
            if let Err(e) = pf::set_clipboard(&text) {
                crate::log_warn!("{e}");
            }
            Some(Body::Clip { text })
        }
        XferItem::Text { text } => Some(Body::Text { text }),
        XferItem::File { path, size, thumb, .. } => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let file = File { name, path, size, thumb, gone: false };
            // 同一组的后续文件并进已有的卡片
            let existing = st.batches.iter().find(|b| b.0 == from && b.1 == r.batch).map(|b| b.2);
            match existing.and_then(|id| st.cards.iter_mut().find(|c| c.id == id)) {
                Some(card) => {
                    if let Body::Files { files, .. } = &mut card.body {
                        files.push(file);
                    }
                    card.open = true;
                    card.at = now;
                    None
                }
                None => Some(Body::Files { files: vec![file], of: r.of }),
            }
        }
    };
    if let Some(body) = body {
        st.seq += 1;
        let id = st.seq;
        if matches!(body, Body::Files { of, .. } if of > 1) {
            st.batches.push((from, r.batch, id));
            if st.batches.len() > KEEP {
                st.batches.remove(0);
            }
        }
        st.cards.insert(0, Card { id, at: now, open: true, body });
        st.cards.truncate(KEEP);
    }
    // 面板上只留最新的几张
    for c in st.cards.iter_mut().skip(SHOWN) {
        c.open = false;
    }
    let open: Vec<&Card> = st.cards.iter().filter(|c| c.open).collect();
    let h = estimate(&open);
    drop(st);
    ui::show_inbox(app, h);
    emit(app);
}

fn with_card<R>(id: u64, f: impl FnOnce(&mut Card) -> R) -> Option<R> {
    STATE.lock().unwrap().cards.iter_mut().find(|c| c.id == id).map(f)
}

/// 收起一张卡片；面板上没有卡片了就隐藏面板
pub fn dismiss(app: &AppHandle, id: u64) {
    with_card(id, |c| c.open = false);
    if !STATE.lock().unwrap().cards.iter().any(|c| c.open) {
        ui::hide_inbox(app);
    }
    emit(app);
}

/// 「复制」：把这张卡片的文字放进剪贴板，然后收起
pub fn copy(app: &AppHandle, id: u64) -> Result<(), String> {
    let text = with_card(id, |c| match &c.body {
        Body::Clip { text } | Body::Text { text } => Some(text.clone()),
        Body::Files { .. } => None,
    })
    .flatten()
    .ok_or("这一项不是文字")?;
    pf::set_clipboard(&text)?;
    dismiss(app, id);
    Ok(())
}

/// 「在文件夹中显示」：资源管理器里选中这组文件，然后收起
pub fn reveal(app: &AppHandle, id: u64) -> Result<(), String> {
    let files = with_card(id, |c| match &c.body {
        Body::Files { files, .. } => files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
        _ => Vec::new(),
    })
    .unwrap_or_default();
    if files.is_empty() {
        return Err("这一项没有文件".into());
    }
    pf::reveal(&files)?;
    dismiss(app, id);
    Ok(())
}
