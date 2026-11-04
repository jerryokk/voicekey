//! 手机端接入：本地服务、隧道、信令、WebRTC 收流与触摸板。
//!
//! 只做「把手机的音频变成一条 16k 帧流、把触摸与按键变成事件交给上层」，
//! 不认识识别引擎，也不认识 Tauri。
//!
//! 一条链路的形状：
//! ```text
//! 浏览器 --https/ws--> cloudflared --> 127.0.0.1:port --> Hub --> Event
//! 浏览器 --SRTP/SCTP 直连--------------------------> Receiver --┘
//! ```
//!
//! 权限只有一条规则：有人正在说话时，只有说话的那台设备能动鼠标、退格；
//! 候选只能由最后说话的那台设备来选。

pub mod protocol;
mod rtc;
mod tunnel;

pub use protocol::{Button, Pad};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::header,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use protocol::{Op, Out};
use rtc::{Inbox, Receiver};
use serde::Deserialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch, Mutex as AsyncMutex};
use tunnel::Tunnel;
use voicekey_core::Audio;

const PAGE: &str = include_str!("page.html");
/// 手机页扫码重连用的二维码解码库（jsQR 1.4.0，Apache-2.0），连上电脑时就取回来装进 Worker，扫码时不再依赖网络
const JSQR: &str = include_str!("jsqr.min.js");

/// 一条 WS 连接（一台设备）的编号，从 1 开始；0 留给「没有人」
pub type Conn = u64;

/// 手机端事件
#[derive(Debug)]
pub enum Event {
    /// 某台设备按下，本句开始，附带本句的音频帧流
    Press(Audio),
    /// 松手，进入定稿
    Release,
    /// 上滑取消：这一句不要了，丢弃音频且不产出文字
    Cancel,
    /// 说到一半断线：保留已识别的文字
    Abort(String),
    /// 用户在候选里选了一条，参数是渠道 id
    Commit(String),
    /// 退格一个字
    Erase,
    /// 某台设备的触摸板
    Pad(Conn, Pad),
    /// 某台设备的触摸板通道没了（断线或换了一条直连）：它按住没放的鼠标键要替它放开
    Gone(Conn),
}

pub struct Config {
    pub port: u16,
    pub cloudflared: std::path::PathBuf,
}

pub struct Service {
    hub: Arc<Hub>,
    events: mpsc::UnboundedReceiver<Event>,
    /// 承载 HTTP/WS 的服务任务。它握着 listener，不显式 abort 的话
    /// 端口会一直被占着，重新拉起服务时绑不上。
    server: tokio::task::JoinHandle<()>,
    tunnel: Tunnel,
}

impl Drop for Service {
    fn drop(&mut self) {
        self.server.abort();
        // 服务没了，已连上的页面也得断，不然它们会对着一个空 Hub 说话
        self.hub.kick();
    }
}

impl Service {
    /// 拉起本地服务和隧道。隧道起不来就是失败：手机只能经它拿到 https，没有别的路
    pub async fn start(config: Config) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port))
            .await
            .map_err(|e| format!("端口 {} 绑定失败：{e}", config.port))?;
        let tunnel = tunnel::start(&config.cloudflared, &format!("http://127.0.0.1:{}", config.port)).await?;

        let (events_tx, events) = mpsc::unbounded_channel();
        let hub = Arc::new(Hub {
            token: std::sync::RwLock::new(new_token()?),
            origin: tunnel.origin().to_string(),
            events: events_tx,
            out: broadcast::channel(64).0,
            log: broadcast::channel(64).0,
            epoch: watch::channel(0).0,
            devices: watch::channel(0).0,
            hold: AsyncMutex::new(None),
            holding: AtomicU64::new(0),
            owner: AtomicU64::new(0),
        });

        let router = Router::new().route("/", get(page)).route("/jsqr.js", get(jsqr)).route("/ws", get(ws)).with_state(hub.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        Ok(Self { hub, events, server, tunnel })
    }

    /// 公网域名，用于诊断展示（不带 token）
    pub fn origin(&self) -> &str {
        &self.hub.origin
    }

    /// 关掉服务：断开所有手机、停掉隧道，等监听端口真正释放后返回，
    /// 紧接着重新 `start` 才绑得上同一个端口
    pub async fn close(mut self) {
        self.server.abort();
        let _ = (&mut self.server).await;
    }

    /// 下一个手机端事件。Err 表示服务已经不可用（隧道进程退出了），附带原因
    pub async fn next_event(&mut self) -> Result<Event, String> {
        tokio::select! {
            // 发送端在 Hub 里，Hub 活多久它就活多久，这里不会收到 None
            Some(e) = self.events.recv() => Ok(e),
            reason = self.tunnel.exited() => Err(reason),
        }
    }

    /// 在线设备数，变化时通知
    pub fn devices(&self) -> watch::Receiver<usize> {
        self.hub.devices.subscribe()
    }

    /// 运行时错误（解码失败、坏消息等），交给上层落盘。本层不认识日志设施，也不往 stderr 打：GUI 版看不到
    pub fn logs(&self) -> broadcast::Receiver<String> {
        self.hub.log.subscribe()
    }

    /// 可以跨任务克隆的句柄：识别任务要一边跑一边把中间结果推给手机
    pub fn handle(&self) -> Handle {
        Handle { hub: self.hub.clone() }
    }
}

/// 跨任务可克隆的句柄
#[derive(Clone)]
pub struct Handle {
    hub: Arc<Hub>,
}

impl Handle {
    /// 广播给所有已连接设备（识别中间结果、候选行、错误提示都走这里）
    pub fn publish(&self, value: Value) {
        let _ = self.hub.out.send(value.to_string());
    }

    /// 轮换 token 并断开所有在线手机，旧二维码立刻失效
    pub fn kick(&self) -> Result<(), String> {
        *self.hub.token.write().unwrap() = new_token()?;
        self.hub.kick();
        Ok(())
    }

    /// 扫码用的完整地址（随 token 轮换而变）
    pub fn url(&self) -> String {
        format!("{}/?t={}", self.hub.origin, self.hub.token.read().unwrap())
    }
}

struct Hub {
    /// 可变：断开手机时轮换。先换 token 再推 epoch，[`ws`] 依赖这个顺序
    token: std::sync::RwLock<String>,
    origin: String,
    events: mpsc::UnboundedSender<Event>,
    out: broadcast::Sender<String>,
    /// 运行时错误，见 [`Service::logs`]
    log: broadcast::Sender<String>,
    /// 每次踢人加一，所有连接都盯着它
    epoch: watch::Sender<u64>,
    devices: watch::Sender<usize>,
    hold: AsyncMutex<Option<Hold>>,
    /// 正在说话的连接，0 表示没人。触摸板回调是同步的，拿不了异步锁，所以单独放一份
    holding: AtomicU64,
    /// 最后说话的连接：松手后候选还在，只有它能选
    owner: AtomicU64,
}

struct Hold {
    conn: Conn,
    stop: Arc<AtomicBool>,
    pump: tokio::task::JoinHandle<()>,
}

impl Hub {
    fn kick(&self) {
        self.epoch.send_modify(|e| *e += 1);
    }

    /// 没人在说，或者调用者就是说话的人
    fn may_act(&self, conn: Conn) -> bool {
        let holding = self.holding.load(Ordering::Relaxed);
        holding == 0 || holding == conn
    }

    /// 候选归最后说话的设备；它走了就谁都能选
    fn may_commit(&self, conn: Conn) -> bool {
        let owner = self.owner.load(Ordering::Relaxed);
        owner == 0 || owner == conn
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    /// 开始一句话。已经有设备在说就返回 false，由调用方回 busy。
    async fn begin(&self, conn: Conn, inbox: Arc<Inbox>) -> bool {
        let mut hold = self.hold.lock().await;
        if hold.is_some() {
            return false;
        }
        let (tx, rx) = mpsc::unbounded_channel::<Vec<i16>>();
        let stop = Arc::new(AtomicBool::new(false));
        inbox.set_live();
        let pump = tokio::spawn(pump(inbox, tx, stop.clone()));
        *hold = Some(Hold { conn, stop, pump });
        self.holding.store(conn, Ordering::Relaxed);
        self.owner.store(conn, Ordering::Relaxed);
        self.emit(Event::Press(rx));
        true
    }

    /// 持有者结束这一句：停泵、交还 hold，再发出 `then`。调用者不是持有者就什么都不做。
    ///
    /// `then` 必须在锁内发出：锁一放，别的设备就能 `begin`，
    /// 它的 Press 不能抢在这一句的 Release 前面进会话。
    async fn end(&self, conn: Conn, then: Event) {
        let mut hold = self.hold.lock().await;
        let Some(current) = hold.take_if(|h| h.conn == conn) else { return };
        self.holding.store(0, Ordering::Relaxed);
        current.stop.store(true, Ordering::Relaxed);
        // 泵退出时丢弃发送端，引擎随即收到流结束——这一定在 Release 之前
        let _ = current.pump.await;
        self.emit(then);
    }
}

/// 把 Inbox 里的样本按 20ms 切帧送出去。10ms 轮询一次：跟得上实时，又不会空转。
async fn pump(inbox: Arc<Inbox>, tx: mpsc::UnboundedSender<Vec<i16>>, stop: Arc<AtomicBool>) {
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    loop {
        tick.tick().await;
        let end = stop.load(Ordering::Relaxed);
        for frame in inbox.take(end) {
            let _ = tx.send(frame);
        }
        if end {
            return;
        }
    }
}

async fn page() -> Html<&'static str> {
    Html(PAGE)
}

/// 内容随版本固定，浏览器可以一直缓存
async fn jsqr() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")],
        JSQR,
    )
}

#[derive(Deserialize)]
struct WsQuery {
    t: String,
}

/// token 不对也先升级再说 kicked：直接回 403 的话页面只看到断线，会无休止地重连
async fn ws(upgrade: WebSocketUpgrade, Query(q): Query<WsQuery>, State(hub): State<Arc<Hub>>) -> Response {
    // 先订阅再比 token：踢人是「先换 token 再推 epoch」，
    // 比对用的若还是旧 token，那次 epoch 一定在订阅之后，连接会被它踢掉
    let epoch = hub.epoch.subscribe();
    let valid = q.t == *hub.token.read().unwrap();
    upgrade.on_upgrade(move |socket| async move {
        if valid {
            handle(socket, hub, epoch).await;
        } else {
            refuse(socket).await;
        }
    })
}

async fn refuse(mut socket: WebSocket) {
    let _ = socket.send(Message::Text(Out::Kicked.json().to_string().into())).await;
    let _ = socket.close().await;
}

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

async fn handle(socket: WebSocket, hub: Arc<Hub>, mut epoch: watch::Receiver<u64>) {
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    // 写端排空后主动发 Close：被踢的页面要先收到 kicked 再看到断开
    let writer = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                return;
            }
        }
        let _ = sink.close().await;
    });

    // 识别结果等下行消息从广播来，转发到本连接的写端。
    // 落后太多只丢掉中间那段：partial / rows 都是整份快照，下一条会补齐
    let mut feed = hub.out.subscribe();
    let relay = out_tx.clone();
    let relay_task = tokio::spawn(async move {
        loop {
            match feed.recv().await {
                Ok(text) => {
                    if relay.send(text).is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    let say = |value: Value| {
        let _ = out_tx.send(value.to_string());
    };

    say(Out::Hello { ice: &rtc::ICE }.json());
    hub.devices.send_modify(|n| *n += 1);

    let inbox = Arc::new(Inbox::default());
    let mut receiver: Option<Receiver> = None;

    loop {
        let msg = tokio::select! {
            _ = epoch.changed() => {
                say(Out::Kicked.json());
                break;
            }
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(text))) => text,
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => continue,
            },
        };
        let Ok(op) = serde_json::from_str::<Op>(&msg) else {
            let _ = hub.log.send(format!("无法解析的手机消息：{}", msg.as_str()));
            continue;
        };
        match op {
            Op::Offer { id, sdp } => {
                if let Some(old) = receiver.take() {
                    old.close().await;
                    hub.emit(Event::Gone(conn));
                }
                let gate = hub.clone();
                // 松键不设门槛：按下时有资格、松开时别人开始说话了，也得让它把键放开
                let pad: rtc::PadSink = Arc::new(move |p| {
                    if gate.may_act(conn) || matches!(p, Pad::Up { .. }) {
                        gate.emit(Event::Pad(conn, p));
                    }
                });
                let r = match Receiver::new(inbox.clone(), pad, hub.log.clone(), out_tx.downgrade()).await {
                    Ok(r) => r,
                    Err(e) => {
                        say(protocol::error("rtc", e));
                        continue;
                    }
                };
                match r.accept(sdp).await {
                    Ok(answer) => {
                        say(Out::Answer { id, sdp: answer }.json());
                        receiver = Some(r);
                    }
                    Err(e) => {
                        r.close().await;
                        say(protocol::error("answer", e));
                    }
                }
            }
            Op::Start => {
                if receiver.is_none() {
                    say(protocol::error("rtc", "音频通道还没建立"));
                } else if !hub.begin(conn, inbox.clone()).await {
                    say(Out::Busy.json());
                }
            }
            Op::Stop => hub.end(conn, Event::Release).await,
            Op::Cancel => hub.end(conn, Event::Cancel).await,
            Op::Commit { channel } => {
                if hub.may_commit(conn) {
                    hub.emit(Event::Commit(channel));
                }
            }
            Op::Backspace => {
                if hub.may_act(conn) {
                    hub.emit(Event::Erase);
                }
            }
        }
    }

    // 说到一半断线：保留已识别的文字。已经松手的不打断，定稿照常上屏
    hub.end(conn, Event::Abort("连接断开".into())).await;
    let _ = hub.owner.compare_exchange(conn, 0, Ordering::Relaxed, Ordering::Relaxed);
    if let Some(r) = receiver {
        r.close().await;
    }
    hub.emit(Event::Gone(conn));
    hub.devices.send_modify(|n| *n -= 1);
    relay_task.abort();
    drop(out_tx);
    let _ = writer.await;
}

/// 128 位随机 token。二维码就是钥匙，拿到它的人能操作这台电脑的键鼠
fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("系统随机数不可用：{e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_and_distinct() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
