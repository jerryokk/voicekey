//! Windows 上屏：剪贴板 + Ctrl+V，随后恢复原剪贴板。
//!
//! 不走逐字模拟，是因为中文长句用 SendInput 打太慢；剪贴板一条 Ctrl+V 就完事，
//! 代价是要在 500ms 后把用户原来的剪贴板还回去，且只在用户没改动过时才还。
//!
//! 还有一件事必须处理：Windows 的 UIPI 会**静默丢弃**低权限进程发往高权限窗口
//! （以管理员身份运行的窗口）的键鼠输入。表现是「识别成功、字一个也没进去」，
//! 用户只会觉得软件坏了。所以上屏前先比对前台窗口的权限，发现打不进去就直接
//! 把正文留在剪贴板里，并如实说明「请手动粘贴」。

use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// 一次上屏的结果，调用方据此决定要不要提示用户
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pasted {
    /// 已按下 Ctrl+V。系统不会告诉我们它是否真的落进了输入框
    Sent,
    /// 前台窗口权限更高，输入会被系统丢弃：正文留在剪贴板里，等用户手动粘贴
    Blocked,
    /// 剪贴板不可用，退化成逐字输入
    Typed,
}

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

/// 前台窗口是否高到我们打不进去。流式上屏受同一条限制，所以进入会话前就该问一次。
pub fn input_blocked() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return false;
        }
        elevated(pid) && !elevated(std::process::id())
    }
}

/// 某进程是否以管理员身份运行。查不到时按「不是」处理：
/// 错判的代价只是多试一次粘贴，比误报「打不进去」要好。
fn elevated(pid: u32) -> bool {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut token = HANDLE::default();
        let mut value = TOKEN_ELEVATION::default();
        let mut length = 0u32;
        let mut enabled = false;
        if OpenProcessToken(process, TOKEN_QUERY, &mut token).is_ok() {
            enabled = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut value as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut length,
            )
            .is_ok()
                && value.TokenIsElevated != 0;
            let _ = CloseHandle(token);
        }
        let _ = CloseHandle(process);
        enabled
    }
}

/// 借剪贴板 + Ctrl+V 上屏，随后恢复原剪贴板（文本/图片）
pub fn paste(text: &str) -> Pasted {
    let Ok(mut cb) = arboard::Clipboard::new() else {
        type_text(text);
        return Pasted::Typed;
    };
    let saved_text = cb.get_text().ok();
    let saved_image = if saved_text.is_none() { cb.get_image().ok() } else { None };
    if cb.set_text(text).is_err() {
        type_text(text);
        return Pasted::Typed;
    }
    // 高权限窗口：Ctrl+V 会被系统丢掉。这时剪贴板里必须留着正文，
    // 不能走「500ms 后恢复」，否则用户想手动粘贴时拿到的是旧内容。
    if input_blocked() {
        return Pasted::Blocked;
    }
    let seq = unsafe { GetClipboardSequenceNumber() };
    send(&[
        key_input(0x11, 0, KEYBD_EVENT_FLAGS(0)),
        key_input(0x56, 0, KEYBD_EVENT_FLAGS(0)),
        key_input(0x56, 0, KEYEVENTF_KEYUP),
        key_input(0x11, 0, KEYEVENTF_KEYUP),
    ]);
    if saved_text.is_none() && saved_image.is_none() {
        return Pasted::Sent;
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
    Pasted::Sent
}
