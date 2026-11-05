//! 资源管理器定位文件、写剪贴板，以及不抢焦点地显示 / 隐藏我们自己的窗口。

use std::path::{Path, PathBuf};
use windows::core::HSTRING;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};

/// 在资源管理器里打开这些文件所在的文件夹并选中它们。
///
/// 不拼 `explorer /select,` 命令行：那条命令对带逗号、空格的路径解析不可靠，
/// 一次也只能选中一个。文件必须在同一个文件夹里（收件都在下载目录，满足）
pub fn reveal(files: &[PathBuf]) -> Result<(), String> {
    let Some(first) = files.first() else { return Ok(()) };
    let folder = first.parent().ok_or("文件没有上级目录")?.to_path_buf();
    let files: Vec<PathBuf> = files.iter().filter(|f| f.parent() == Some(&folder) && f.exists()).cloned().collect();
    // 文件都不在了（被删、被挪）：直接打开文件夹。SHOpenFolderAndSelectItems 不给子项时会打开上一级
    if files.is_empty() {
        return std::process::Command::new("explorer").arg(&folder).spawn().map(drop).map_err(|e| format!("打开文件夹失败：{e}"));
    }
    // Shell 要单线程套间。调用方可能在任意线程（异步运行时的工作线程），自己开一个
    std::thread::spawn(move || unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let r = open(&folder, &files);
        if com {
            CoUninitialize();
        }
        r
    })
    .join()
    .unwrap_or_else(|_| Err("打开文件夹时出错".into()))
}

unsafe fn open(folder: &Path, files: &[PathBuf]) -> Result<(), String> {
    let pidl = |p: &Path| unsafe { ILCreateFromPathW(&HSTRING::from(p.as_os_str())) };
    let dir = pidl(folder);
    if dir.is_null() {
        return Err(format!("找不到文件夹：{}", folder.display()));
    }
    let items: Vec<*mut ITEMIDLIST> = files.iter().map(|f| pidl(f)).filter(|p| !p.is_null()).collect();
    let selected: Vec<*const ITEMIDLIST> = items.iter().map(|&p| p as *const _).collect();
    let r = unsafe { SHOpenFolderAndSelectItems(dir, Some(&selected[..]), 0) };
    for p in items {
        unsafe { ILFree(Some(p)) };
    }
    unsafe { ILFree(Some(dir)) };
    r.map_err(|e| format!("打开文件夹失败：{}", e.message()))
}

/// 显示窗口但不激活它：用户正在别处打字时，弹出来的面板不能把焦点拿走。
/// 置顶也一并用 `SWP_NOACTIVATE` 做，否则改 Z 序那一下同样会激活
pub fn show_no_activate(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE,
    };
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

pub fn hide_window(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    unsafe {
        let _ = ShowWindow(HWND(hwnd as *mut _), SW_HIDE);
    }
}

pub fn window_visible(hwnd: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    unsafe { IsWindowVisible(HWND(hwnd as *mut _)).as_bool() }
}

/// 当前的前台窗口（句柄值）
pub fn foreground() -> isize {
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0 as isize }
}

/// 把焦点还给某个窗口。只在我们自己是前台时调用（用户刚点了我们的浮层），系统才允许
pub fn set_foreground(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        if hwnd.is_invalid() || !IsWindow(Some(hwnd)).as_bool() {
            return;
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// 把文字写进剪贴板，不做任何恢复：这正是用户想要的结果
pub fn set_clipboard(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut cb| cb.set_text(text))
        .map_err(|e| format!("写剪贴板失败：{e}"))
}
