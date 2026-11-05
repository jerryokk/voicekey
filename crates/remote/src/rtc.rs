//! WebRTC 接收端：浏览器 offer → 本机 answer → 收 RTP(Opus) 解成 16k PCM，
//! 收 DataChannel `input`（触摸板）与 `xfer`（发到电脑）。
//!
//! 每台设备一条 PeerConnection、一个 [`Inbox`]。音频只进自己的 Inbox，
//! 由 Hub 决定哪个 Inbox 被接进本句——别的设备的声音碰不到正在说的这一句。

use crate::protocol::{self, Pad, PAD_CHANNEL, XFER_CHANNEL};
use crate::xfer::{self, Received};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use webrtc::data_channel::RTCDataChannel;
use tokio::sync::{broadcast, mpsc};
use voicekey_core::audio::Inbox;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

/// 国内优先：本机实测 bilibili 37ms、Cloudflare 167ms。候选收集是并行的，
/// 但先到的候选先进 SDP，弱网下这点差别能感觉到。
pub const ICE: [&str; 2] = ["stun:stun.chat.bilibili.com:3478", "stun:stun.cloudflare.com:3478"];

/// 一个 Opus 包最长 120ms，解到 16kHz 是 1920 个样本
const MAX_PACKET_SAMPLES: usize = 16_000 * 120 / 1000;

/// 收集本机候选的上限。STUN 不通时 webrtc-rs 要等它自己的超时，这里不陪它等
const GATHER_TIMEOUT: Duration = Duration::from_secs(5);

/// 触摸板事件的去处，由 Hub 决定这台设备此刻有没有资格动鼠标
pub type PadSink = Arc<dyn Fn(Pad) + Send + Sync>;

pub struct Receiver {
    connection: Arc<RTCPeerConnection>,
}

impl Receiver {
    /// `log` 是交给上层落盘的运行时错误，`say` 是这台设备的 WS 下行。
    /// 后者只拿弱引用：回调挂在 PeerConnection 上，强引用会让连接断开时写端等不到通道关闭
    pub async fn new(
        inbox: Arc<Inbox>,
        pad: PadSink,
        xfer: XferSink,
        log: broadcast::Sender<String>,
        say: mpsc::WeakUnboundedSender<String>,
    ) -> Result<Self, String> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs().map_err(|e| format!("codecs: {e}"))?;
        let registry = register_default_interceptors(Registry::new(), &mut media_engine)
            .map_err(|e| format!("interceptors: {e}"))?;
        let api = APIBuilder::new().with_media_engine(media_engine).with_interceptor_registry(registry).build();

        let connection = Arc::new(
            api.new_peer_connection(RTCConfiguration {
                ice_servers: ICE.iter().map(|u| RTCIceServer { urls: vec![u.to_string()], ..Default::default() }).collect(),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("peer connection: {e}"))?,
        );

        let track_log = log.clone();
        connection.on_track(Box::new(move |track, _receiver, _transceiver| {
            let inbox = inbox.clone();
            let log = track_log.clone();
            let say = say.clone();
            Box::pin(async move {
                // 每个 track 一个解码器（Opus 有状态）。手机按住时 replaceTrack 换麦克风不产生新 track，各句共用它
                let mut decoder = match Decoder::new() {
                    Ok(d) => d,
                    Err(e) => {
                        // 这条音轨从此没有声音：记下来，也让手机知道，不然只会觉得「说了没反应」
                        let _ = log.send(e.clone());
                        if let Some(say) = say.upgrade() {
                            let _ = say.send(protocol::error("rtc", e).to_string());
                        }
                        return;
                    }
                };
                // 坏包只丢这一个：Opus 下一个包自带同步，不必断开整条音轨
                while let Ok((packet, _)) = track.read_rtp().await {
                    match decoder.decode(&packet.payload) {
                        Ok(samples) => inbox.push(samples),
                        Err(e) => {
                            let _ = log.send(e);
                        }
                    }
                }
            })
        }));

        connection.on_data_channel(Box::new(move |channel| {
            let pad = pad.clone();
            let log = log.clone();
            let xfer = xfer.clone();
            Box::pin(async move {
                if channel.label() == XFER_CHANNEL {
                    return accept_xfer(channel, xfer);
                }
                if channel.label() != PAD_CHANNEL {
                    let _ = channel.close().await;
                    return;
                }
                channel.on_message(Box::new(move |msg| {
                    match serde_json::from_slice::<Pad>(&msg.data) {
                        Ok(p) if p.sane() => pad(p),
                        _ => {
                            let _ = log.send(format!("无法解析的触摸板消息：{}", String::from_utf8_lossy(&msg.data)));
                        }
                    }
                    Box::pin(async {})
                }));
            })
        }));

        Ok(Self { connection })
    }

    /// 应用浏览器 offer，等到候选收集完成再返回 answer（不做 trickle，少一个状态机）
    pub async fn accept(&self, sdp: String) -> Result<String, String> {
        let offer = RTCSessionDescription::offer(sdp).map_err(|e| format!("offer: {e}"))?;
        self.connection.set_remote_description(offer).await.map_err(|e| format!("remote offer: {e}"))?;
        let answer = self.connection.create_answer(None).await.map_err(|e| format!("create answer: {e}"))?;
        let mut gathered = self.connection.gathering_complete_promise().await;
        self.connection.set_local_description(answer).await.map_err(|e| format!("local answer: {e}"))?;
        // 超时就用已经收到的候选：主机候选总是先到，同一局域网下够用
        let _ = tokio::time::timeout(GATHER_TIMEOUT, gathered.recv()).await;
        let local = self.connection.local_description().await.ok_or("没有本地 answer")?;
        Ok(local.sdp)
    }

    pub async fn close(&self) {
        let _ = self.connection.close().await;
    }
}

/// 收好的一项交给谁，以及存到哪
#[derive(Clone)]
pub struct XferSink {
    pub dir: PathBuf,
    pub done: Arc<dyn Fn(Received) + Send + Sync>,
}

/// 「发到电脑」通道。消息回调是逐条等完再收下一条的，写盘就在回调里同步做，顺序天然有保证。
/// 通道关了（断线、页面重建直连）时正在收的那一项随接收端一起丢掉，半个文件不留
fn accept_xfer(channel: Arc<RTCDataChannel>, sink: XferSink) {
    let rx = Arc::new(StdMutex::new(xfer::Receiver::new(sink.dir.clone())));
    let on_close = rx.clone();
    channel.on_close(Box::new(move || {
        on_close.lock().unwrap().discard();
        Box::pin(async {})
    }));
    let reply_to = Arc::downgrade(&channel);
    channel.on_message(Box::new(move |msg| {
        let step = {
            let mut r = rx.lock().unwrap();
            if msg.is_string {
                r.on_text(&String::from_utf8_lossy(&msg.data))
            } else {
                r.on_binary(&msg.data)
            }
        };
        if let Some(done) = step.done {
            (sink.done)(done);
        }
        let reply_to = reply_to.clone();
        Box::pin(async move {
            if let (Some(text), Some(ch)) = (step.reply, reply_to.upgrade()) {
                let _ = ch.send_text(text).await;
            }
        })
    }));
}

/// Opus 解码器：浏览器发的是 48kHz，直接解到 16kHz 单声道——
/// 省掉一次重采样，且 20ms 正好 320 样本，正好是 Engine 要的帧长。
struct Decoder {
    dec: *mut opus_sys::OpusDecoder,
    buf: Vec<i16>,
}

unsafe impl Send for Decoder {}

impl Decoder {
    fn new() -> Result<Self, String> {
        let mut err = 0;
        let dec = unsafe { opus_sys::opus_decoder_create(16_000, 1, &mut err) };
        if dec.is_null() || err != 0 {
            return Err(format!("Opus 解码器创建失败：{err}"));
        }
        Ok(Self { dec, buf: vec![0; MAX_PACKET_SAMPLES] })
    }

    fn decode(&mut self, payload: &[u8]) -> Result<&[i16], String> {
        let n = unsafe {
            opus_sys::opus_decode(
                self.dec,
                payload.as_ptr(),
                payload.len() as i32,
                self.buf.as_mut_ptr(),
                self.buf.len() as i32,
                0,
            )
        };
        if n < 0 {
            return Err(format!("Opus 解码失败 {n}"));
        }
        Ok(&self.buf[..n as usize])
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { opus_sys::opus_decoder_destroy(self.dec) }
    }
}
