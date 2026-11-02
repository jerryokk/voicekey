//! 手机端接入：本地服务、隧道、信令、WebRTC 收流。
//!
//! 只做「把手机的音频变成一条 16k 帧流，并把事件交给上层」，不认识识别引擎，
//! 也不认识 Tauri。这样它可以被单独测试，桌面壳只负责把事件接进会话状态机。
//!
//! 一条链路的形状：
//! ```text
//! 浏览器 --https/ws--> cloudflared --> 127.0.0.1:port --> Hub --> Event / Audio
//! 浏览器 --SRTP/UDP 直连----------------------------> Receiver --┘
//! ```

mod rtc;
mod tunnel;

pub use rtc::{Inbox, Receiver, Stats};
pub use tunnel::Tunnel;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};
use voicekey_core::Audio;

const PAGE: &str = include_str!("page.html");

/// 手机端事件。音频不在事件里，按 Press 之后用 `Service::take_audio` 取走本句的帧流。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 某台设备按下，本句开始
    Press,
    /// 松手，进入定稿
    Release,
    /// 断线或出错：保留已识别的文字
    Abort(String),
    /// 用户在候选里选了一条，参数是渠道 id
    Commit(String),
    /// 手机端请求往已上屏的文字里退格，参数是删除的字符数
    Erase(u32),
}

pub struct Config {
    pub port: u16,
    /// 显式指定 cloudflared；None 则按程序目录与 PATH 查找
    pub cloudflared: Option<PathBuf>,
    /// 是否开启公网隧道
    pub tunnel: bool,
}

pub struct Service {
    hub: Arc<Hub>,
    events: mpsc::UnboundedReceiver<Event>,
    origin: String,
    _tunnel: Option<Tunnel>,
}

impl Service {
    pub async fn start(config: Config) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port))
            .await
            .map_err(|e| format!("端口 {} 绑定失败：{e}", config.port))?;

        let token = new_token();
        let (events_tx, events) = mpsc::unbounded_channel();
        let (out, _) = broadcast::channel(64);
        let hub = Arc::new(Hub {
            token: std::sync::RwLock::new(token),
            events: events_tx,
            out,
            inbox: Arc::new(Inbox::default()),
            hold: AsyncMutex::new(None),
            audio: AsyncMutex::new(None),
        });

        let local = format!("http://127.0.0.1:{}", config.port);
        let mut origin = local.clone();
        let mut tunnel = None;
        if config.tunnel {
            match tunnel::find(config.cloudflared) {
                Some(exe) => match tunnel::start(&exe, &local).await {
                    Ok(t) => {
                        origin = t.origin().to_string();
                        tunnel = Some(t);
                    }
                    Err(e) => eprintln!("隧道不可用，仅限局域网访问：{e}"),
                },
                None => eprintln!("找不到 cloudflared，仅限局域网访问"),
            }
        }

        let router = Router::new()
            .route("/", get(page))
            .route("/ws", get(ws))
            .with_state(hub.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        Ok(Self { hub, events, origin, _tunnel: tunnel })
    }

    /// 扫码用的完整地址
    pub fn url(&self) -> String {
        format!("{}/?t={}", self.origin, self.hub.token.read().unwrap().clone())
    }

    /// 断开所有手机：轮换 token（旧二维码立即失效）并通知在线页面退出。
    /// 返回刷新后的扫码地址。
    pub fn disconnect(&self) -> String {
        self.hub.kick();
        self.url()
    }

    /// 公网域名（隧道不可用时为本地地址），用于诊断展示
    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }

    /// 取走本句的音频帧流。必须在收到 `Event::Press` 之后立即调用，否则会取到上一句的。
    pub async fn take_audio(&self) -> Option<Audio> {
        self.hub.audio.lock().await.take()
    }

    /// 广播给所有已连接设备（识别中间结果、候选行、错误提示都走这里）
    pub fn publish(&self, value: Value) {
        let _ = self.hub.out.send(value.to_string());
    }

    /// 可以跨任务克隆的下行句柄：识别任务要一边跑一边把中间结果推给手机
    pub fn handle(&self) -> Handle {
        Handle(self.hub.clone())
    }
}

/// 跨任务可克隆的下行句柄
#[derive(Clone)]
pub struct Handle(Arc<Hub>);

impl Handle {
    pub fn publish(&self, value: Value) {
        let _ = self.0.out.send(value.to_string());
    }

    /// 轮换 token 并踢掉在线手机，桌面端用它实现「断开手机」
    pub fn kick(&self) {
        self.0.kick();
    }

    /// 当前 token（断开后重新拼二维码用）
    pub fn token(&self) -> String {
        self.0.token.read().unwrap().clone()
    }
}

struct Hub {
    /// 可变：断开手机时轮换，旧二维码立刻失效
    token: std::sync::RwLock<String>,
    events: mpsc::UnboundedSender<Event>,
    out: broadcast::Sender<String>,
    inbox: Arc<Inbox>,
    hold: AsyncMutex<Option<Hold>>,
    audio: AsyncMutex<Option<Audio>>,
}

struct Hold {
    conn: u64,
    stop: Arc<AtomicBool>,
    pump: tokio::task::JoinHandle<()>,
}

impl Hub {
    /// 轮换 token 并让在线页面断开
    fn kick(&self) {
        *self.token.write().unwrap() = new_token();
        let _ = self.out.send(json!({ "type": "kicked" }).to_string());
    }

    fn holder(&self) -> Option<u64> {
        self.hold.try_lock().ok().and_then(|g| g.as_ref().map(|h| h.conn))
    }

    /// 开始一句话。已经有设备在说就返回 false，由调用方回 busy。
    async fn begin(self: &Arc<Self>, conn: u64) -> bool {
        let mut hold = self.hold.lock().await;
        if hold.is_some() {
            return false;
        }
        self.inbox.clear();
        let (tx, rx) = mpsc::unbounded_channel::<Vec<i16>>();
        *self.audio.lock().await = Some(rx);
        let stop = Arc::new(AtomicBool::new(false));
        let pump = tokio::spawn(pump(self.inbox.clone(), tx, stop.clone()));
        *hold = Some(Hold { conn, stop, pump });
        let _ = self.events.send(Event::Press);
        true
    }

    /// 结束一句话。只有持有者能结束，返回是否真的结束了。
    async fn finish(&self, conn: u64, error: Option<String>) -> bool {
        let mut hold = self.hold.lock().await;
        let Some(current) = hold.as_ref() else { return false };
        if current.conn != conn {
            return false;
        }
        let Some(current) = hold.take() else { return false };
        drop(hold);
        current.stop.store(true, Ordering::Relaxed);
        // 泵退出时丢弃发送端，引擎随即收到流结束
        let _ = current.pump.await;
        let _ = self.events.send(match error {
            Some(reason) => Event::Abort(reason),
            None => Event::Release,
        });
        true
    }
}

/// 把 Inbox 里的样本按 20ms 切帧送出去。10ms 轮询一次：跟得上实时，又不会空转。
async fn pump(inbox: Arc<Inbox>, tx: mpsc::UnboundedSender<Vec<i16>>, stop: Arc<AtomicBool>) {
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    loop {
        tick.tick().await;
        for frame in inbox.frames() {
            if tx.send(frame).is_err() {
                return;
            }
        }
        if stop.load(Ordering::Relaxed) {
            if let Some(tail) = inbox.flush() {
                let _ = tx.send(tail);
            }
            return;
        }
    }
}

async fn page() -> Html<&'static str> {
    Html(PAGE)
}

#[derive(Deserialize)]
struct WsQuery {
    t: Option<String>,
}

async fn ws(upgrade: WebSocketUpgrade, Query(q): Query<WsQuery>, State(hub): State<Arc<Hub>>) -> Response {
    let expected = hub.token.read().unwrap().clone();
    if q.t.as_deref() != Some(expected.as_str()) {
        return (StatusCode::FORBIDDEN, "token 不正确").into_response();
    }
    upgrade.on_upgrade(move |socket| handle(socket, hub))
}

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

async fn handle(socket: WebSocket, hub: Arc<Hub>) {
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    let writer = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // 识别结果等下行消息从广播来，转发到本连接的写端
    let mut feed = hub.out.subscribe();
    let relay = out_tx.clone();
    let relay_task = tokio::spawn(async move {
        while let Ok(text) = feed.recv().await {
            if relay.send(text).is_err() {
                break;
            }
        }
    });

    let say = |value: Value| {
        let _ = out_tx.send(value.to_string());
    };

    say(json!({ "type": "hello", "ice": rtc::ICE }));

    let mut receiver: Option<Receiver> = None;
    let inbox = hub.inbox.clone();

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Binary(chunk) => {
                // 只有持有会话的设备，音频才算进这一句
                if hub.holder() == Some(conn) {
                    let samples: Vec<i16> = chunk.chunks_exact(2).map(|p| i16::from_le_bytes([p[0], p[1]])).collect();
                    inbox.push(&samples);
                }
            }
            Message::Text(text) => {
                let Ok(msg) = serde_json::from_str::<Value>(&text) else { continue };
                match msg.get("op").and_then(|o| o.as_str()).unwrap_or_default() {
                    "offer" => {
                        let sdp = msg.get("sdp").and_then(|s| s.as_str()).unwrap_or_default().to_string();
                        let r = match Receiver::new(inbox.clone()).await {
                            Ok(r) => r,
                            Err(e) => {
                                say(json!({ "type": "error", "stage": "rtc", "message": e }));
                                continue;
                            }
                        };
                        match r.accept(sdp).await {
                            Ok(answer) => {
                                say(json!({ "type": "answer", "sdp": answer }));
                                receiver = Some(r);
                            }
                            Err(e) => say(json!({ "type": "error", "stage": "answer", "message": e })),
                        }
                    }
                    "start" => {
                        if !hub.begin(conn).await {
                            say(json!({ "type": "busy" }));
                            continue;
                        }
                        if let Some(r) = &receiver {
                            r.reset();
                        }
                        say(json!({ "type": "started" }));
                    }
                    "stop" => {
                        if let Some(r) = &receiver {
                            let stats = r.snapshot();
                            say(json!({
                                "type": "stats",
                                "packets": stats.packets,
                                "seconds": stats.seconds(),
                                "loss": stats.loss_percent(),
                                "state": format!("{:?}", r.state()),
                            }));
                            // Opus 是预测编码，丢包会污染解码器状态且很难从文字上看出来。
                            // 超过阈值就让页面下次改走回落通道——宁可多花带宽，不要悄悄变差。
                            let loss = stats.loss_percent();
                            if stats.packets > 0 && loss > 5.0 {
                                say(json!({
                                    "type": "prefer",
                                    "transport": "pcm",
                                    "reason": format!("丢包 {loss:.1}%"),
                                }));
                            }
                        }
                        hub.finish(conn, None).await;
                    }
                    // 候选由手机上的点击决定：只有持有本句的设备能提交
                    "commit" => {
                        let channel = msg.get("channel").and_then(|c| c.as_str()).unwrap_or_default().to_string();
                        if hub.holder() == Some(conn) || hub.holder().is_none() {
                            let _ = hub.events.send(Event::Commit(channel));
                        }
                    }
                    // 退格：删除已打到前台窗口里的字。上限拦住误触发的批量请求
                    "backspace" => {
                        let n = msg.get("n").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 50) as u32;
                        let _ = hub.events.send(Event::Erase(n));
                    }
                    _ => {}
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // 说到一半断线：保留已识别的文字
    hub.finish(conn, Some("连接断开".into())).await;
    if let Some(r) = receiver {
        r.close().await;
    }
    relay_task.abort();
    drop(out_tx);
    let _ = writer.await;
}

fn new_token() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    format!("{:016x}", (nanos as u64) << 32 | std::process::id() as u64)
}
