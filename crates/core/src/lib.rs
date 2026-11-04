pub mod audio;
mod baidu;
mod chatterfly;
mod doubao;
mod ifly;
mod opus;
mod pb;
mod qwen;
mod sogou;
pub mod util;
mod wetype;
mod ws;

use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;

/// 16kHz/mono/Int16 的 20ms（320 样本）帧流，发送端关闭即松手
pub type Audio = mpsc::UnboundedReceiver<Vec<i16>>;
pub type Partial = Box<dyn Fn(&str) + Send + Sync>;

/// 自检结果：能拿到非空文字就说明渠道链路是通的
pub struct Probe {
    pub ms: u64,
    pub text: String,
}

/// 随二进制分发的 1.6 秒测试语音
const PROBE_WAV: &[u8] = include_bytes!("../assets/probe.wav");

#[async_trait::async_trait]
pub trait Engine: Send + Sync {
    /// 消费音频帧，中间结果经 partial 回调。
    /// 正常结束返回定稿（没识别到内容为空串）；任何失败都返回 Err，不拿中间结果充当定稿
    async fn run(&self, audio: Audio, partial: Partial) -> Result<String>;
    /// 预先建连，缩短首包延迟
    async fn prewarm(&self) {}

    /// 端到端自检：把内置测试语音按实时节奏喂一遍，返回识别结果与耗时。
    /// 所有渠道共用这一份实现——渠道是否可用，只有真跑一次才知道。
    async fn probe(&self) -> Result<Probe> {
        let audio = audio::realtime(audio::wav_frames(PROBE_WAV)?);
        let t0 = Instant::now();
        let text = self.run(audio, Box::new(|_| {})).await?;
        Ok(Probe { ms: t0.elapsed().as_millis() as u64, text })
    }
}

pub use baidu::BaiduEngine;
pub use chatterfly::ChatterflyEngine;
pub use doubao::DoubaoEngine;
pub use ifly::IflyEngine;
pub use qwen::QwenEngine;
pub use sogou::SogouEngine;
pub use wetype::WeTypeEngine;
