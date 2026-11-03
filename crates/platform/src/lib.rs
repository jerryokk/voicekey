//! 平台层：把文字打进当前前台窗口。
//!
//! 只剩这一件事了。快捷键监听、光标定位、候选面板都随手机端接管而删除——
//! 现在不需要知道光标在哪，也不需要抢焦点，往当前前台窗口粘就行。

#[cfg(windows)]
mod win;

#[cfg(windows)]
use win as imp;

pub use imp::{backspace, input_blocked, paste, Pasted};

/// 边说边上屏：与已打出的文本比对，退格删掉分歧部分再补打新内容（不经剪贴板）
#[derive(Default)]
pub struct Typer {
    typed: Vec<char>,
    done: bool,
}

impl Typer {
    pub fn update(&mut self, text: &str) {
        if self.done {
            return;
        }
        let next: Vec<char> = text.chars().collect();
        let common = self.typed.iter().zip(&next).take_while(|(a, b)| a == b).count();
        imp::backspace(self.typed.len() - common);
        let tail: String = next[common..].iter().collect();
        if !tail.is_empty() {
            imp::type_text(&tail);
        }
        self.typed = next;
    }

    pub fn finish(&mut self) {
        self.done = true;
    }

    /// 放弃这一句：把已经打出去的字退回来。
    /// 用户上滑取消时，输入框里不该留下半句。
    pub fn discard(&mut self) {
        if self.done {
            return;
        }
        imp::backspace(self.typed.len());
        self.typed.clear();
        self.done = true;
    }
}
