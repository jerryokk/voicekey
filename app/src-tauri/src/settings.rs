use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use voicekey_core::util::data_file;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Doubao,
    Wetype,
    Qwen,
    Baidu,
    Sogou,
    Iflytek,
    Chatterfly,
    WetypeOffline,
    /// 多渠道：勾选的渠道同时识别，手机上挑一条上屏
    All,
}

impl Channel {
    pub fn title(self) -> &'static str {
        match self {
            Channel::Doubao => "豆包输入法",
            Channel::Wetype => "微信输入法",
            Channel::Qwen => "千问输入法",
            Channel::Baidu => "百度输入法",
            Channel::Sogou => "搜狗输入法",
            Channel::Iflytek => "讯飞输入法",
            Channel::Chatterfly => "Chatterfly",
            Channel::WetypeOffline => "微信离线",
            Channel::All => "多渠道（说完挑选）",
        }
    }

    /// 可用的识别渠道（不含「全部」）
    pub fn engines() -> Vec<Channel> {
        vec![
            Channel::Doubao,
            Channel::Wetype,
            Channel::Qwen,
            Channel::Baidu,
            Channel::Sogou,
            Channel::Iflytek,
            Channel::Chatterfly,
            Channel::WetypeOffline,
        ]
    }

    pub fn available() -> Vec<Channel> {
        let mut v = Self::engines();
        v.push(Channel::All);
        v
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub channel: Channel,
    /// 边说边把字打进当前输入框
    pub streaming: bool,
    /// 浮层里显示识别中的文字
    pub live_text: bool,
    /// 多渠道模式下同时识别的渠道（至少 2 个）
    pub multi: Vec<Channel>,
    /// 候选里上次选中的渠道，下次默认聚焦
    pub last_pick: Option<Channel>,
    /// 各渠道上屏次数，次数多的排前
    pub pick_counts: HashMap<Channel, u32>,
    pub autostart: bool,
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            channel: Channel::Doubao,
            streaming: true,
            live_text: true,
            multi: Channel::engines().into_iter().filter(|&c| c != Channel::WetypeOffline).collect(),
            last_pick: None,
            pick_counts: HashMap::new(),
            autostart: false,
            theme: "system".into(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Settings = std::fs::read(data_file("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        s.sanitize();
        s
    }

    pub fn save(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(data_file("settings.json"), b);
        }
    }

    /// 不可用的渠道回落默认，勾选不足两个按全部渠道算
    pub fn sanitize(&mut self) {
        if !Channel::available().contains(&self.channel) {
            self.channel = Channel::Doubao;
        }
        let engines = Channel::engines();
        let mut multi: Vec<Channel> = engines.iter().copied().filter(|c| self.multi.contains(c)).collect();
        if multi.len() < 2 {
            multi = engines;
        }
        self.multi = multi;
    }
}
