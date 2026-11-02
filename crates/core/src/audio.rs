use crate::Audio;
use anyhow::Result;
use rubato::{FftFixedIn, Resampler};
use tokio::sync::mpsc;

pub const FRAME: usize = 320;
/// 语音前垫的静音，给服务端 VAD 起音
const LEAD: usize = 10;

/// 先送 200ms 静音，之后最快 20ms 一帧。握手期间积压的音频留在上游，不会一次性灌出去
pub struct Lead {
    rx: mpsc::Receiver<Vec<i16>>,
}

impl Lead {
    pub async fn recv(&mut self) -> Option<Vec<i16>> {
        self.rx.recv().await
    }
}

#[cfg(test)]
mod lead_tests {
    use super::*;

    #[tokio::test]
    async fn lead_pads_silence_then_plays_realtime() {
        let (tx, rx) = mpsc::unbounded_channel();
        for i in 0..5 {
            tx.send(vec![i; FRAME]).unwrap();
        }
        drop(tx);
        let mut live = lead(rx);
        let started = std::time::Instant::now();
        let mut got = Vec::new();
        while let Some(frame) = live.recv().await {
            got.push(frame[0]);
        }
        assert_eq!(got, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4]);
        assert!(started.elapsed() >= std::time::Duration::from_millis(200));
    }
}

pub fn lead(mut audio: Audio) -> Lead {
    let (tx, rx) = mpsc::channel(1);
    tokio::spawn(async move {
        let mut next = tokio::time::Instant::now();
        for _ in 0..LEAD {
            if !emit(&tx, vec![0; FRAME], &mut next).await {
                return;
            }
        }
        while let Some(frame) = audio.recv().await {
            if !emit(&tx, frame, &mut next).await {
                return;
            }
        }
    });
    Lead { rx }
}

async fn emit(tx: &mpsc::Sender<Vec<i16>>, frame: Vec<i16>, next: &mut tokio::time::Instant) -> bool {
    let now = tokio::time::Instant::now();
    if *next > now {
        tokio::time::sleep_until(*next).await;
    } else {
        *next = now;
    }
    if tx.send(frame).await.is_err() {
        return false;
    }
    *next += tokio::time::Duration::from_millis(20);
    true
}

/// 任意采样率的单声道 f32 → 16kHz Int16，按 20ms 切帧
pub struct Framer {
    rs: Option<FftFixedIn<f32>>,
    inbuf: Vec<f32>,
    pending: Vec<i16>,
}

impl Framer {
    pub fn new(rate: u32) -> Result<Self> {
        let rs = if rate == 16000 {
            None
        } else {
            Some(FftFixedIn::new(rate as usize, 16000, (rate / 100) as usize, 1, 1)?)
        };
        Ok(Self { rs, inbuf: Vec::new(), pending: Vec::new() })
    }

    pub fn push(&mut self, mono: &[f32]) -> Vec<Vec<i16>> {
        match &mut self.rs {
            None => self.pending.extend(mono.iter().map(|&s| to_i16(s))),
            Some(rs) => {
                self.inbuf.extend_from_slice(mono);
                loop {
                    let need = rs.input_frames_next();
                    if self.inbuf.len() < need {
                        break;
                    }
                    if let Ok(out) = rs.process(&[&self.inbuf[..need]], None) {
                        self.pending.extend(out[0].iter().map(|&s| to_i16(s)));
                    }
                    self.inbuf.drain(..need);
                }
            }
        }
        let mut frames = Vec::new();
        while self.pending.len() >= FRAME {
            frames.push(self.pending.drain(..FRAME).collect());
        }
        frames
    }

    /// 不足一帧的尾巴补零
    pub fn flush(&mut self) -> Option<Vec<i16>> {
        if self.pending.is_empty() {
            return None;
        }
        let mut f = std::mem::take(&mut self.pending);
        f.resize(FRAME, 0);
        Some(f)
    }
}

fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// RMS → dBFS，-50dB 以下视为静音，-10dB 封顶，归一到 0...1
pub fn level(frame: &[i16]) -> f32 {
    let sum: f32 = frame.iter().map(|&s| (s as f32) * (s as f32)).sum();
    let rms = (sum / frame.len() as f32).sqrt() / 32768.0;
    let db = 20.0 * rms.max(1e-6).log10();
    ((db + 50.0) / 40.0).clamp(0.0, 1.0)
}

/// 调试用：读 wav 文件，按实时节奏喂帧
/// 从内存里的 wav 字节切帧（自检用，不走文件系统）
pub fn frames_from_bytes(bytes: &[u8]) -> Result<Vec<Vec<i16>>> {
    let mut r = hound::WavReader::new(std::io::Cursor::new(bytes))?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 / scale)).collect::<Result<_, _>>()?
        }
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
    };
    let mono: Vec<f32> = samples.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    let mut framer = Framer::new(spec.sample_rate)?;
    let mut frames = framer.push(&mono);
    frames.extend(framer.flush());
    Ok(frames)
}

pub fn file_frames(path: &str, realtime: bool) -> Result<Audio> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 / scale)).collect::<Result<_, _>>()?
        }
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
    };
    let mono: Vec<f32> = samples.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    let mut framer = Framer::new(spec.sample_rate)?;
    let mut frames = framer.push(&mono);
    frames.extend(framer.flush());
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        for f in frames {
            if tx.send(f).is_err() {
                break;
            }
            if realtime {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
    });
    Ok(rx)
}
