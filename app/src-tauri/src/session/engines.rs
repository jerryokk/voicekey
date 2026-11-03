//! 渠道到引擎实例的注册表，以及每句话那些引擎任务的生命周期。
//!
//! 这两件事放在一起是有原因的：任务句柄必须和「这一句属于谁」一起管理——
//! 取消一句话时要能把它的任务全部停掉，漏掉一个就会往新句子里灌旧结果。
//! 新增渠道也只需要改这个文件。

use std::collections::HashMap;
use std::sync::Arc;

use tauri::async_runtime::JoinHandle;
use tokio::sync::mpsc::{self, UnboundedSender};
use voicekey_core::{
    Audio, BaiduEngine, ChatterflyEngine, DoubaoEngine, Engine, IflyEngine, QwenEngine, SogouEngine, WeTypeEngine,
};

use crate::session::types::Msg;
use crate::settings::Channel;

pub struct Engines {
    registry: HashMap<Channel, Arc<dyn Engine>>,
    /// 本句在跑的引擎任务（含多渠道的扇出任务）
    tasks: Vec<JoinHandle<()>>,
}

impl Engines {
    pub fn new(qwen: Arc<QwenEngine>) -> Self {
        let mut registry: HashMap<Channel, Arc<dyn Engine>> = HashMap::new();
        registry.insert(Channel::Doubao, Arc::new(DoubaoEngine));
        registry.insert(Channel::Wetype, Arc::new(WeTypeEngine::default()));
        registry.insert(Channel::Qwen, qwen);
        registry.insert(Channel::Baidu, Arc::new(BaiduEngine));
        registry.insert(Channel::Sogou, Arc::new(SogouEngine));
        registry.insert(Channel::Iflytek, Arc::new(IflyEngine));
        registry.insert(Channel::Chatterfly, Arc::new(ChatterflyEngine));
        registry.insert(Channel::WetypeOffline, crate::wtoffline::engine());
        Self { registry, tasks: Vec::new() }
    }

    pub fn get(&self, ch: &Channel) -> Option<Arc<dyn Engine>> {
        self.registry.get(ch).cloned()
    }

    /// 预热：建立长连接，缩短首包延迟。失败无所谓，真正跑的时候还会再连一次。
    pub fn prewarm(&self, channels: &[Channel]) {
        for ch in channels {
            if let Some(engine) = self.get(ch) {
                tauri::async_runtime::spawn(async move { engine.prewarm().await });
            }
        }
    }

    /// 起一个引擎任务：中间结果与定稿都以 `Msg` 回传，并带上这一句的代际。
    pub fn spawn(&mut self, ch: Channel, seg: usize, gen: u64, audio: Audio, tx: UnboundedSender<Msg>) {
        let Some(engine) = self.get(&ch) else { return };
        self.tasks.push(tauri::async_runtime::spawn(async move {
            let ptx = tx.clone();
            let partial = Box::new(move |t: &str| {
                let _ = ptx.send(Msg::Partial { gen, ch, seg, text: t.to_string() });
            });
            let res = engine.run(audio, partial).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Done { gen, ch, seg, res });
        }));
    }

    /// 同一段音频扇给多个引擎（多渠道）。每个引擎拿一条独立的通道，
    /// 扇出任务负责复制帧——音频流本身不能克隆。
    pub fn fan_out(&mut self, audio: Audio, targets: Vec<(Channel, usize)>, gen: u64, tx: UnboundedSender<Msg>) {
        let mut senders = Vec::new();
        for (ch, seg) in targets {
            let (frame_tx, frame_rx) = mpsc::unbounded_channel();
            senders.push(frame_tx);
            self.spawn(ch, seg, gen, frame_rx, tx.clone());
        }
        self.tasks.push(tauri::async_runtime::spawn(async move {
            let mut audio = audio;
            while let Some(frame) = audio.recv().await {
                for s in &senders {
                    let _ = s.send(frame.clone());
                }
            }
        }));
    }

    /// 这一句结束或被取消：停掉它的所有任务，句柄一并丢弃
    pub fn abort_all(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}
