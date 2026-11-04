//! 用户设置：渠道、输出方式、触摸板速度、外观，存在数据目录的 settings.json。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use voicekey_core::util::data_file;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
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
    /// 显示名，手机上的候选行用它
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
    /// 不该默认参与对比。默认值与「勾选太少时的修正」都用这一份，避免两处定义漂移。
    pub fn default_multi() -> Vec<Channel> {
        Self::engines().into_iter().filter(|&c| c != Self::WetypeOffline).collect()
    }

    pub fn available() -> Vec<Channel> {
        let mut v = Self::engines();
        v.push(Channel::All);
        v
    }
}

/// 触摸板速度的取值范围：倍率乘在手机端算好的位移上
pub const POINTER_SPEED: (f32, f32) = (0.4, 2.5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// 配置文件与 1.0.0 的结构一一对应：缺字段、多字段都按损坏处理，整份回到默认
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    pub theme: Theme,
    /// 触摸板速度倍率
    pub pointer_speed: f32,
}

/// 界面能改的那几项，只带改了的字段。
/// 上屏历史（`last_pick`、`pick_counts`）由会话自己记，界面拿着旧快照整份写回会把它冲掉
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Patch {
    channel: Option<Channel>,
    streaming: Option<bool>,
    live_text: Option<bool>,
    multi: Option<Vec<Channel>>,
    autostart: Option<bool>,
    theme: Option<Theme>,
    pointer_speed: Option<f32>,
}

impl Patch {
    pub fn apply(self, s: &mut Settings) {
        let Patch { channel, streaming, live_text, multi, autostart, theme, pointer_speed } = self;
        s.channel = channel.unwrap_or(s.channel);
        s.streaming = streaming.unwrap_or(s.streaming);
        s.live_text = live_text.unwrap_or(s.live_text);
        s.multi = multi.unwrap_or_else(|| std::mem::take(&mut s.multi));
        s.autostart = autostart.unwrap_or(s.autostart);
        s.theme = theme.unwrap_or(s.theme);
        s.pointer_speed = pointer_speed.unwrap_or(s.pointer_speed);
    }
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
            theme: Theme::System,
            pointer_speed: 1.0,
        }
    }
}

const FILE: &str = "settings.json";

impl Settings {
    /// 没有配置文件是首次启动，静默用默认；有但读不懂就记下原因再用默认
    pub fn load() -> Self {
        let path = data_file(FILE);
        let mut s = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                crate::log_warn!("设置文件无法解析，已恢复默认：{e}");
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                crate::log_warn!("设置文件读取失败，已恢复默认：{e}");
                Self::default()
            }
        };
        s.sanitize();
        s
    }

    pub fn save(&self) -> Result<(), String> {
        let b = serde_json::to_vec_pretty(self).expect("Settings 总能序列化");
        std::fs::write(data_file(FILE), b).map_err(|e| format!("设置保存失败：{e}"))
    }

    /// 界面或手改配置给出的越界值收回到合法范围。勾选不足两个按默认勾选算。
    /// JSON 表达不了 NaN 和无穷，速度只需要夹到范围内
    pub fn sanitize(&mut self) {
        let (lo, hi) = POINTER_SPEED;
        self.pointer_speed = self.pointer_speed.clamp(lo, hi);
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
    use serde_json::{json, Value};

    /// 以默认值为底，改几个字段再解析
    fn with(patch: Value) -> Settings {
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        for (k, x) in patch.as_object().unwrap() {
            v[k] = x.clone();
        }
        let mut s: Settings = serde_json::from_value(v).unwrap();
        s.sanitize();
        s
    }

    /// 不认旧格式：缺字段或带着不认识的字段都解析失败，load 会整份回到默认
    #[test]
    fn rejects_incomplete_or_foreign_config() {
        assert!(serde_json::from_str::<Settings>(r#"{"channel":"qwen","theme":"dark"}"#).is_err());
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        v["hotkey"] = json!("F8");
        assert!(serde_json::from_value::<Settings>(v).is_err());
    }

    #[test]
    fn roundtrips_default() {
        let s = Settings::default();
        let back: Settings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back.channel, s.channel);
        assert_eq!(back.multi, s.multi);
        assert_eq!(back.pointer_speed, 1.0);
    }

    /// 多渠道至少要两家，否则那个模式没有意义
    #[test]
    fn multi_with_one_channel_is_expanded() {
        let s = with(json!({ "multi": ["doubao"] }));
        assert_eq!(s.multi, Channel::default_multi());
        assert!(!s.multi.contains(&Channel::WetypeOffline), "不该顺手把要下载模型的渠道勾上");
    }

    /// 手改过的 multi 可能重复、乱序，甚至含「全部」这种不该出现在列表里的值
    #[test]
    fn multi_is_deduped_and_ordered() {
        let s = with(json!({ "multi": ["sogou", "doubao", "sogou", "all", "baidu"] }));
        assert_eq!(s.multi, vec![Channel::Doubao, Channel::Baidu, Channel::Sogou]);
    }

    #[test]
    fn pointer_speed_is_clamped() {
        assert_eq!(with(json!({ "pointerSpeed": 9.0 })).pointer_speed, POINTER_SPEED.1);
        assert_eq!(with(json!({ "pointerSpeed": 0.0 })).pointer_speed, POINTER_SPEED.0);
    }

    #[test]
    fn unknown_theme_is_rejected() {
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        v["theme"] = json!("neon");
        assert!(serde_json::from_value::<Settings>(v).is_err());
    }

    /// 补丁只动它带着的字段，上屏历史原样保留；界面不能借补丁改历史
    #[test]
    fn patch_touches_only_given_fields() {
        let mut s = Settings {
            last_pick: Some(Channel::Qwen),
            pick_counts: [(Channel::Qwen, 3)].into(),
            ..Default::default()
        };
        let p: Patch = serde_json::from_value(json!({ "streaming": false, "theme": "dark" })).unwrap();
        p.apply(&mut s);
        assert!(!s.streaming);
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.channel, Channel::Doubao);
        assert_eq!(s.last_pick, Some(Channel::Qwen));
        assert_eq!(s.pick_counts[&Channel::Qwen], 3);
        assert!(serde_json::from_value::<Patch>(json!({ "pickCounts": {} })).is_err());
    }

    /// sanitize 必须幂等：设置会在每次启动和每次改动后跑一遍
    #[test]
    fn sanitize_is_idempotent() {
        let mut s = Settings::default();
        s.sanitize();
        let once = s.clone();
        s.sanitize();
        assert_eq!(s.channel, once.channel);
        assert_eq!(s.multi, once.multi);
        assert_eq!(s.pointer_speed, once.pointer_speed);
    }

    /// 「全部」是合法的当前渠道（多渠道模式的入口）
    #[test]
    fn all_is_a_valid_current_channel() {
        assert_eq!(with(json!({ "channel": "all" })).channel, Channel::All);
    }
}
