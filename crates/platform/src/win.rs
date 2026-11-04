//! Windows 键盘注入：一次性上屏走剪贴板 + Ctrl+V，边说边上屏走逐字 Unicode 键入。
//!
//! 定稿整句走剪贴板：一条 Ctrl+V 就完事，不受长句和输入法状态影响，
//! 代价是要在 500ms 后把用户原来的剪贴板还回去，且只在用户没改动过时才还。
//! 流式上屏每次只补打几个字，用 [`type_chars`] 直接发 Unicode 键，不碰剪贴板。
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
    VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// 一次上屏的结果，调用方据此决定要不要提示用户
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pasted {
    /// 已按下 Ctrl+V。系统不会告诉我们它是否真的落进了输入框
    Sent,
    /// 前台窗口权限更高，输入会被系统丢弃：正文留在剪贴板里，等用户手动粘贴
    Blocked,
}

const PRESS: KEYBD_EVENT_FLAGS = KEYBD_EVENT_FLAGS(0);

fn key_input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

/// 注入一串输入，返回系统实际收下的条数。少于请求数说明被系统挡下了（UIPI、安全桌面），
/// 此时错误原因在线程错误码里，必须紧接着取
fn inject(inputs: &[INPUT]) -> Result<(), (usize, String)> {
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent == inputs.len() {
        Ok(())
    } else {
        Err((sent, format!("系统拦下了模拟输入（{sent}/{}）：{}", inputs.len(), windows::core::Error::from_thread())))
    }
}

pub(crate) fn send(inputs: &[INPUT]) -> Result<(), String> {
    inject(inputs).map_err(|(_, e)| e)
}

/// 注入若干按键，每组是一个字符的按下与松开。返回已落地的组数：
/// 字符在它最后一个按下事件送达时落地，只差末尾的松开也算
fn send_keys(keys: Vec<Vec<INPUT>>) -> (usize, Result<(), String>) {
    let ends: Vec<usize> = keys.iter().scan(0, |end, k| Some(*end + k.len())).collect();
    let inputs: Vec<INPUT> = keys.into_iter().flatten().collect();
    match inject(&inputs) {
        Ok(()) => (ends.len(), Ok(())),
        Err((sent, e)) => (ends.iter().take_while(|&&end| end - 1 <= sent).count(), Err(e)),
    }
}

/// 退格 n 次，返回实际删掉的字数
pub(crate) fn erase(n: usize) -> (usize, Result<(), String>) {
    send_keys((0..n).map(|_| vec![key_input(VK_BACK, 0, PRESS), key_input(VK_BACK, 0, KEYEVENTF_KEYUP)]).collect())
}

pub fn backspace(n: usize) -> Result<(), String> {
    erase(n).1
}

/// 逐字发 Unicode 键，返回实际打出的字数
pub(crate) fn type_chars(chars: &[char]) -> (usize, Result<(), String>) {
    let none = VIRTUAL_KEY(0);
    send_keys(
        chars
            .iter()
            .map(|c| {
                c.encode_utf16(&mut [0; 2])
                    .iter()
                    .flat_map(|&u| [key_input(none, u, KEYEVENTF_UNICODE), key_input(none, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)])
                    .collect()
            })
            .collect(),
    )
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

/// 某进程是否以管理员身份运行。查不到时按「不是」处理：拿不到令牌的多半是
/// 系统保护进程（它们本来也不是输入目标），错判的代价只是多试一次粘贴。
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
pub fn paste(text: &str) -> Result<Pasted, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用：{e}"))?;
    let saved_text = cb.get_text().ok();
    let saved_image = if saved_text.is_none() { cb.get_image().ok() } else { None };
    cb.set_text(text).map_err(|e| format!("写剪贴板失败：{e}"))?;
    // 高权限窗口：Ctrl+V 会被系统丢掉。这时剪贴板里必须留着正文，
    // 不能走「500ms 后恢复」，否则用户想手动粘贴时拿到的是旧内容。
    if input_blocked() {
        return Ok(Pasted::Blocked);
    }
    let seq = unsafe { GetClipboardSequenceNumber() };
    send(&[
        key_input(VK_CONTROL, 0, PRESS),
        key_input(VK_V, 0, PRESS),
        key_input(VK_V, 0, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ])?;
    if saved_text.is_none() && saved_image.is_none() {
        return Ok(Pasted::Sent);
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
    Ok(Pasted::Sent)
}
