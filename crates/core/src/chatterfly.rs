use crate::ws::{self, Ws};
use crate::{Audio, Engine, Partial};
use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Oaep, RsaPublicKey};
use serde_json::{json, Value};
use sha2::Sha256;
use std::time::Duration;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

const HOST: &str = "srss.speech.sogou.com";
const PATH: &str = "/srss/v1/speech/streaming_recognize";
const PRODUCT: &str = "com.tencent.inputmethod.chatterfly";
const PRODUCT_VERSION: &str = "1.0.9.13509";

/// 服务端下发的会话密钥加密公钥（3072 位，DER SubjectPublicKeyInfo）
const PUBKEY: &[u8] = include_bytes!("chatterfly_pub.der");

/// Chatterfly（元宝输入法）云端流式识别：wss + 加密 config 首帧 + 裸 PCM 二进制帧
pub struct ChatterflyEngine;

/// 空闲超时：服务端每次出字即刷新，长语音不会因为总时长被截断
const IDLE: Duration = Duration::from_secs(30);

fn hex32() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn config() -> Value {
    // audio_id / audio_slice_id 必须是 32 位 hex，否则服务端回 invalid audio info
    let dev = json!({
        "device_category": "mac",
        "device_uuid": hex32(),
        "device_aid": hex32(),
        "device_qid": hex32(),
    });
    json!({
        "config": {
            "encoding": "LINEAR16",
            "language_code": "zh-cmn-Hans-CN",
            "model": "default",
            "punctuation_mode": "NORMAL_PUNCTUATION",
            "convert_number": true,
            "metadata": {
                // client_info 缺失或结构不符 -> invalid client info
                "client_info": {
                    "product_category": "ime",
                    "product_id": PRODUCT,
                    "product_version": PRODUCT_VERSION,
                },
                "sdk_info": { "sdk_category": "mac", "sdk_id": "nsrss", "sdk_version": "1.0.0" },
                "record_device_info": dev,
                "record_os_info": { "os_category": "mac", "os_id": "mac", "os_version": "14.0" },
                "host_device_info": dev,
                "host_os_info": { "os_category": "mac", "os_id": "mac", "os_version": "14.0" },
                "user_info": { "user_category": "normal", "user_id": "0" },
                "audio_info": { "audio_id": hex32(), "audio_slice_id": hex32() },
                "network_info": { "network_type": "wifi", "network_proxy_type": "none" },
                "runtime_info": {
                    "consumer_product_id": PRODUCT,
                    "consumer_purpose": "ime",
                    "consumer_input_type": "voice",
                },
            },
        },
        "interim_results": true,
        "single_utterance": true,
    })
}

struct Sealed {
    body: String,
    sec: String,
    vec: String,
}

/// AES-256-CBC(PKCS7) 加密 config，AES 密钥用 RSA-OAEP(SHA256) 封装
fn seal(plain: &[u8]) -> Result<Sealed> {
    let mut key = [0u8; 32];
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut key);
    rand::thread_rng().fill_bytes(&mut iv);
    let body = cbc::Encryptor::<aes::Aes256>::new((&key).into(), (&iv).into())
        .encrypt_padded_vec_mut::<Pkcs7>(plain);
    let pubkey = RsaPublicKey::from_public_key_der(PUBKEY).map_err(|e| anyhow!("公钥解析失败：{e}"))?;
    let sec = pubkey
        .encrypt(&mut rand::thread_rng(), Oaep::new::<Sha256>(), &key)
        .map_err(|e| anyhow!("密钥封装失败：{e}"))?;
    Ok(Sealed { body: B64.encode(body), sec: B64.encode(sec), vec: B64.encode(iv) })
}

/// 拼出本次响应携带的整句文本
fn transcript(v: &Value) -> String {
    let Some(rs) = v["results"].as_array() else { return String::new() };
    rs.iter()
        .filter_map(|r| Some(r["alternatives"].as_array()?.first()?["transcript"].as_str()?.to_string()))
        .collect::<Vec<_>>()
        .join("")
}

fn is_final(v: &Value) -> bool {
    v["results"].as_array().is_some_and(|rs| rs.iter().any(|r| r["is_final"] == true))
}

/// 服务端判定这句话已结束（无语音时随后直接关连接，不给 is_final）
fn utterance_ended(v: &Value) -> bool {
    v["events"].as_array().is_some_and(|es| es.iter().any(|e| e["event_type"] == "END_OF_SINGLE_UTTERANCE"))
}

fn server_error(v: &Value) -> Option<anyhow::Error> {
    let e = &v["error"];
    if e.is_null() {
        return None;
    }
    Some(anyhow!("Chatterfly {}（{}）", e["message"].as_str().unwrap_or("识别失败"), e["code"]))
}

async fn open() -> Result<(Ws, String)> {
    let sealed = seal(&serde_json::to_vec(&config())?)?;
    let headers = [
        ("X-Srss-Cipher-Key-Type", "1"),
        ("X-Srss-Cipher-Key-Sec", sealed.sec.as_str()),
        ("X-Srss-Cipher-Key-Vec", sealed.vec.as_str()),
    ];
    let ws = ws::connect(&format!("wss://{HOST}{PATH}"), &headers).await?;
    Ok((ws, sealed.body))
}

#[async_trait::async_trait]
impl Engine for ChatterflyEngine {
    async fn run(&self, mut audio: Audio, partial: Partial) -> Result<String> {
        let (ws, body) = open().await?;
        let (mut sink, mut stream) = ws.split();
        sink.send(Message::Text(body.into())).await?;

        let mut text = String::new();
        let mut closed = false;
        let mut ended = false;
        let mut deadline = Instant::now() + IDLE;
        loop {
            tokio::select! {
                biased;
                frame = audio.recv(), if !closed => match frame {
                    Some(pcm) => {
                        sink.send(Message::Binary(pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>().into())).await?;
                    }
                    None => {
                        closed = true;
                        // 结束信号：文本帧 "{}"，服务端据此返回 is_final（带标点）
                        sink.send(Message::Text("{}".into())).await?;
                    }
                },
                msg = stream.next() => {
                    let chunk = match ws::data(msg) {
                        Ok(Some(d)) => d,
                        Ok(None) => continue,
                        // 判定句末后关连接是服务端的正常收尾：没给 is_final，已出的字就是这句话的全部
                        Err(_) if ended => break,
                        Err(e) => return Err(e),
                    };
                    // 无语音结束时服务端会先发一个空文本帧
                    if chunk.is_empty() {
                        continue;
                    }
                    let v: Value = serde_json::from_slice(&chunk).context("Chatterfly 响应解析失败")?;
                    deadline = Instant::now() + IDLE;
                    if let Some(e) = server_error(&v) {
                        return Err(e);
                    }
                    ended |= utterance_ended(&v);
                    let t = transcript(&v);
                    if t != text {
                        text = t;
                        partial(&text);
                    }
                    if is_final(&v) {
                        break;
                    }
                }
                _ = tokio::time::sleep_until(deadline) => bail!("Chatterfly 响应超时"),
            }
        }
        Ok(text)
    }
}
