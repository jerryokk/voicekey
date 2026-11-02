//! 渠道到引擎实例的注册表。新增渠道时只改这里一处。

use std::collections::HashMap;
use std::sync::Arc;

use voicekey_core::{
    BaiduEngine, ChatterflyEngine, DoubaoEngine, Engine, IflyEngine, QwenEngine, SogouEngine, WeTypeEngine,
};

use crate::settings::Channel;

pub fn registry(qwen: Arc<QwenEngine>) -> HashMap<Channel, Arc<dyn Engine>> {
    let mut m: HashMap<Channel, Arc<dyn Engine>> = HashMap::new();
    m.insert(Channel::Doubao, Arc::new(DoubaoEngine));
    m.insert(Channel::Wetype, Arc::new(WeTypeEngine::default()));
    m.insert(Channel::Qwen, qwen);
    m.insert(Channel::Baidu, Arc::new(BaiduEngine));
    m.insert(Channel::Sogou, Arc::new(SogouEngine));
    m.insert(Channel::Iflytek, Arc::new(IflyEngine));
    m.insert(Channel::Chatterfly, Arc::new(ChatterflyEngine));
    m.insert(Channel::WetypeOffline, crate::wtoffline::engine());
    m
}
