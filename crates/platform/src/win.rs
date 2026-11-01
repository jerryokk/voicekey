//! Windows 上屏：剪贴板 + Ctrl+V，随后恢复原剪贴板。
//!
//! 不走逐字模拟，是因为中文长句用 SendInput 打太慢；剪贴板一条 Ctrl+V 就完事，
//! 代价是要在 500ms 后把用户原来的剪贴板还回去，且只在用户没改动过时才还。

use std::time::Duration;
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    VIRTUAL_KEY,
};

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn send(inputs: &[INPUT]) {
    if !inputs.is_empty() {
        unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    }
}

pub fn backspace(n: usize) {
    let inputs: Vec<INPUT> = (0..n)
        .flat_map(|_| [key_input(0x08, 0, KEYBD_EVENT_FLAGS(0)), key_input(0x08, 0, KEYEVENTF_KEYUP)])
        .collect();
    send(&inputs);
}

pub(crate) fn type_text(text: &str) {
    let inputs: Vec<INPUT> = text
        .encode_utf16()
        .flat_map(|u| [key_input(0, u, KEYEVENTF_UNICODE), key_input(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)])
        .collect();
    send(&inputs);
}

/// 借剪贴板 + Ctrl+V 上屏，随后恢复原剪贴板（文本/图片）
pub fn paste(text: &str) {
    let Ok(mut cb) = arboard::Clipboard::new() else {
        type_text(text);
        return;
    };
    let saved_text = cb.get_text().ok();
    let saved_image = if saved_text.is_none() { cb.get_image().ok() } else { None };
    if cb.set_text(text).is_err() {
        type_text(text);
        return;
    }
    let seq = unsafe { GetClipboardSequenceNumber() };
    send(&[
        key_input(0x11, 0, KEYBD_EVENT_FLAGS(0)),
        key_input(0x56, 0, KEYBD_EVENT_FLAGS(0)),
        key_input(0x56, 0, KEYEVENTF_KEYUP),
        key_input(0x11, 0, KEYEVENTF_KEYUP),
    ]);
    if saved_text.is_none() && saved_image.is_none() {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        // 用户在这 500ms 里自己复制了别的东西，就别覆盖回去
        if unsafe { GetClipboardSequenceNumber() } != seq {
            return;
        }
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Some(t) = saved_text {
                let _ = cb.set_text(t);
            } else if let Some(i) = saved_image {
                let _ = cb.set_image(i);
            }
        }
    });
}
