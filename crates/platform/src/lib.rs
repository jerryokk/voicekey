//! 平台层：把文字打进当前前台窗口，把手机触摸板变成鼠标。
//!
//! 只支持 Windows：键鼠注入、UIPI 判定、显示器换算都是 Win32 的事。

mod mouse;
mod win;

pub use mouse::{Button, Pointer};
pub use win::{backspace, input_blocked, paste, Pasted};

/// 边说边上屏：与已打出的文本比对，退格删掉分歧部分再补打新内容（不经剪贴板）
#[derive(Default)]
pub struct Typer {
    typed: Vec<char>,
}

impl Typer {
    /// 让输入框里的内容变成 `text`。注入只成功一部分时按实际落地的键记账，
    /// 下一次比对以屏幕上真有的字为准
    pub fn update(&mut self, text: &str) -> Result<(), String> {
        let next: Vec<char> = text.chars().collect();
        let common = self.typed.iter().zip(&next).take_while(|(a, b)| a == b).count();
        let (erased, r) = win::erase(self.typed.len() - common);
        self.typed.truncate(self.typed.len() - erased);
        r?;
        let (typed, r) = win::type_chars(&next[common..]);
        self.typed.extend_from_slice(&next[common..common + typed]);
        r
    }

    /// 放弃这一句：把已经打出去的字退回来。
    /// 用户上滑取消时，输入框里不该留下半句。
    pub fn discard(self) -> Result<(), String> {
        win::backspace(self.typed.len())
    }
}
