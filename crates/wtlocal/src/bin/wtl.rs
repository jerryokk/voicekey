//! wtl <模型目录> a.wav [b.wav ...]   逐文件离线解码并计时
//! wtl <模型目录> --live a.wav         按实时节奏推流，打印中间结果
//! wtl --unpack pack.apk <模型目录>    解包官方语音包
//! wav 须为 16kHz 单声道 16 位
use std::time::Instant;
use voicekey_core::audio::{realtime, wav_frames, FRAME_TIME};
use voicekey_core::Engine;
use voicekey_wtlocal::{load, pack, LocalEngine, Stream};

const USAGE: &str = "usage: wtl <模型目录> a.wav [b.wav ...] | wtl <模型目录> --live a.wav | wtl --unpack pack.apk <模型目录>";

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

fn frames(path: &str) -> Vec<Vec<i16>> {
    std::fs::File::open(path).map_err(anyhow::Error::from).and_then(wav_frames).unwrap_or_else(|e| {
        eprintln!("读取 {path} 失败：{e:#}");
        std::process::exit(2);
    })
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        usage();
    }
    if args[0] == "--unpack" {
        if args.len() != 3 {
            usage();
        }
        let t = Instant::now();
        pack::unpack(args[1].as_ref(), args[2].as_ref()).expect("解包失败");
        println!("unpacked in {:.1}s", t.elapsed().as_secs_f64());
        return;
    }
    if args[1] == "--live" && args.len() != 3 {
        usage();
    }
    let dir = std::path::PathBuf::from(&args[0]);
    let t = Instant::now();
    let m = load(&dir).expect("模型加载失败");
    eprintln!("load {:.0}ms", t.elapsed().as_secs_f64() * 1e3);
    if args[1] == "--live" {
        let frames = frames(&args[2]);
        let audio_len = FRAME_TIME * frames.len() as u32;
        let t = Instant::now();
        let r = LocalEngine { dir }.run(realtime(frames), Box::new(move |p| println!("… {:.2}s {p}", t.elapsed().as_secs_f64()))).await;
        let total = t.elapsed();
        let after = total.saturating_sub(audio_len);
        println!("FINAL +{:.0}ms after audio end ({:.2}s) {}", after.as_secs_f64() * 1e3, total.as_secs_f64(), r.expect("识别失败"));
        return;
    }
    for f in &args[1..] {
        let frames = frames(f);
        let t = Instant::now();
        let mut s = Stream::new(m.clone());
        for c in &frames {
            s.push(c);
        }
        let txt = s.finish();
        let name = std::path::Path::new(f).file_stem().expect("文件名").to_string_lossy();
        let secs = (FRAME_TIME * frames.len() as u32).as_secs_f64();
        println!("{name} audio {secs:.2}s {:.0}ms\t{txt}", t.elapsed().as_secs_f64() * 1e3);
    }
}
