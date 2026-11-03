//! WS 协议：手机页与电脑之间的唯一契约。
//!
//! 手机页是单文件内联脚本，刻意不引入构建工具（它要能从任意地址直接加载，
//! 多一层打包就多一处出错的地方），所以契约没法靠类型系统跨语言传递。
//! 这里的做法是：Rust 侧集中定义一份枚举（改字段名会编译报错），再用
//! `page_agrees_with_protocol` 这个测试和 `page.html` 对账——任何一边改名，
//! `cargo test` 都会失败，而不是等到用户按住说话才发现没反应。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 手机 → 电脑
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    /// 按下：开始一句话
    Start,
    /// 松手：进入定稿
    Stop,
    /// 上滑取消：这一句不要了，服务端丢弃音频、不跑识别、不上屏
    Cancel,
    /// 删除前台窗口里的字。上限由服务端再夹一次，挡住误触发的批量请求
    Backspace {
        #[serde(default = "one")]
        n: usize,
    },
    /// 选中某条候选上屏
    Commit { channel: String },
    /// WebRTC offer，正文是 SDP
    Offer { sdp: String },
}

impl Op {
    /// 全部 `op` 取值，对账测试要用
    pub const NAMES: &'static [&'static str] = &["start", "stop", "cancel", "backspace", "commit", "offer"];
}

fn one() -> usize {
    1
}

/// 电脑 → 手机
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Out {
    /// 连接建立，附带给浏览器的 ICE 服务器清单
    Hello { ice: &'static [&'static str] },
    /// 本句已受理
    Started,
    /// 另一台设备正持有本句
    Busy,
    /// WebRTC answer
    Answer { sdp: String },
    /// 识别中的文字
    Partial { text: String },
    /// 定稿文字（空串表示没识别到）
    Final { text: String },
    /// 需要用户知道的一句话：失败原因、权限提示、通道切换说明
    Notice { text: String },
    /// 出错，stage 说明是哪一段（asr / rtc / answer / session）
    Error { stage: &'static str, message: String },
    /// 多渠道候选快照
    Rows { model: Value },
    /// 某条候选不可选，抖一下
    Shake { row: usize },
    /// 建议改走回落通道
    Prefer { transport: &'static str, reason: String },
    /// 本句链路统计
    Stats { packets: u64, seconds: f64, loss: f64, state: String },
    /// 电脑端主动断开，页面不要再重连
    Kicked,
}

impl Out {
    pub fn json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// 全部 `type` 取值，对账测试要用
    pub const NAMES: &'static [&'static str] = &[
        "hello", "started", "busy", "answer", "partial", "final", "notice", "error", "rows", "shake", "prefer",
        "stats", "kicked",
    ];
}

/// 一条 `rows` 消息：候选模型是前端定义的形状，这里只做透传
pub fn rows(model: Value) -> Value {
    Out::Rows { model }.json()
}

/// 一条 `error` 消息
pub fn error(stage: &'static str, message: impl Into<String>) -> Value {
    Out::Error { stage, message: message.into() }.json()
}

/// 一条 `notice` 消息
pub fn notice(text: impl Into<String>) -> Value {
    Out::Notice { text: text.into() }.json()
}

/// 一条 `partial` 消息
pub fn partial(text: impl Into<String>) -> Value {
    Out::Partial { text: text.into() }.json()
}

/// 一条 `final` 消息
pub fn fin(text: impl Into<String>) -> Value {
    Out::Final { text: text.into() }.json()
}

/// 一条 `prefer` 消息
pub fn prefer(reason: impl Into<String>) -> Value {
    Out::Prefer { transport: "pcm", reason: reason.into() }.json()
}

/// 一条 `shake` 消息
pub fn shake(row: usize) -> Value {
    json!(Out::Shake { row })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_op() {
        for (text, expect) in [
            (r#"{"op":"start"}"#, Op::Start),
            (r#"{"op":"stop"}"#, Op::Stop),
            (r#"{"op":"cancel"}"#, Op::Cancel),
            (r#"{"op":"backspace"}"#, Op::Backspace { n: 1 }),
            (r#"{"op":"backspace","n":3}"#, Op::Backspace { n: 3 }),
            (r#"{"op":"commit","channel":"qwen"}"#, Op::Commit { channel: "qwen".into() }),
            (r#"{"op":"offer","sdp":"v=0"}"#, Op::Offer { sdp: "v=0".into() }),
        ] {
            assert_eq!(serde_json::from_str::<Op>(text).unwrap(), expect, "text: {text}");
        }
    }

    /// 拼错的 op 与缺字段都要报错，而不是像以前那样静默走进 `_ => {}`
    #[test]
    fn rejects_unknown_or_incomplete() {
        for text in [
            r#"{"op":"strat"}"#,
            r#"{"op":"commit"}"#,
            r#"{"op":"offer"}"#,
            r#"{"type":"start"}"#,
            r#"{}"#,
        ] {
            assert!(serde_json::from_str::<Op>(text).is_err(), "应当拒绝：{text}");
        }
    }

    #[test]
    fn emits_matching_type_tags() {
        assert_eq!(Out::Started.json()["type"], "started");
        assert_eq!(fin("你好")["text"], "你好");
        assert_eq!(error("asr", "超时")["stage"], "asr");
        assert_eq!(prefer("丢包 8.0%")["transport"], "pcm");
    }

    /// 契约的两端必须用同一批名字：手机页是纯 JS，编译期抓不到，只能这样守
    #[test]
    fn page_agrees_with_protocol() {
        let page = include_str!("page.html");
        for name in Op::NAMES {
            assert!(page.contains(&format!("'{name}'")), "page.html 里找不到 op: {name}");
        }
        for name in Out::NAMES {
            assert!(page.contains(&format!("'{name}'")), "page.html 里找不到 type: {name}");
        }
    }
}
