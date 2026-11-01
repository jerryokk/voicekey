//! WebRTC 接收端：浏览器 offer → 本机 answer → 收 RTP(Opus) → 解成 16k PCM。
//!
//! 两件事同时做：把音频交给下游，以及统计质量（时长按 RTP 时间戳算、丢包按 seq 缺口算）。
//! 后者是判断「要不要切回落」的唯一依据，不能省。

use std::sync::{Arc, Mutex};
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

/// 每帧样本数：16kHz × 20ms
const FRAME: usize = 320;

#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub packets: u64,
    pub bytes: u64,
    first_seq: Option<u16>,
    last_seq: u16,
    first_ts: Option<u32>,
    last_ts: u32,
}

impl Stats {
    pub fn reset(&mut self) {
        *self = Stats::default();
    }

    /// 按 RTP 时间戳换算的音频时长（Opus 时钟恒为 48kHz）
    pub fn seconds(&self) -> f64 {
        match self.first_ts {
            Some(first) => self.last_ts.wrapping_sub(first) as f64 / 48_000.0,
            None => 0.0,
        }
    }

    /// 丢包率。Opus 是预测编码，丢一包会污染解码器状态，这个数超标就说明该切回落。
    pub fn loss_percent(&self) -> f64 {
        let Some(first) = self.first_seq else { return 0.0 };
        let expected = self.last_seq.wrapping_sub(first).wrapping_add(1) as f64;
        if expected <= 0.0 {
            return 0.0;
        }
        ((expected - self.packets as f64) / expected * 100.0).max(0.0)
    }

    fn observe(&mut self, seq: u16, ts: u32, len: usize) {
        if self.first_seq.is_none() {
            self.first_seq = Some(seq);
            self.first_ts = Some(ts);
        }
        self.last_seq = seq;
        self.last_ts = ts;
        self.packets += 1;
        self.bytes += len as u64;
    }
}

/// 两条通道（WebRTC、回落 PCM）共用的音频入口。谁先到谁进，下游只认 320 样本一帧。
#[derive(Default)]
pub struct Inbox {
    pcm: Mutex<Vec<i16>>,
}

impl Inbox {
    pub fn push(&self, samples: &[i16]) {
        self.pcm.lock().unwrap().extend_from_slice(samples);
    }

    /// 取走所有整帧，不足一帧的余量留在原地等下一批
    pub fn frames(&self) -> Vec<Vec<i16>> {
        let mut guard = self.pcm.lock().unwrap();
        let whole = guard.len() / FRAME * FRAME;
        if whole == 0 {
            return Vec::new();
        }
        let rest = guard.split_off(whole);
        let full = std::mem::replace(&mut *guard, rest);
        full.chunks(FRAME).map(<[i16]>::to_vec).collect()
    }

    /// 收尾：把残余样本补零成一帧，否则最后一个字会被截掉
    pub fn flush(&self) -> Option<Vec<i16>> {
        let mut guard = self.pcm.lock().unwrap();
        if guard.is_empty() {
            return None;
        }
        let mut tail = std::mem::take(&mut *guard);
        tail.resize(FRAME, 0);
        Some(tail)
    }

    pub fn clear(&self) {
        self.pcm.lock().unwrap().clear();
    }
}

pub struct Receiver {
    connection: Arc<RTCPeerConnection>,
    stats: Arc<Mutex<Stats>>,
}

impl Receiver {
    pub async fn new(inbox: Arc<Inbox>) -> Result<Self, String> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs().map_err(|e| format!("codecs: {e}"))?;
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine).map_err(|e| format!("interceptors: {e}"))?;
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        let connection = Arc::new(
            api.new_peer_connection(RTCConfiguration {
                ice_servers: ICE
                    .iter()
                    .map(|u| RTCIceServer { urls: vec![u.to_string()], ..Default::default() })
                    .collect(),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("peer connection: {e}"))?,
        );

        let stats = Arc::new(Mutex::new(Stats::default()));
        let sink = stats.clone();
        connection.on_track(Box::new(move |track, _receiver, _transceiver| {
            let sink = sink.clone();
            let decoded = inbox.clone();
            Box::pin(async move {
                // 每个 track 一个解码器：Opus 是状态化的，跨 track 复用会带进前一句的预测状态
                let mut decoder = Decoder::new();
                while let Ok((packet, _)) = track.read_rtp().await {
                    sink.lock().unwrap().observe(
                        packet.header.sequence_number,
                        packet.header.timestamp,
                        packet.payload.len(),
                    );
                    if let Ok(d) = decoder.as_mut() {
                        if let Ok(samples) = d.decode(&packet.payload) {
                            decoded.push(&samples);
                        }
                    }
                }
            })
        }));

        Ok(Self { connection, stats })
    }

    /// 应用浏览器 offer，等到候选收集完成再返回 answer（不做 trickle，少一个状态机）
    pub async fn accept(&self, sdp: String) -> Result<String, String> {
        let offer = RTCSessionDescription::offer(sdp).map_err(|e| format!("offer: {e}"))?;
        self.connection.set_remote_description(offer).await.map_err(|e| format!("remote offer: {e}"))?;
        let answer = self.connection.create_answer(None).await.map_err(|e| format!("create answer: {e}"))?;
        self.connection.set_local_description(answer).await.map_err(|e| format!("local answer: {e}"))?;
        self.connection.gathering_complete_promise().await.recv().await;
        let local = self.connection.local_description().await.ok_or("没有本地 answer")?;
        Ok(local.sdp)
    }

    pub fn snapshot(&self) -> Stats {
        self.stats.lock().unwrap().clone()
    }

    pub fn reset(&self) {
        self.stats.lock().unwrap().reset();
    }

    pub fn state(&self) -> webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState {
        self.connection.connection_state()
    }

    pub async fn close(&self) {
        let _ = self.connection.close().await;
    }
}

/// Opus 解码器：浏览器发的是 48kHz，直接解到 16kHz 单声道——
/// 省掉一次重采样，且 20ms 正好 320 样本，正好是 Engine 要的帧长。
struct Decoder {
    dec: *mut audiopus_sys::OpusDecoder,
    buf: Vec<i16>,
}

unsafe impl Send for Decoder {}

impl Decoder {
    fn new() -> Result<Self, String> {
        let mut err = 0;
        let dec = unsafe { audiopus_sys::opus_decoder_create(16_000, 1, &mut err) };
        if dec.is_null() || err != 0 {
            return Err(format!("Opus 解码器创建失败：{err}"));
        }
        // 最长的 Opus 帧是 60ms，即 960 个 16k 样本；留足余量
        Ok(Self { dec, buf: vec![0; 5760] })
    }

    fn decode(&mut self, payload: &[u8]) -> Result<Vec<i16>, String> {
        let n = unsafe {
            audiopus_sys::opus_decode(
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
        Ok(self.buf[..n as usize].to_vec())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { audiopus_sys::opus_decoder_destroy(self.dec) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_is_counted_from_sequence_gaps() {
        let mut s = Stats::default();
        for seq in [100u16, 101, 104] {
            s.observe(seq, 960 * seq as u32, 80);
        }
        // 100..=104 期望 5 包，实际 3 包
        assert_eq!(s.packets, 3);
        assert!((s.loss_percent() - 40.0).abs() < 0.01);
    }

    #[test]
    fn inbox_yields_whole_frames_and_keeps_remainder() {
        let inbox = Inbox::default();
        inbox.push(&vec![7i16; 700]);
        let frames = inbox.frames();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len(), FRAME);
        // 700 - 640 = 60 个样本留在原地
        assert_eq!(inbox.flush().map(|f| f.len()), Some(FRAME));
    }

    #[test]
    fn duration_comes_from_rtp_timestamps() {
        let mut s = Stats::default();
        s.observe(1, 0, 80);
        s.observe(2, 48_000, 80);
        assert!((s.seconds() - 1.0).abs() < 0.001);
    }
}
