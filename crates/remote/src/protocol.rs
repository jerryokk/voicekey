//! 手机页与电脑之间的唯一契约。
//!
//! 两条通道各一份：
//! - WS（经隧道）：信令与会话控制，[`Op`] 上行、[`Out`] 下行
//! - DataChannel `input`（WebRTC 直连）：触摸板，[`Pad`] 上行
//!
//! 手机页是单文件内联脚本，刻意不引入构建工具，契约没法靠类型系统跨语言传递。
//! 这里集中定义枚举（改字段名会编译报错），再用 `page_agrees_with_protocol`
//! 和 `page.html` 对账——任何一边改名，`cargo test` 都会失败。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 手机 → 电脑（WS）
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum Op {
    /// 按下：开始一句话
    Start,
    /// 松手：进入定稿
    Stop,
    /// 上滑取消：这一句不要了，服务端丢弃音频、不跑识别、不上屏
    Cancel,
    /// 删除前台窗口里的一个字。连删由手机按节奏重复发
    Backspace,
    /// 选中某条候选上屏
    Commit { channel: String },
    /// WebRTC offer。id 由页面递增，answer 原样带回：重建直连时迟到的旧 answer 靠它认出来
    Offer { id: u32, sdp: String },
}

/// 电脑 → 手机（WS）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum Out {
    /// 连接建立，附带给浏览器的 ICE 服务器清单
    Hello { ice: &'static [&'static str] },
    /// 另一台设备正持有本句
    Busy,
    /// WebRTC answer，id 是对应 offer 的 id
    Answer { id: u32, sdp: String },
    /// 识别中的文字
    Partial { text: String },
    /// 定稿文字（空串表示没识别到）
    Final { text: String },
    /// 需要用户知道的一句话：失败原因、权限提示
    Notice { text: String },
    /// 出错，stage 说明是哪一段（asr / rtc / answer / session）
    Error { stage: &'static str, message: String },
    /// 多渠道候选快照
    Rows { model: Value },
    /// 某条候选不可选，抖一下
    Shake { row: usize },
    /// 电脑端主动断开（或二维码已失效），页面不要再重连
    Kicked,
}

impl Out {
    /// 枚举里只有字符串、数字和 `Value`，序列化不会失败
    pub(crate) fn json(&self) -> Value {
        serde_json::to_value(self).expect("Out 总能序列化")
    }
}

/// 触摸板 DataChannel 的 label。别的 label 一律关掉
pub const PAD_CHANNEL: &str = "input";

/// 鼠标按键
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// 手机 → 电脑（DataChannel `input`）：触摸板。
///
/// 位移一律是「电脑屏幕宽的比例」：x、y 都以光标所在显示器的宽为 1。
/// 两轴同一基准，斜着划在电脑上还是同一个角度（各按宽高算的话 16:9 屏上竖向会被压扁）。
/// 手机不知道也不需要知道电脑分辨率，换算交给电脑端做——
/// 这样同一划动在 1080p 和 4K 上走过的视觉距离一致。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "k", rename_all = "lowercase", deny_unknown_fields)]
pub enum Pad {
    /// 移动光标
    Move { x: f32, y: f32 },
    /// 按住（拖拽开始）
    Down { b: Button },
    /// 松开（拖拽结束）
    Up { b: Button },
    /// 单击：按下松开一次完成，双击由系统按两次单击的间隔判定
    Click { b: Button },
    /// 滚动，同样按屏幕比例，是手指位移（向下为正）。按自然滚动处理：内容跟着手指走
    Scroll { x: f32, y: f32 },
}

impl Pad {
    /// 拒绝 NaN、无穷和离谱的单帧位移：一帧走半个屏幕以上只可能是脚本出错
    pub fn sane(&self) -> bool {
        let ok = |v: f32| v.is_finite() && v.abs() <= 0.5;
        match *self {
            Pad::Move { x, y } | Pad::Scroll { x, y } => ok(x) && ok(y),
            Pad::Down { .. } | Pad::Up { .. } | Pad::Click { .. } => true,
        }
    }
}

/// 一条 `rows` 消息：候选模型由 controller 定义形状，这里只做透传
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

/// 一条 `shake` 消息
pub fn shake(row: usize) -> Value {
    Out::Shake { row }.json()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个变体各一条样本。match 没有通配分支：加了变体不补样本就编译不过，
    /// 对账测试从样本里取名字，名字表就不会和枚举漂移
    fn ops() -> Vec<(&'static str, Op)> {
        let all = vec![
            (r#"{"op":"start"}"#, Op::Start),
            (r#"{"op":"stop"}"#, Op::Stop),
            (r#"{"op":"cancel"}"#, Op::Cancel),
            (r#"{"op":"backspace"}"#, Op::Backspace),
            (r#"{"op":"commit","channel":"qwen"}"#, Op::Commit { channel: "qwen".into() }),
            (r#"{"op":"offer","id":3,"sdp":"v=0"}"#, Op::Offer { id: 3, sdp: "v=0".into() }),
        ];
        for (_, op) in &all {
            match op {
                Op::Start | Op::Stop | Op::Cancel | Op::Backspace | Op::Commit { .. } | Op::Offer { .. } => {}
            }
        }
        all
    }

    fn outs() -> Vec<Out> {
        let all = vec![
            Out::Hello { ice: &[] },
            Out::Busy,
            Out::Answer { id: 1, sdp: String::new() },
            Out::Partial { text: String::new() },
            Out::Final { text: String::new() },
            Out::Notice { text: String::new() },
            Out::Error { stage: "asr", message: String::new() },
            Out::Rows { model: Value::Null },
            Out::Shake { row: 0 },
            Out::Kicked,
        ];
        for out in &all {
            match out {
                Out::Hello { .. }
                | Out::Busy
                | Out::Answer { .. }
                | Out::Partial { .. }
                | Out::Final { .. }
                | Out::Notice { .. }
                | Out::Error { .. }
                | Out::Rows { .. }
                | Out::Shake { .. }
                | Out::Kicked => {}
            }
        }
        all
    }

    fn pads() -> Vec<(&'static str, Pad)> {
        let all = vec![
            (r#"{"k":"move","x":0.01,"y":-0.002}"#, Pad::Move { x: 0.01, y: -0.002 }),
            (r#"{"k":"down","b":"left"}"#, Pad::Down { b: Button::Left }),
            (r#"{"k":"up","b":"left"}"#, Pad::Up { b: Button::Left }),
            (r#"{"k":"click","b":"right"}"#, Pad::Click { b: Button::Right }),
            (r#"{"k":"click","b":"middle"}"#, Pad::Click { b: Button::Middle }),
            (r#"{"k":"scroll","x":0,"y":0.1}"#, Pad::Scroll { x: 0.0, y: 0.1 }),
        ];
        for (_, pad) in &all {
            match pad {
                Pad::Move { .. } | Pad::Down { .. } | Pad::Up { .. } | Pad::Click { .. } | Pad::Scroll { .. } => {}
            }
        }
        all
    }

    fn tag(json: &str, key: &str) -> String {
        serde_json::from_str::<Value>(json).unwrap()[key].as_str().unwrap().to_string()
    }

    #[test]
    fn parses_every_op() {
        for (text, expect) in ops() {
            assert_eq!(serde_json::from_str::<Op>(text).unwrap(), expect, "text: {text}");
        }
    }

    /// 拼错的 op、缺字段、多字段都要报错，而不是静默走进默认分支
    #[test]
    fn rejects_unknown_or_incomplete() {
        for text in [
            r#"{"op":"strat"}"#,
            r#"{"op":"commit"}"#,
            r#"{"op":"offer","sdp":"v=0"}"#,
            r#"{"op":"commit","channel":"qwen","x":1}"#,
            r#"{"type":"start"}"#,
            r#"{}"#,
        ] {
            assert!(serde_json::from_str::<Op>(text).is_err(), "应当拒绝：{text}");
        }
    }

    #[test]
    fn parses_pad_and_rejects_garbage() {
        for (text, expect) in pads() {
            let p: Pad = serde_json::from_str(text).unwrap();
            assert_eq!(p, expect, "text: {text}");
            assert!(p.sane());
        }
        assert!(!Pad::Move { x: 0.9, y: 0.0 }.sane());
        assert!(!Pad::Scroll { x: f32::NAN, y: 0.0 }.sane());
        for text in [r#"{"k":"click"}"#, r#"{"k":"click","b":"x1"}"#, r#"{"k":"move","x":1}"#] {
            assert!(serde_json::from_str::<Pad>(text).is_err(), "应当拒绝：{text}");
        }
    }

    #[test]
    fn helpers_fill_the_right_fields() {
        assert_eq!(fin("你好")["text"], "你好");
        assert_eq!(error("asr", "超时")["stage"], "asr");
        assert_eq!(shake(2)["row"], 2);
        assert_eq!(Out::Answer { id: 7, sdp: "s".into() }.json()["id"], 7);
    }

    /// 契约的两端必须用同一批名字：手机页是纯 JS，编译期抓不到，只能这样守
    #[test]
    fn page_agrees_with_protocol() {
        let page = include_str!("page.html");
        let mut names: Vec<String> = ops().iter().map(|(t, _)| tag(t, "op")).collect();
        names.extend(outs().iter().map(|o| o.json()["type"].as_str().unwrap().to_string()));
        for (t, _) in pads() {
            names.push(tag(t, "k"));
            if let Ok(v) = serde_json::from_str::<Value>(t) {
                if let Some(b) = v["b"].as_str() {
                    names.push(b.to_string());
                }
            }
        }
        names.push(PAD_CHANNEL.to_string());
        for name in names {
            assert!(page.contains(&format!("'{name}'")), "page.html 里找不到：{name}");
        }
    }
}
