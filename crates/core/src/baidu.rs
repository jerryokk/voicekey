use crate::util::{rand_lower, IdFile};
use crate::ws::AbortOnDrop;
use crate::{opus, Audio, Engine, Partial};
use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use flate2::write::DeflateEncoder;
use flate2::{Crc, Compression};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

const HOST: &str = "https://vse.baidu.com/v2";
const VER: &str = "4.8.3.285";
const PID: i64 = 597;
const KEY: &str = "com.baidu.input_nlu";
const APP: &str = "com.baidu.input";
const PACK: usize = 8;
/// 下行帧长上限：正常 JSON 帧不过几 KB，超出说明流已错位
const MAX_FRAME: usize = 1 << 20;
const ID: IdFile = IdFile("baidu.json");

/// 百度输入法云端 ASR：HTTP 分块 up/down + deflate JSON 首包 + Opus
pub struct BaiduEngine;

fn pkt(typ: u8, payload: &[u8]) -> Vec<u8> {
    let n = payload.len() + 1;
    let mut v = Vec::with_capacity(4 + n);
    v.extend_from_slice(&(n as u32).to_le_bytes());
    v.push(typ);
    v.extend_from_slice(payload);
    v
}

fn start_pkt(sn: &str, cuid: &str) -> Result<Vec<u8>> {
    let j = json!({
        "key": KEY,
        "appid": PID,
        "pam": "",
        "vp_pam": "",
        "wwd": "",
        "ctl": "{\"s_wake\":true,\"s_sdk_vad_mode\":1,\"s_link_switch\":\"ws\"}",
        "cuid": cuid,
        "sdk_vad_mode": 0,
        "pfm": "",
        "ver": VER,
        "use_mapping": 0,
        "map_multi_info": "",
        "pid": PID,
        "smr": 16000,
        "trig": 0,
        "enable_combined_tts": 1,
        "sn": sn,
        "lsn": sn,
        "rgn_id": "",
        "app": APP,
        "offline_auth_ctrl": 0,
        "cid": 0,
        "prop_list": [10005],
        "debug": 0,
        "msn": sn,
        "crt": 0,
        "dep": "{\"pkg\":\"com.apple.mobilenotes\"}",
        "cookie": "",
        "masr": 0,
        "midx": 1,
        "fun": 537198592,
    });
    let raw = serde_json::to_vec_pretty(&j)?;
    // 官方用 tab 缩进；serde pretty 是两空格，服务端不校验空白
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    enc.write_all(&raw)?;
    let defl = enc.finish()?;
    let mut crc = Crc::new();
    crc.update(&raw);
    let mut payload = Vec::with_capacity(10 + defl.len() + 8);
    payload.extend_from_slice(b"u{");
    payload.extend_from_slice(&[8, 0, 0, 0, 0, 0, 0, 3]);
    payload.extend_from_slice(&defl);
    payload.extend_from_slice(&crc.sum().to_le_bytes());
    payload.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    Ok(pkt(0, &payload))
}

/// 下行帧：LE32(n) + 类型 1 字节 + 负载 n-1 字节
struct Frames(Vec<u8>);

impl Frames {
    /// 取出下一个完整帧；数据不够返回 None，帧头非法（长度 0 或超限）报错
    fn next(&mut self) -> Result<Option<(u8, Vec<u8>)>> {
        let Some(head) = self.0.first_chunk::<4>() else { return Ok(None) };
        let n = u32::from_le_bytes(*head) as usize;
        if n == 0 || n > MAX_FRAME {
            bail!("百度下行帧头非法（长度 {n}）");
        }
        if self.0.len() < 4 + n {
            return Ok(None);
        }
        let typ = self.0[4];
        let body = self.0[5..4 + n].to_vec();
        self.0.drain(..4 + n);
        Ok(Some((typ, body)))
    }
}

/// 下行帧类型
const T_RESULT: u8 = 240;
const T_FINAL: u8 = 241;
const T_END: u8 = 243;

/// 收下行直到定稿：中间结果经 partial 回调，返回定稿文本（无有效语音为空串）
async fn receive(down: impl std::future::Future<Output = reqwest::Result<reqwest::Response>>, partial: Partial) -> Result<String> {
    let mut resp = down.await.context("百度下行失败")?;
    if !resp.status().is_success() {
        bail!("百度下行 HTTP {}", resp.status());
    }
    let mut frames = Frames(Vec::new());
    let mut text = String::new();
    while let Some(chunk) = resp.chunk().await.context("百度下行中断")? {
        frames.0.extend_from_slice(&chunk);
        while let Some((typ, body)) = frames.next()? {
            match typ {
                T_RESULT | T_FINAL => {
                    let v: Value = serde_json::from_slice(&body).context("百度结果解析失败")?;
                    match v["err_no"].as_i64() {
                        Some(0) => {}
                        // 没有检测到有效语音
                        Some(-3005) => return Ok(String::new()),
                        _ => bail!("百度{}（{}）", v["err_msg"].as_str().unwrap_or("识别失败"), v["err_no"]),
                    }
                    let w = v["result"]["word"][0].as_str().context("百度结果缺少 word")?;
                    if typ == T_FINAL {
                        return Ok(w.to_string());
                    }
                    if w != text {
                        text = w.to_string();
                        partial(&text);
                    }
                }
                T_END => bail!("百度没有返回定稿"),
                // 244 心跳、248 VAD 状态等
                _ => {}
            }
        }
    }
    bail!("百度下行提前结束")
}

#[derive(Serialize, Deserialize)]
struct Dev {
    cuid: String,
}

#[async_trait::async_trait]
impl Engine for BaiduEngine {
    async fn run(&self, audio: Audio, partial: Partial) -> Result<String> {
        let mut audio = crate::audio::lead(audio);
        let sn = Uuid::new_v4().to_string();
        let cuid = ID
            .get_or_create(|| Dev {
                cuid: format!("{}|{}", Uuid::new_v4().simple().to_string().to_uppercase(), rand_lower(9).to_uppercase()),
            })?
            .cuid;
        let client = reqwest::Client::builder().tcp_nodelay(true).http1_only().build()?;
        let down = client
            .post(format!("{HOST}/down?sn={sn}"))
            .header("Content-Type", "text/json")
            .header("Accept-Encoding", "identity")
            .send();

        let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
        let stream = stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|b| (Ok::<_, std::io::Error>(b), rx))
        });
        let up = client
            .post(format!("{HOST}/up?sn={sn}"))
            .header("Content-Type", "application/octet-stream")
            .body(reqwest::Body::wrap_stream(stream))
            .send();
        let mut up = AbortOnDrop(tokio::spawn(up));
        let mut down = AbortOnDrop(tokio::spawn(receive(down, partial)));

        // 上行请求失败时 rx 随之丢弃，send 报错
        let send = |p: Vec<u8>| tx.send(Bytes::from(p)).map_err(|_| anyhow!("百度上行中断"));
        send(start_pkt(&sn, &cuid)?)?;
        send(pkt(5, br#"{"realtime-log":"INPUT&a1&13.3.16.2&0&com.baidu.input"}"#))?;

        let mut enc = opus::Opus::baidu()?;
        // 首个音频包带 4 字节前缀
        let mut batch = vec![0x44, 0, 0, 0];
        let mut n = 0usize;
        while let Some(frame) = audio.recv().await {
            batch.extend_from_slice(&enc.encode_bd(&frame)?);
            n += 1;
            if n == PACK {
                send(pkt(1, &batch))?;
                batch.clear();
                n = 0;
            }
        }
        if n > 0 {
            send(pkt(1, &batch))?;
        }
        send(pkt(9, &[]))?;
        send(pkt(7, &[]))?;
        // 关闭上行流（send 只借用 tx）
        drop(tx);

        let text = tokio::time::timeout(Duration::from_secs(8), &mut down).await.map_err(|_| anyhow!("百度定稿超时"))???;
        let resp = tokio::time::timeout(Duration::from_secs(4), &mut up).await.map_err(|_| anyhow!("百度上行超时"))??.context("百度上行失败")?;
        if !resp.status().is_success() {
            bail!("百度上行 HTTP {}", resp.status());
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_split_and_wait_for_more() {
        let mut f = Frames([pkt(240, b"ab"), pkt(243, &[])[..3].to_vec()].concat());
        assert_eq!(f.next().unwrap(), Some((240, b"ab".to_vec())));
        assert_eq!(f.next().unwrap(), None);
        f.0.extend_from_slice(&pkt(243, &[])[3..]);
        assert_eq!(f.next().unwrap(), Some((243, vec![])));
        assert_eq!(f.next().unwrap(), None);
    }

    #[test]
    fn bad_header_is_error_not_stall() {
        assert!(Frames(vec![0, 0, 0, 0, 9]).next().is_err());
        assert!(Frames(u32::MAX.to_le_bytes().to_vec()).next().is_err());
    }
}
