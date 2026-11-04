use crate::pb::{self, PBuf};
use crate::util::{now_ms, IdFile};
use crate::ws::{self, AbortOnDrop};
use crate::{audio, opus, Audio, Engine, Partial};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, Message};

const WS_URL: &str = "wss://frontier-audio-ime-ws.doubao.com/ocean/api/v1/ws";
const OK: i64 = 20_000_000;
/// 官方客户端内置的 ASR app key（1.4.6 IAsr.b()）
const APP_KEY: &str = "OrnqKvSSrs";
/// TaskRequest 的帧序：首帧、中间帧、结束帧
const FIRST: u64 = 1;
const MIDDLE: u64 = 3;
const LAST: u64 = 9;
const ID: IdFile = IdFile("doubao.json");

/// 豆包输入法流式识别：asr.AsrRequest protobuf over WSS，Opus 20ms 帧
pub struct DoubaoEngine;

fn request(token: &str, method: &str, payload: &str, audio: &[u8], rid: &str, frame: u64) -> Message {
    let mut pb = PBuf::new();
    if !token.is_empty() {
        pb = pb.s(2, token);
    }
    pb = pb.s(3, "ASR").s(5, method);
    if !payload.is_empty() {
        pb = pb.s(6, payload);
    }
    if !audio.is_empty() {
        pb = pb.b(7, audio);
    }
    pb = pb.s(8, rid);
    if frame != 0 {
        pb = pb.v(9, frame);
    }
    Message::Binary(pb.0.into())
}

/// asr.AsrResponse；proto3 省略默认值，缺字段即零值
struct Response {
    event: String,
    status: i64,
    message: String,
    result: String,
}

impl Response {
    fn parse(data: &[u8]) -> Result<Self> {
        let f = pb::parse(data)?;
        Ok(Self {
            event: f.string(4).unwrap_or_default(),
            // int32 负值按 64 位补码编码
            status: f.varint(5).unwrap_or(0) as i32 as i64,
            message: f.string(6).unwrap_or_default(),
            result: f.string(7).unwrap_or_default(),
        })
    }

    /// TaskFailed / SessionFailed 等：服务端拒绝本次会话
    fn failed(&self) -> bool {
        self.event.ends_with("Failed")
    }

    fn error(&self) -> anyhow::Error {
        anyhow!("豆包 {} {}: {}", self.event, self.status, self.message)
    }
}

/// results[0].text 是整段累计文本（其余为辅助候选）；results 为 null 的是心跳
fn result_text(json: &str) -> Result<Option<String>> {
    let v: Value = serde_json::from_str(json).context("豆包结果解析失败")?;
    let Some(first) = v["results"].as_array().and_then(|r| r.first()) else { return Ok(None) };
    Ok(Some(first["text"].as_str().context("豆包结果缺少 text")?.to_string()))
}

/// 录音分流：边转发给当前会话边留底；换设备重试时先灌入留底，后续音频继续转给新会话。
/// 录音结束后 out 为 None，新会话的音频流读完留底即关闭
struct Tee {
    kept: Vec<Vec<i16>>,
    out: Option<mpsc::UnboundedSender<Vec<i16>>>,
}

impl Tee {
    fn spawn(mut audio: Audio) -> (Arc<Mutex<Tee>>, Audio, AbortOnDrop<()>) {
        let (out, rx) = mpsc::unbounded_channel();
        let tee = Arc::new(Mutex::new(Tee { kept: Vec::new(), out: Some(out) }));
        let t = tee.clone();
        let task = AbortOnDrop(tokio::spawn(async move {
            while let Some(f) = audio.recv().await {
                let mut t = t.lock().unwrap();
                if let Some(out) = &t.out {
                    // 会话已失败时接收端已丢弃，只留底
                    let _ = out.send(f.clone());
                }
                t.kept.push(f);
            }
            t.lock().unwrap().out = None;
        }));
        (tee, rx, task)
    }

    fn restart(&mut self) -> Audio {
        let (out, rx) = mpsc::unbounded_channel();
        for f in &self.kept {
            out.send(f.clone()).expect("接收端就在本函数里");
        }
        if self.out.is_some() {
            self.out = Some(out);
        }
        rx
    }
}

#[async_trait::async_trait]
impl Engine for DoubaoEngine {
    async fn run(&self, audio: Audio, partial: Partial) -> Result<String> {
        let (tee, first, _task) = Tee::spawn(audio);
        let partial: Arc<Partial> = Arc::new(partial);
        match session(first, partial.clone()).await {
            Ok(t) => Ok(t),
            Err(Fail::Other(e)) => Err(e),
            // 设备被服务端拒绝（如 service discovery failure）：换新设备，录音从头重放
            Err(Fail::Rejected(e)) => {
                ID.remove()?;
                let replay = tee.lock().unwrap().restart();
                session(replay, partial).await.map_err(|r| {
                    let r = match r {
                        Fail::Rejected(r) => {
                            // 新设备同样被拒，不留着它
                            let _ = ID.remove();
                            r
                        }
                        Fail::Other(r) => r,
                    };
                    anyhow!("{e}；换设备重试：{r:#}")
                })
            }
        }
    }
}

/// Rejected：尚未出字时服务端判会话失败，换设备可能恢复
enum Fail {
    Other(anyhow::Error),
    Rejected(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for Fail {
    fn from(e: E) -> Self {
        Fail::Other(e.into())
    }
}

async fn session(audio: Audio, partial: Arc<Partial>) -> std::result::Result<String, Fail> {
    let mut audio = audio::lead(audio);
    let device = DoubaoDevice::load().await?;
    let rid = uuid::Uuid::new_v4().to_string();
    let url = format!("{WS_URL}?aid=401734&device_id={}", device.did);
    let ua = device.phone.ua();
    let ws = ws::connect(&url, &[("User-Agent", &ua), ("proto-version", "v2"), ("x-custom-keepalive", "true")]).await?;
    let (mut tx, mut rx) = ws.split();

    let session = json!({
        "audio_info": {"channel": 1, "format": "speech_opus", "sample_rate": 16000},
        "enable_punctuation": true,
        "enable_speech_rejection": true,
        "extra": {"app_name": "oime", "cell_compress_rate": 8, "did": device.did,
                  "enable_asr_threepass": false, "enable_asr_twopass": true, "input_mode": "stream"},
    })
    .to_string();
    tx.send(request(APP_KEY, "StartTask", "", &[], &rid, 0)).await?;
    expect(&mut rx, "TaskStarted").await?;
    tx.send(request(APP_KEY, "StartSession", &session, &[], &rid, 0)).await?;
    expect(&mut rx, "SessionStarted").await?;

    let mut receiver = AbortOnDrop(tokio::spawn(async move {
        let mut text = String::new();
        loop {
            // 服务端约每秒一个心跳包，静音期间也不会长时间无包；结束阶段由外层限时
            let r = Response::parse(&ws::recv(&mut rx, Duration::from_secs(30)).await?)?;
            if r.failed() {
                return Err(if text.is_empty() { Fail::Rejected(r.error()) } else { Fail::Other(r.error()) });
            }
            if !r.result.is_empty() {
                if let Some(t) = result_text(&r.result)? {
                    if t != text {
                        text = t;
                        partial(&text);
                    }
                }
            }
            if r.event == "SessionFinished" {
                return Ok(text);
            }
        }
    }));

    let mut enc = opus::Opus::new(opus::VOIP, 16000, 5)?;
    let ts0 = now_ms();
    let meta = |i: u64| format!(r#"{{"extra":{{}},"timestamp_ms":{}}}"#, ts0 + i * 20);
    let mut index = 0u64;
    while let Some(frame) = audio.recv().await {
        let pkt = enc.encode(&frame)?;
        let kind = if index == 0 { FIRST } else { MIDDLE };
        tx.send(request("", "TaskRequest", &meta(index), &pkt, &rid, kind)).await?;
        index += 1;
    }
    tx.send(request("", "TaskRequest", &meta(index), &[], &rid, LAST)).await?;
    tx.send(request(APP_KEY, "FinishSession", "", &[], &rid, 0)).await?;

    let limit = Duration::from_secs_f64(10.0 + index as f64 / 200.0);
    tokio::time::timeout(limit, &mut receiver).await.map_err(|_| anyhow!("豆包定稿超时"))??
}

/// 握手阶段等待指定事件；此时被拒都算设备问题
async fn expect<S>(rx: &mut S, event: &str) -> std::result::Result<(), Fail>
where
    S: Stream<Item = Result<Message, tungstenite::Error>> + Unpin,
{
    loop {
        let r = Response::parse(&ws::recv(rx, Duration::from_secs(10)).await?)?;
        if r.failed() || (r.event == event && r.status != OK) {
            return Err(Fail::Rejected(r.error()));
        }
        if r.event == event {
            return Ok(());
        }
    }
}

/// 注册得到的设备 id 与随机选定的机型，一起缓存复用
#[derive(Serialize, Deserialize)]
struct DoubaoDevice {
    did: String,
    phone: Phone,
}

const APP: &[(&str, &str)] = &[
    ("aid", "401734"), ("app_name", "oime"), ("channel", "official"),
    ("version_code", "100406010"), ("version_name", "1.4.6"),
    ("manifest_version_code", "100406010"), ("update_version_code", "100406010"),
    ("package", "com.bytedance.android.doubaoime"),
];
/// 真实机型：brand, model, Android 版本, API, build, resolution, dpi
const PHONES: &[[&str; 7]] = &[
    ["google", "Pixel 7 Pro", "16", "36", "BP2A.250605.031.A2", "1080*2340", "420"],
    ["google", "Pixel 8", "15", "35", "AP4A.250205.002", "1080*2400", "420"],
    ["google", "Pixel 9", "15", "35", "AP4A.250205.002", "1080*2424", "420"],
    ["google", "Pixel 6", "14", "34", "AP2A.240805.005", "1080*2400", "420"],
    ["samsung", "SM-S9210", "14", "34", "UP1A.231005.007", "1080*2340", "480"],
    ["samsung", "SM-S9280", "14", "34", "UP1A.231005.007", "1080*2340", "450"],
    ["samsung", "SM-S9180", "14", "34", "UP1A.231005.007", "1080*2316", "450"],
    ["Xiaomi", "23127PN0CC", "14", "34", "UKQ1.230804.001", "1200*2670", "440"],
    ["Xiaomi", "2211133C", "14", "34", "UKQ1.230804.001", "1080*2400", "440"],
    ["Xiaomi", "24129PN74C", "15", "35", "AQ3A.240912.001", "1200*2670", "440"],
    ["Redmi", "23090RA98C", "14", "34", "UKQ1.231003.002", "1220*2712", "440"],
    ["Redmi", "23113RKC6C", "14", "34", "UKQ1.231003.002", "1440*3200", "560"],
    ["vivo", "V2309A", "14", "34", "UP1A.231005.007", "1260*2800", "480"],
    ["vivo", "V2324A", "14", "34", "UP1A.231005.007", "1260*2800", "480"],
    ["OPPO", "PHZ110", "14", "34", "UKQ1.231108.001", "1264*2780", "480"],
    ["OnePlus", "PJD110", "14", "34", "UKQ1.230924.001", "1440*3168", "560"],
    ["OnePlus", "PJE110", "14", "34", "UKQ1.230924.001", "1264*2780", "480"],
];

#[derive(Serialize, Deserialize)]
struct Phone {
    brand: String,
    model: String,
    os: String,
    api: String,
    build: String,
    resolution: String,
    dpi: String,
}

impl Phone {
    fn random() -> Self {
        let [brand, model, os, api, build, resolution, dpi] = PHONES[rand::random::<usize>() % PHONES.len()].map(String::from);
        Self { brand, model, os, api, build, resolution, dpi }
    }

    fn ua(&self) -> String {
        format!(
            "com.bytedance.android.doubaoime/100406010 (Linux; U; Android {}; en_US; {}; Build/{}; Cronet/TTNetVersion:94cf429a 2025-11-17 QuicVersion:1f89f732 2025-05-08)",
            self.os, self.model, self.build
        )
    }

    fn params(&self) -> Vec<(&'static str, String)> {
        let mut v: Vec<(&str, String)> = APP.iter().map(|(k, v)| (*k, v.to_string())).collect();
        v.extend([
            ("device_platform", "android".into()), ("os", "android".into()), ("os_api", self.api.clone()),
            ("os_version", self.os.clone()), ("device_type", self.model.clone()), ("device_brand", self.brand.clone()),
            ("device_model", self.model.clone()), ("resolution", self.resolution.clone()), ("dpi", self.dpi.clone()),
            ("language", "zh".into()), ("timezone", "8".into()), ("access", "wifi".into()),
            ("rom", self.build.clone()), ("rom_version", self.build.clone()),
        ]);
        v
    }
}

impl DoubaoDevice {
    async fn load() -> Result<Self> {
        if let Some(d) = ID.load() {
            return Ok(d);
        }
        let d = Self::register().await?;
        ID.save(&d)?;
        Ok(d)
    }

    async fn register() -> Result<Self> {
        let phone = Phone::random();
        let cdid = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let mut q = phone.params();
        let mut header: serde_json::Map<String, Value> = q.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        let extra = json!({
            "device_id": 0, "install_id": 0, "cdid": cdid,
            "openudid": format!("{:016x}", rand::random::<u64>()),
            "clientudid": uuid::Uuid::new_v4().to_string(),
            "region": "CN", "tz_name": "Asia/Shanghai", "tz_offset": 28800,
            "sim_region": "cn", "carrier_region": "cn", "cpu_abi": "arm64-v8a", "build_serial": "unknown",
            "not_request_sender": 0, "sig_hash": "", "google_aid": "", "mc": "", "serial_number": "",
        });
        let Value::Object(extra) = extra else { unreachable!() };
        header.extend(extra);
        let body = json!({"magic_tag": "ss_app_log", "header": header, "_gen_time": now});

        q.extend([("ssmix", "a".into()), ("_rticket", now.to_string()), ("cdid", cdid), ("ac", "wifi".into())]);
        let reg: Value = reqwest::Client::new()
            .post("https://log-klink.zijieapi.com/service/2/device_register/")
            .query(&q)
            .timeout(Duration::from_secs(10))
            .header("User-Agent", phone.ua())
            .json(&body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .context("豆包设备注册失败")?
            .json()
            .await
            .context("豆包设备注册响应解析失败")?;
        let did = reg["device_id_str"].as_str().context("豆包设备注册响应缺少 device_id_str")?;
        if did.is_empty() || did == "0" {
            bail!("豆包设备注册被拒绝");
        }
        Ok(Self { did: did.to_string(), phone })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_text_takes_cumulative_first() {
        let r = result_text(r#"{"results":[{"text":"不错，我们"},{"text":"不错我们"}]}"#).unwrap();
        assert_eq!(r.as_deref(), Some("不错，我们"));
    }

    #[test]
    fn heartbeat_has_no_text() {
        assert_eq!(result_text(r#"{"results":null,"extra":{"heartbeat_num":3}}"#).unwrap(), None);
    }

    #[test]
    fn malformed_result_is_error() {
        assert!(result_text("{").is_err());
        assert!(result_text(r#"{"results":[{}]}"#).is_err());
    }

    #[tokio::test]
    async fn tee_replays_kept_and_closes_after_source_ends() {
        let (tx, rx) = mpsc::unbounded_channel();
        let (tee, mut first, _task) = Tee::spawn(rx);
        tx.send(vec![1]).unwrap();
        assert_eq!(first.recv().await, Some(vec![1]));
        drop(first);
        let mut replay = tee.lock().unwrap().restart();
        tx.send(vec![2]).unwrap();
        drop(tx);
        assert_eq!(replay.recv().await, Some(vec![1]));
        assert_eq!(replay.recv().await, Some(vec![2]));
        assert_eq!(replay.recv().await, None);
    }
}
