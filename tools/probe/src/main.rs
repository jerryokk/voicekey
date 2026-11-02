//! 手机语音输入的最小可用外壳：启动服务 → 扫码 → 按住说话 → 识别 → 上屏。
//!
//! 传输、信令、隧道、解码都在 `voicekey-remote` 里；这里只做三件事：
//! 接线（把 Press 事件变成一次引擎调用）、上屏、把二维码摆出来。

use serde_json::json;
use std::path::PathBuf;
use voicekey_core::{DoubaoEngine, Engine};
use voicekey_remote::{Config, Event, Service};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 自测用：单独验证上屏，不启动服务
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--paste-test") {
        let text = args.get(2).cloned().unwrap_or_else(|| "上屏测试".into());
        let outcome = match voicekey_platform::paste(&text) {
            voicekey_platform::Pasted::Sent => "已发送 Ctrl+V",
            voicekey_platform::Pasted::Blocked => "前台窗口权限更高，正文已留在剪贴板",
            voicekey_platform::Pasted::Typed => "剪贴板不可用，已逐字输入",
        };
        println!("上屏：{outcome}（{text}）");
        println!("前台窗口权限是否拦住输入：{}", voicekey_platform::input_blocked());
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("--backspace-test") {
        let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        voicekey_platform::backspace(n);
        println!("已发送退格 ×{n}");
        return Ok(());
    }

    let port: u16 = std::env::var("VKSPIKE_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(47800);
    // 与产品同一个位置：app/src-tauri/binaries。env 覆盖留给特殊环境
    let cloudflared = std::env::var_os("VKSPIKE_CLOUDFLARED")
        .map(PathBuf::from)
        .or_else(|| {
            let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/binaries/cloudflared.exe");
            p.is_file().then_some(p)
        });

    let mut service = Service::start(Config { port, cloudflared, tunnel: true }).await?;
    let url = service.url();
    println!("\n扫码地址：{url}\n");
    match write_qr(&url) {
        Ok(path) => {
            println!("二维码已写入：{}", path.display());
            if std::env::var_os("VKSPIKE_NO_OPEN").is_none() {
                let _ = std::process::Command::new("explorer").arg(&path).spawn();
            }
        }
        Err(e) => println!("二维码生成失败：{e}"),
    }

    let handle = service.handle();
    let paste = std::env::var_os("VKSPIKE_NO_PASTE").is_none();

    while let Some(event) = service.next_event().await {
        match event {
            Event::Press => {
                let Some(audio) = service.take_audio().await else { continue };
                let sink = handle.clone();
                tokio::spawn(async move {
                    let out = sink.clone();
                    let partial = Box::new(move |t: &str| out.publish(json!({ "type": "partial", "text": t })));
                    match DoubaoEngine.run(audio, partial).await {
                        Ok(text) => {
                            sink.publish(json!({ "type": "final", "text": text }));
                            if paste && !text.is_empty() {
                                let body = text.clone();
                                let _ = tokio::task::spawn_blocking(move || voicekey_platform::paste(&body)).await;
                            }
                        }
                        Err(e) => {
                            sink.publish(json!({ "type": "error", "stage": "asr", "message": format!("{e:#}") }));
                        }
                    }
                });
            }
            // 音频流在松手时自然结束，引擎会自己定稿；这里不需要做事
            Event::Release => {}
            // 这个外壳只跑单渠道，不涉及候选挑选
            Event::Commit(ch) => println!("收到候选提交：{ch}（probe 单渠道，忽略）"),
            Event::Erase(n) => {
                if paste {
                    let _ = tokio::task::spawn_blocking(move || voicekey_platform::backspace(n as usize)).await;
                } else {
                    println!("收到退格 ×{n}（NO_PASTE，未发送按键）");
                }
            }
            Event::Undo => {
                if paste {
                    let _ = tokio::task::spawn_blocking(voicekey_platform::undo).await;
                    println!("收到撤销（已发送 Ctrl+Z）");
                } else {
                    println!("收到撤销（NO_PASTE，未发送按键）");
                }
            }
            Event::Abort(reason) => {
                handle.publish(json!({ "type": "error", "stage": "session", "message": reason }));
            }
        }
    }

    Ok(())
}

fn write_qr(url: &str) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let code = qrcode::QrCode::new(url.as_bytes())?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(360, 360)
        .quiet_zone(true)
        .build();
    let path = std::env::current_dir()?.join("qr.svg");
    std::fs::write(&path, svg)?;
    Ok(path)
}
