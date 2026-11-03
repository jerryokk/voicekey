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

    /// 多渠道默认勾选的渠道：**不含**微信离线——它要先下 100MB 模型，
    /// 不该默认参与对比。默认值与「列表太短时的兜底」都用这一份，避免两处定义漂移。
    pub fn default_multi() -> Vec<Channel> {
        Self::engines().into_iter().filter(|&c| c != Self::WetypeOffline).collect()
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
            multi: Channel::default_multi(),
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
            multi = Channel::default_multi();
        }
        self.multi = multi;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧版本的配置文件缺新字段时，必须还能读出来。`serde(default)` +
    /// `sanitize()` 是这条兼容性的全部实现，所以要有测试钉住。
    #[test]
    fn legacy_config_missing_fields_still_loads() {
        let mut s: Settings = serde_json::from_str(r#"{"channel":"qwen","theme":"dark"}"#).unwrap();
        s.sanitize();
        assert_eq!(s.channel, Channel::Qwen);
        assert_eq!(s.theme, "dark");
        // 没写的字段取默认：流式与实时文字都开
        assert!(s.streaming);
        assert!(s.live_text);
        // 多渠道没配过时取默认勾选（不含要下载模型的微信离线）
        assert_eq!(s.multi, Channel::default_multi());
    }

    /// 多渠道至少要两家，否则那个模式没有意义
    #[test]
    fn multi_with_one_channel_is_expanded() {
        let mut s: Settings = serde_json::from_str(r#"{"multi":["doubao"]}"#).unwrap();
        s.sanitize();
        assert_eq!(s.multi, Channel::default_multi());
        assert!(!s.multi.contains(&Channel::WetypeOffline), "不该顺手把要下载模型的渠道勾上");
    }

    /// 手改过的 multi 可能重复、乱序，甚至含「全部」这种不该出现在列表里的值
    #[test]
    fn multi_is_deduped_and_ordered() {
        let mut s: Settings =
            serde_json::from_str(r#"{"multi":["sogou","doubao","sogou","all","baidu"]}"#).unwrap();
        s.sanitize();
        assert_eq!(s.multi, vec![Channel::Doubao, Channel::Baidu, Channel::Sogou]);
    }

    /// sanitize 必须幂等：设置会在每次启动和每次改动后跑一遍，
    /// 非幂等会让「读出来—存回去」的循环里值越滚越偏。
    #[test]
    fn sanitize_is_idempotent() {
        let mut s = Settings::default();
        s.sanitize();
        let once = s.clone();
        s.sanitize();
        assert_eq!(s.channel, once.channel);
        assert_eq!(s.multi, once.multi);
    }

    /// 「全部」是合法的当前渠道（多渠道模式的入口），不该被当成非法值回落
    #[test]
    fn all_is_a_valid_current_channel() {
        let mut s: Settings = serde_json::from_str(r#"{"channel":"all"}"#).unwrap();
        s.sanitize();
        assert_eq!(s.channel, Channel::All);
    }
}
