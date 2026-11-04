use crate::util::{civil_from_days, urlencode};
use crate::ws;
use crate::{Audio, Engine, Partial};
use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

const HOST: &str = "100ime-iat-api.xfyun.cn";
const PATH: &str = "/v2/iat";
const APPID: &str = "100IME";
const API_KEY: &str = "15461402826c8360ebcf1270cbba88e5";
const API_SECRET: &str = "sH8v2wSSxxHTy1UNfN6dFaSqVjlmR7BB";
const CHUNK: usize = 1280;

/// 讯飞输入法云端 ASR：IAT WebSocket + HMAC-SHA256
pub struct IflyEngine;

fn http_gmt() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let days = t.div_euclid(86400);
    let tod = t.rem_euclid(86400);
    let h = tod / 3600;
    let m = (tod % 3600) / 60;
    let s = tod % 60;
    let wday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    let (y, mo, d) = civil_from_days(days);
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][mo as usize - 1];
    format!("{wday}, {d:02} {mon} {y} {h:02}:{m:02}:{s:02} GMT")
}

fn auth_url() -> Result<String> {
    let date = http_gmt();
    let origin = format!("host: {HOST}\ndate: {date}\nGET {PATH} HTTP/1.1");
    let mut mac = Hmac::<Sha256>::new_from_slice(API_SECRET.as_bytes())?;
    mac.update(origin.as_bytes());
    let sig = B64.encode(mac.finalize().into_bytes());
    let auth_origin = format!(r#"api_key="{API_KEY}", algorithm="hmac-sha256", headers="host date request-line", signature="{sig}""#);
    let auth = B64.encode(auth_origin);
    Ok(format!("wss://{HOST}{PATH}?authorization={}&date={}&host={HOST}", urlencode(&auth), urlencode(&date)))
}

fn pgs_text(segs: &mut BTreeMap<i64, String>, v: &Value) -> Option<String> {
    let r = v.get("data")?.get("result")?;
    let sn = r.get("sn")?.as_i64()?;
    let mut t = String::new();
    if let Some(ws) = r.get("ws").and_then(Value::as_array) {
        for w in ws {
            if let Some(s) = w.get("cw").and_then(Value::as_array).and_then(|a| a.first()).and_then(|c| c.get("w")).and_then(Value::as_str) {
                t.push_str(s);
            }
        }
    }
    // rpl：本条替换 sn 在 rg=[a, b] 内的旧片段
    if r.get("pgs").and_then(Value::as_str) == Some("rpl") {
        if let Some([a, b]) = r.get("rg").and_then(Value::as_array).and_then(|rg| rg.iter().map(Value::as_i64).collect::<Option<Vec<_>>>()).as_deref() {
            segs.retain(|k, _| k < a || k > b);
        }
    }
    segs.insert(sn, t);
    Some(segs.values().cloned().collect())
}

/// status：0 首帧（带 common/business）、1 中间帧、2 结束帧
fn frame(status: i32, audio: &[u8]) -> Message {
    let data = json!({"status": status, "format": "audio/L16;rate=16000", "encoding": "raw", "audio": B64.encode(audio)});
    let v = if status == 0 {
        json!({
            "common": {"app_id": APPID},
            "business": {"language":"zh_cn","domain":"iat","accent":"mandarin","vad_eos":3000,"dwa":"wpgs","nunum":1,"ptt":1},
            "data": data,
        })
    } else {
        json!({ "data": data })
    };
    Message::Text(v.to_string().into())
}

#[async_trait::async_trait]
impl Engine for IflyEngine {
    async fn run(&self, audio: Audio, partial: Partial) -> Result<String> {
        let mut audio = crate::audio::lead(audio);
        let ws = ws::connect(&auth_url()?, &[]).await?;
        let mut segs = BTreeMap::new();
        let mut text = String::new();
        let mut status = 0;
        let mut buf = Vec::new();

        // 返回是否为最后一条结果
        let mut apply = |raw: &[u8], text: &mut String| -> Result<bool> {
            let v: Value = serde_json::from_slice(raw).context("讯飞响应解析失败")?;
            let code = v["code"].as_i64().context("讯飞响应缺少 code")?;
            if code != 0 {
                bail!("讯飞{}（{code}）", v["message"].as_str().unwrap_or("识别失败"));
            }
            if let Some(w) = pgs_text(&mut segs, &v) {
                if w != *text {
                    *text = w;
                    partial(text);
                }
            }
            Ok(v["data"]["status"].as_i64() == Some(2))
        };

        // 服务端按 vad_eos 判定说完时会主动回 status=2 并关连接：收到即定稿，返回已识别的文字，不再发帧。
        // 先收后发（biased），已到的 status=2 不会被下一次发送的失败盖掉
        let (mut sink, mut stream) = ws.split();
        loop {
            tokio::select! {
                biased;
                msg = stream.next() => {
                    if let Some(d) = ws::data(msg)? {
                        if apply(&d, &mut text)? {
                            return Ok(text);
                        }
                    }
                }
                pcm = audio.recv() => {
                    let end = pcm.is_none();
                    buf.extend(pcm.iter().flatten().flat_map(|s| s.to_le_bytes()));
                    let sent: Result<()> = async {
                        while buf.len() >= CHUNK {
                            sink.send(frame(status, &buf[..CHUNK])).await?;
                            buf.drain(..CHUNK);
                            status = 1;
                        }
                        if end {
                            if !buf.is_empty() {
                                sink.send(frame(status, &buf)).await?;
                            }
                            sink.send(frame(2, &[])).await?;
                        }
                        Ok(())
                    }
                    .await;
                    // 发送失败多半是服务端定稿后关了连接：停发，去接收端取 status=2；真断了那边会报错
                    if end || sent.is_err() {
                        break;
                    }
                }
            }
        }
        let deadline = Instant::now() + Duration::from_millis(2500);
        loop {
            let d = ws::recv(&mut stream, deadline.saturating_duration_since(Instant::now())).await.context("讯飞定稿")?;
            if apply(&d, &mut text)? {
                return Ok(text);
            }
        }
    }
}
