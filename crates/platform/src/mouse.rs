//! 触摸板落地：屏幕比例 → 像素 → SendInput。
//!
//! 位移用绝对坐标注入（MOUSEEVENTF_ABSOLUTE | VIRTUALDESK），不用相对位移：
//! 相对位移会再过一遍系统的「提高指针精确度」曲线，手机端已经算过一次加速，
//! 叠两层就没法预测手感了。比例按光标**当前所在**的显示器换算，跨屏时自然切换。

use crate::win::send;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// 滚轮刻度：120 是一格。一格在多数程序里滚三行，约 48 像素
const WHEEL_PER_PIXEL: f32 = 120.0 / 48.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    fn flags(self) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
        match self {
            Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        }
    }
}

/// 一台虚拟鼠标，几台手机共用。
///
/// 记着小数余量：手机一帧常常只移动零点几个像素，直接取整会让慢速移动完全不动。
/// 也记着每个键是被哪台设备按住的（0 表示没按）：一台设备断开时只放开它自己按的键，
/// 不会把另一台正在拖拽的键一起松掉。
#[derive(Default)]
pub struct Pointer {
    rest: (f32, f32),
    wheel: (f32, f32),
    /// 按 [`Button`] 的声明顺序：左、右、中
    held: [u64; 3],
}

fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

/// 光标位置与它所在显示器的像素宽度
fn here() -> Result<(POINT, f32), String> {
    unsafe {
        let mut at = POINT::default();
        GetCursorPos(&mut at).map_err(|e| format!("取不到光标位置：{e}"))?;
        let monitor = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Err("取不到显示器信息".into());
        }
        let r = info.rcMonitor;
        Ok((at, (r.right - r.left) as f32))
    }
}

/// 把余量四舍五入取出整数，差值留下。用 round 不用 trunc：浮点累加常停在 2.9999，
/// trunc 会让那一像素永远差一点到不了
fn take(rest: &mut f32, add: f32) -> i32 {
    *rest += add;
    let whole = rest.round();
    *rest -= whole;
    whole as i32
}

/// 虚拟桌面内的像素偏移 → SendInput 的 0..=65535 绝对坐标。
/// 系统按 `n * size / 65536` 向下取整回到像素，所以这里要向上取整，否则会差一像素
fn normalize(offset: i32, size: i32) -> i32 {
    ((offset as u64 * 65536).div_ceil(size as u64)) as i32
}

impl Pointer {
    /// 按屏幕比例移动：x、y 都以光标所在显示器的宽为 1，两轴同一基准才不会把斜线压扁
    pub fn move_by(&mut self, x: f32, y: f32) -> Result<(), String> {
        let (at, w) = here()?;
        let dx = take(&mut self.rest.0, x * w);
        let dy = take(&mut self.rest.1, y * w);
        if dx == 0 && dy == 0 {
            return Ok(());
        }
        // 绝对坐标归一到整个虚拟桌面（多屏拼起来的大矩形）
        let (vx, vy, vw, vh) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        let tx = (at.x + dx).clamp(vx, vx + vw - 1);
        let ty = (at.y + dy).clamp(vy, vy + vh - 1);
        let flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        send(&[mouse(normalize(tx - vx, vw), normalize(ty - vy, vh), 0, flags)])
    }

    /// `who` 按下一个键。已经按着就不再发，免得系统收到两次按下
    pub fn down(&mut self, who: u64, b: Button) -> Result<(), String> {
        if self.held[b as usize] != 0 {
            return Ok(());
        }
        send(&[mouse(0, 0, 0, b.flags().0)])?;
        self.held[b as usize] = who;
        Ok(())
    }

    /// `who` 松开它按住的键。别的设备按住的不管
    pub fn up(&mut self, who: u64, b: Button) -> Result<(), String> {
        if self.held[b as usize] != who {
            return Ok(());
        }
        self.held[b as usize] = 0;
        send(&[mouse(0, 0, 0, b.flags().1)])
    }

    /// 单击。这个键正被按住（谁按的都一样）就先放开再点
    pub fn click(&mut self, b: Button) -> Result<(), String> {
        let (down, up) = b.flags();
        let mut inputs = Vec::with_capacity(3);
        if std::mem::take(&mut self.held[b as usize]) != 0 {
            inputs.push(mouse(0, 0, 0, up));
        }
        inputs.extend([mouse(0, 0, 0, down), mouse(0, 0, 0, up)]);
        send(&inputs)
    }

    /// 自然滚动，比例同 [`move_by`](Self::move_by)：手指往下拖，内容跟着往下走
    pub fn scroll(&mut self, x: f32, y: f32) -> Result<(), String> {
        let (_, w) = here()?;
        let v = take(&mut self.wheel.1, y * w * WHEEL_PER_PIXEL);
        let hz = take(&mut self.wheel.0, -x * w * WHEEL_PER_PIXEL);
        let mut inputs = Vec::with_capacity(2);
        if v != 0 {
            inputs.push(mouse(0, 0, v, MOUSEEVENTF_WHEEL));
        }
        if hz != 0 {
            inputs.push(mouse(0, 0, hz, MOUSEEVENTF_HWHEEL));
        }
        send(&inputs)
    }

    /// `who` 走了：放开它按住的键，否则电脑上会一直处于拖拽状态
    pub fn release(&mut self, who: u64) -> Result<(), String> {
        for b in [Button::Left, Button::Right, Button::Middle] {
            self.up(who, b)?;
        }
        Ok(())
    }

    /// 所有设备都走了（服务重启）：谁按的键都放开
    pub fn release_all(&mut self) -> Result<(), String> {
        for who in self.held {
            if who != 0 {
                self.release(who)?;
            }
        }
        self.rest = (0.0, 0.0);
        self.wheel = (0.0, 0.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize, take};

    /// 慢速移动靠余量累积，十次 0.3 像素要走出 3 像素
    #[test]
    fn remainder_accumulates() {
        let mut rest = 0.0;
        let moved: i32 = (0..10).map(|_| take(&mut rest, 0.3)).sum();
        assert_eq!(moved, 3);
        let back: i32 = (0..10).map(|_| take(&mut rest, -0.3)).sum();
        assert_eq!(back, -3);
    }

    /// 每个像素归一化后再按系统的规则换回来，必须落回同一个像素，且不越出 0..=65535
    #[test]
    fn absolute_coordinates_roundtrip() {
        for size in [1024, 1080, 1920, 2560, 3840, 5760, 7680] {
            for p in 0..size {
                let n = normalize(p, size);
                assert!((0..=65535).contains(&n), "size {size} p {p} → {n}");
                assert_eq!((n as i64 * size as i64 / 65536) as i32, p, "size {size} p {p}");
            }
        }
    }
}
