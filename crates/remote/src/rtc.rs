//! WebRTC 接收端：浏览器 offer → 本机 answer → 收 RTP(Opus) 解成 16k PCM，收 DataChannel 触摸板。
//!
//! 每台设备一条 PeerConnection、一个 [`Inbox`]。音频只进自己的 Inbox，
//! 由 Hub 决定哪个 Inbox 被接进本句——别的设备的声音碰不到正在说的这一句。

use crate::protocol::{self, Pad, PAD_CHANNEL};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use voicekey_core::audio::FRAME;
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

/// 没在说话时保留的最近音频：16kHz × 500ms。
/// RTP 直连，`start` 绕隧道，后者常常晚到几百毫秒；没有这段，第一个字会被吃掉。
const PREROLL: usize = 8_000;

/// 一个 Opus 包最长 120ms，解到 16kHz 是 1920 个样本
const MAX_PACKET_SAMPLES: usize = 16_000 * 120 / 1000;

/// 收集本机候选的上限。STUN 不通时 webrtc-rs 要等它自己的超时，这里不陪它等
const GATHER_TIMEOUT: Duration = Duration::from_secs(5);

/// 一台设备的音频入口
#[derive(Default)]
pub struct Inbox {
    /// 样本与「正被接进本句」放在同一把锁里：收尾时取残余和退出 live 必须一步完成，
    /// 中间插进来的样本要么属于这一句，要么属于下一句的预录，不能两头都不算
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    pcm: Vec<i16>,
    /// 不在说话时只留最近 [`PREROLL`] 个样本
    live: bool,
}

impl Inbox {
    pub fn push(&self, samples: &[i16]) {
        let mut s = self.state.lock().unwrap();
        s.pcm.extend_from_slice(samples);
        if !s.live && s.pcm.len() > PREROLL {
            let drop = s.pcm.len() - PREROLL;
            s.pcm.drain(..drop);
        }
    }

    /// 接进本句：从此不再裁剪，预录的那段成为这一句的开头
    pub fn set_live(&self) {
        self.state.lock().unwrap().live = true;
    }

    /// 取走所有整帧，不足一帧的余量留在原地等下一批。
    /// `end` 时连余量也补零成一帧取走（否则最后一个字会被截掉），并退出 live
    pub fn take(&self, end: bool) -> Vec<Vec<i16>> {
        let mut s = self.state.lock().unwrap();
        let len = s.pcm.len();
        let whole = if end { len.div_ceil(FRAME) * FRAME } else { len / FRAME * FRAME };
        s.pcm.resize(len.max(whole), 0);
        let frames = s.pcm.drain(..whole).collect::<Vec<_>>().chunks(FRAME).map(<[i16]>::to_vec).collect();
        if end {
            s.live = false;
        }
        frames
    }
}

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
                // 每个 track 一个解码器：Opus 是状态化的，跨 track 复用会带进前一句的预测状态
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
            Box::pin(async move {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_yields_whole_frames_and_pads_the_tail() {
        let inbox = Inbox::default();
        inbox.set_live();
        inbox.push(&[7i16; 700]);
        let frames = inbox.take(false);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len(), FRAME);
        // 700 - 640 = 60 个样本留在原地，收尾时补零成一帧
        let tail = inbox.take(true);
        assert_eq!(tail.len(), 1);
        assert_eq!(&tail[0][..60], &[7i16; 60]);
        assert!(tail[0][60..].iter().all(|&s| s == 0));
        assert!(inbox.take(true).is_empty());
    }

    /// 没在说话时只留最近半秒：既不吃掉第一个字，也不把上一句的尾巴带进来
    #[test]
    fn idle_inbox_keeps_only_preroll() {
        let inbox = Inbox::default();
        inbox.push(&[1i16; PREROLL]);
        inbox.push(&[2i16; 100]);
        let s = inbox.state.lock().unwrap();
        assert_eq!(s.pcm.len(), PREROLL);
        assert_eq!(*s.pcm.last().unwrap(), 2);
        assert_eq!(s.pcm[PREROLL - 101], 1);
    }

    /// 收尾之后回到预录模式：下一句之前的空闲音频不会无限堆积
    #[test]
    fn ending_returns_to_preroll() {
        let inbox = Inbox::default();
        inbox.set_live();
        inbox.push(&[1i16; PREROLL * 2]);
        assert_eq!(inbox.take(false).len(), PREROLL * 2 / FRAME);
        inbox.take(true);
        inbox.push(&[2i16; PREROLL * 2]);
        assert_eq!(inbox.state.lock().unwrap().pcm.len(), PREROLL);
    }
}
