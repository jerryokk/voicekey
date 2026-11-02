pub mod audio;
pub mod baidu;
pub mod chatterfly;
pub mod doubao;
pub mod opus;
pub mod pb;
pub mod qwen;
pub mod util;
pub mod sogou;
pub mod wetype;
pub mod ifly;
pub mod ws;

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
    /// 消费音频帧，中间结果经 partial 回调，返回最终文本
    async fn run(&self, audio: Audio, partial: Partial) -> Result<String>;
    /// 预先建连，缩短首包延迟
    async fn prewarm(&self) {}

    /// 端到端自检：把内置测试语音按实时节奏喂一遍，返回识别结果与耗时。
    /// 七个渠道共用这一份实现——渠道是否可用，只有真跑一次才知道。
    async fn probe(&self) -> Result<Probe> {
        let frames = audio::frames_from_bytes(PROBE_WAV)?;
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            for f in frames {
                if tx.send(f).is_err() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        });
        let t0 = Instant::now();
        let text = self.run(rx, Box::new(|_| {})).await?;
        Ok(Probe { ms: t0.elapsed().as_millis() as u64, text })
    }
}

pub use baidu::BaiduEngine;
pub use chatterfly::ChatterflyEngine;
pub use doubao::DoubaoEngine;
pub use qwen::QwenEngine;
pub use sogou::SogouEngine;
pub use wetype::WeTypeEngine;
pub use ifly::IflyEngine;
