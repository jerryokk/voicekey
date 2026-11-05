use crate::Audio;
use anyhow::{bail, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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

/// 没在说话时保留的最近音频：16kHz × 500ms。没有这段，第一个字会被吃掉：
/// 手机的 start 绕隧道，常比直连过来的声音晚到；本机麦克风按下就开，要按满防误触的时长才接进这一句
pub const PREROLL: usize = 8_000;
const PREROLL_TIME: Duration = Duration::from_millis(500);

/// 一个声源的 16k 样本入口：手机的 WebRTC 解码、电脑本机麦克风都往这里写，
/// 按下后由 [`pump`] 切帧送进本句
#[derive(Default)]
pub struct Inbox {
    /// 样本与「正被接进本句」放在同一把锁里：收尾时取残余和退出 live 必须一步完成，
    /// 中间插进来的样本要么属于这一句，要么属于下一句的预录，不能两头都不算
    state: Mutex<InboxState>,
}

#[derive(Default)]
struct InboxState {
    pcm: Vec<i16>,
    /// 不在说话时只留最近 [`PREROLL`] 个样本
    live: bool,
    /// 最近一次收到样本的时刻
    last: Option<Instant>,
}

impl InboxState {
    /// 麦克风只在按住时开，空闲时没有新样本把旧的挤掉：太久以前的（被拒的那一按、上一句的尾包）不算预录
    fn drop_stale(&mut self) {
        if !self.live && self.last.is_some_and(|t| t.elapsed() > PREROLL_TIME) {
            self.pcm.clear();
        }
    }
}

impl Inbox {
    pub fn push(&self, samples: &[i16]) {
        let mut s = self.state.lock().unwrap();
        s.drop_stale();
        s.last = Some(Instant::now());
        s.pcm.extend_from_slice(samples);
        if !s.live && s.pcm.len() > PREROLL {
            let drop = s.pcm.len() - PREROLL;
            s.pcm.drain(..drop);
        }
    }

    /// 接进本句：从此不再裁剪，预录的那段成为这一句的开头
    pub fn set_live(&self) {
        let mut s = self.state.lock().unwrap();
        s.drop_stale();
        s.live = true;
    }

    /// 取走所有整帧，不足一帧的余量留在原地等下一批。
    /// `end` 时连余量也补零成一帧取走（否则最后一个字会被截掉），并退出 live
    pub fn take(&self, end: bool) -> Vec<Vec<i16>> {
        let mut s = self.state.lock().unwrap();
        let len = s.pcm.len();
        let whole = if end { len.div_ceil(FRAME) * FRAME } else { len / FRAME * FRAME };
        s.pcm.resize(len.max(whole), 0);
        let frames = s.pcm.drain(..whole).collect::<Vec<_>>().chunks(FRAME).map(<[i16]>::to_vec).collect();
        if end {
            s.live = false;
        }
        frames
    }

    /// 丢掉缓存的全部样本：麦克风关掉时残留的声音不属于下一句
    pub fn clear(&self) {
        self.state.lock().unwrap().pcm.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.state.lock().unwrap().pcm.len()
    }
}

/// 把 Inbox 里的样本按 20ms 切帧送出去，`stop` 置位后收尾退出。
/// 10ms 轮询一次：跟得上实时，又不会空转
pub async fn pump(inbox: Arc<Inbox>, tx: mpsc::UnboundedSender<Vec<i16>>, stop: Arc<AtomicBool>) {
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    loop {
        tick.tick().await;
        let end = stop.load(Ordering::Relaxed);
        for frame in inbox.take(end) {
            let _ = tx.send(frame);
        }
        if end {
            return;
        }
    }
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

    #[test]
    fn inbox_yields_whole_frames_and_pads_the_tail() {
        let inbox = Inbox::default();
        inbox.set_live();
        inbox.push(&[7i16; 700]);
        let frames = inbox.take(false);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len(), FRAME);
        // 700 - 640 = 60 个样本留在原地，收尾时补零成一帧
        let tail = inbox.take(true);
        assert_eq!(tail.len(), 1);
        assert_eq!(&tail[0][..60], &[7i16; 60]);
        assert!(tail[0][60..].iter().all(|&s| s == 0));
        assert!(inbox.take(true).is_empty());
    }

    /// 没在说话时只留最近半秒：既不吃掉第一个字，也不把上一句的尾巴带进来
    #[test]
    fn idle_inbox_keeps_only_preroll() {
        let inbox = Inbox::default();
        inbox.push(&[1i16; PREROLL]);
        inbox.push(&[2i16; 100]);
        let s = inbox.state.lock().unwrap();
        assert_eq!(s.pcm.len(), PREROLL);
        assert_eq!(*s.pcm.last().unwrap(), 2);
        assert_eq!(s.pcm[PREROLL - 101], 1);
    }

    /// 收尾之后回到预录模式：下一句之前的空闲音频不会无限堆积
    #[test]
    fn ending_returns_to_preroll() {
        let inbox = Inbox::default();
        inbox.set_live();
        inbox.push(&[1i16; PREROLL * 2]);
        assert_eq!(inbox.take(false).len(), PREROLL * 2 / FRAME);
        inbox.take(true);
        inbox.push(&[2i16; PREROLL * 2]);
        assert_eq!(inbox.len(), PREROLL);
        inbox.clear();
        assert_eq!(inbox.len(), 0);
    }

    /// 麦克风只在按住时开：隔了很久的残留（被拒的那一按）不能接进下一句
    #[test]
    fn stale_preroll_is_dropped() {
        let inbox = Inbox::default();
        inbox.push(&[1i16; 100]);
        inbox.state.lock().unwrap().last = Some(Instant::now() - PREROLL_TIME * 2);
        inbox.set_live();
        assert_eq!(inbox.len(), 0);
    }
}
