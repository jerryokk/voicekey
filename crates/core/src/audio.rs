use crate::Audio;
use anyhow::{bail, Result};
use std::time::Duration;
use tokio::sync::mpsc;

/// 20ms @16kHz 的样本数，即 `Audio` 帧长
pub const FRAME: usize = 320;
pub const FRAME_TIME: Duration = Duration::from_millis(20);
/// 语音前垫的静音，给服务端 VAD 起音
const LEAD: usize = 10;

/// 先送 200ms 静音，之后最快 20ms 一帧。握手期间积压的音频留在上游，不会一次性灌出去
pub(crate) fn lead(mut audio: Audio) -> mpsc::Receiver<Vec<i16>> {
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
    rx
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
    *next += FRAME_TIME;
    true
}

/// 16kHz/mono/16bit wav 切成 20ms 帧，不足一帧的尾巴补零；其他格式报错
pub fn wav_frames<R: std::io::Read>(reader: R) -> Result<Vec<Vec<i16>>> {
    let mut r = hound::WavReader::new(reader)?;
    let s = r.spec();
    if s.sample_rate != 16000 || s.channels != 1 || s.bits_per_sample != 16 || s.sample_format != hound::SampleFormat::Int {
        bail!("只支持 16kHz 单声道 16bit wav（实际 {}Hz {}ch {}bit {:?}）", s.sample_rate, s.channels, s.bits_per_sample, s.sample_format);
    }
    let pcm = r.samples::<i16>().collect::<Result<Vec<_>, _>>()?;
    Ok(pcm
        .chunks(FRAME)
        .map(|c| {
            let mut f = c.to_vec();
            f.resize(FRAME, 0);
            f
        })
        .collect())
}

/// 按实时节奏（20ms 一帧）把帧喂进 `Audio`，喂完即关闭
pub fn realtime(frames: Vec<Vec<i16>>) -> Audio {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(FRAME_TIME);
        for f in frames {
            tick.tick().await;
            if tx.send(f).is_err() {
                return;
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
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
        assert!(started.elapsed() >= Duration::from_millis(200));
    }

    fn wav(rate: u32, samples: &[i16]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
        samples.iter().for_each(|&s| w.write_sample(s).unwrap());
        w.finalize().unwrap();
        out.into_inner()
    }

    #[test]
    fn wav_frames_pads_tail() {
        let frames = wav_frames(&wav(16000, &[7; FRAME + 3])[..]).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1][..3], [7, 7, 7]);
        assert!(frames[1][3..].iter().all(|&s| s == 0));
    }

    #[test]
    fn wav_frames_rejects_other_rates() {
        assert!(wav_frames(&wav(44100, &[0; 10])[..]).is_err());
    }
}
