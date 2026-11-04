use crate::util::{now_ms, IdFile};
use crate::{opus, Audio, Engine, Partial};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

const HOST: &str = "https://ltalk.speech.sogou.com/index.lt";
const PACK: usize = 10;
const TYPE_NO: i32 = 20512;
const VER: i32 = 1258;
const PKG: &str = "com.sogou.sogouinput";
const ID: IdFile = IdFile("sogou.json");

/// 搜狗输入法云端 ASR：HTTP POST + `opus`+LE32 帧
pub struct SogouEngine;

fn wrap(op: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + op.len());
    v.extend_from_slice(b"opus");
    v.extend_from_slice(&(op.len() as u32).to_le_bytes());
    v.extend_from_slice(op);
    v
}

fn word(v: &Value) -> Option<String> {
    v["result"]
        .as_array()?
        .first()?
        .get("vs")?
        .as_array()?
        .first()?
        .get("s")?
        .as_str()
        .map(str::to_string)
}

#[derive(Serialize, Deserialize)]
struct Dev {
    imei: String,
}

/// 每次请求返回截至当前的识别结果；status < 0 为失败
async fn post(client: &reqwest::Client, imei: &str, start: u64, seq: u64, blob: &[u8]) -> Result<Option<String>> {
    let url = format!(
        "{HOST}?area=0&base_no=&cancel=0&imei_no={imei}&start_time={start}&sequence_no={seq}&voice_length={}&result_amount=5&type_no={TYPE_NO}&v={VER}&package_name={PKG}&action_type=0&input_type=0&partial=1&token=0&net_type=wifi&action_time={}&entry_type=0&contactkey=%7C%7C{imei}%7C%7C&chuosign=0&audio_type=1",
        blob.len(),
        now_ms()
    );
    let mut body = Vec::with_capacity(14 + blob.len());
    body.extend_from_slice(b"voice_content=");
    body.extend_from_slice(blob);
    let resp = client.post(url).body(body).send().await?.error_for_status()?;
    let v: Value = resp.json().await?;
    let status = v["status"].as_i64().context("搜狗响应缺少 status")?;
    if status < 0 {
        bail!("搜狗{}（{status}）", v["message"].as_str().unwrap_or("识别失败"));
    }
    Ok(word(&v))
}

#[async_trait::async_trait]
impl Engine for SogouEngine {
    async fn run(&self, audio: Audio, partial: Partial) -> Result<String> {
        let mut audio = crate::audio::lead(audio);
        let imei = ID.get_or_create(|| Dev { imei: Uuid::new_v4().to_string().to_uppercase() })?.imei;
        let start = now_ms();
        let client = reqwest::Client::builder().tcp_nodelay(true).timeout(Duration::from_secs(10)).build()?;
        let mut enc = opus::Opus::new(opus::AUDIO, 16000, 5)?;
        let mut batch = Vec::new();
        let mut n = 0usize;
        let mut seq = 0u64;
        let mut text = String::new();
        let mut done = false;
        while !done {
            let frame = audio.recv().await;
            if let Some(f) = &frame {
                batch.extend_from_slice(&wrap(&enc.encode(f)?));
                n += 1;
            }
            done = frame.is_none();
            if n == PACK || (done && n > 0) {
                if let Some(w) = post(&client, &imei, start, seq, &batch).await? {
                    if w != text {
                        text = w;
                        partial(&text);
                    }
                }
                batch.clear();
                n = 0;
                seq += 1;
            }
        }
        Ok(text)
    }
}
