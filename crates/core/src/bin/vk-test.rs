//! vk-test doubao|wetype|qwen|baidu|sogou|iflytek|chatterfly file.wav（16kHz 单声道 16 位）
use std::time::Instant;
use voicekey_core::{audio, BaiduEngine, ChatterflyEngine, DoubaoEngine, Engine, IflyEngine, QwenEngine, SogouEngine, WeTypeEngine};

const USAGE: &str = "usage: vk-test doubao|wetype|qwen|baidu|sogou|iflytek|chatterfly file.wav";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let engine: Box<dyn Engine> = match args[1].as_str() {
        "doubao" => Box::new(DoubaoEngine),
        "wetype" => Box::new(WeTypeEngine::default()),
        "baidu" => Box::new(BaiduEngine),
        "sogou" => Box::new(SogouEngine),
        "iflytek" => Box::new(IflyEngine),
        "chatterfly" => Box::new(ChatterflyEngine),
        "qwen" => Box::new(QwenEngine::default()),
        other => {
            eprintln!("未知渠道 {other}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let frames = std::fs::File::open(&args[2])
        .map_err(anyhow::Error::from)
        .and_then(audio::wav_frames)
        .unwrap_or_else(|e| {
            eprintln!("读取 {} 失败：{e:#}", args[2]);
            std::process::exit(2);
        });
    let start = Instant::now();
    match engine.run(audio::realtime(frames), Box::new(|p| println!("… {p}"))).await {
        Ok(t) => println!("FINAL: {t} ({:.2}s)", start.elapsed().as_secs_f64()),
        Err(e) => {
            println!("ERROR: {e:#}");
            std::process::exit(1);
        }
    }
}
