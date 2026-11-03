//! 文件日志。
//!
//! 改成 Windows GUI 子系统后没有控制台，stderr 无从查看；出问题只能靠用户描述。
//! 这里把关键事件写进 `%LOCALAPPDATA%\VoiceKey\logs\voicekey-YYYYMMDD.log`，
//! 按天切分、只保留 7 天，量很小。
//!
//! 只记录事件与原因，不记录 token（URL 里带 token，日志里一律只打 origin）。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn init() {
    let path = dir().map(|d| {
        let _ = fs::create_dir_all(&d);
        d.join(format!("voicekey-{}.log", date_stamp(now_secs())))
    });
    if let Some(p) = &path {
        prune(&p.parent().map(PathBuf::from));
    }
    let _ = LOG_PATH.set(path);
    write("info", "启动");
}

fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(d).join("VoiceKey").join("logs"));
    }
    std::env::current_exe().ok()?.parent().map(|d| d.join("logs"))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 只保留最近 7 个日志文件
fn prune(dir: &Option<PathBuf>) {
    let Some(dir) = dir else { return };
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut files: Vec<_> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("voicekey-"))
        .collect();
    if files.len() <= 7 {
        return;
    }
    files.sort_by_key(|e| e.file_name());
    let drop_n = files.len() - 7;
    for e in files.into_iter().take(drop_n) {
        let _ = fs::remove_file(e.path());
    }
}

pub fn write(level: &str, message: &str) {
    let Some(Some(path)) = LOG_PATH.get() else { return };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) else { return };
    let _ = writeln!(f, "[{}] {:5} {}", clock(now_secs()), level.to_uppercase(), message);
}

fn date_stamp(secs: u64) -> String {
    let (y, m, d) = ymd(secs);
    format!("{y:04}{m:02}{d:02}")
}

fn clock(secs: u64) -> String {
    let (y, mo, d) = ymd(secs);
    let rem = secs % 86_400;
    format!("{y:04}-{mo:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// epoch 秒 → 年月日（UTC；日志不需要本地时区）
fn ymd(secs: u64) -> (i64, u32, u32) {
    let days = (secs / 86_400) as i64;
    // Howard Hinnant 的 civil_from_days
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::log::write("info", &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::log::write("warn", &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::log::write("error", &format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::date_stamp;

    /// 日期换算是手写的 Howard Hinnant civil_from_days。写错不只是文件名难看：
    /// 按名字排序裁剪旧文件时会误删当天的日志，所以拿已知时间点钉住。
    #[test]
    fn date_stamp_matches_known_utc_dates() {
        for (secs, want) in [
            (0, "19700101"),
            (1_700_000_000, "20231114"),
            (1_709_164_800, "20240229"), // 闰日
            (1_735_689_600, "20250101"), // 跨年
            (4_102_444_800, "21000101"), // 2100 不是闰年
        ] {
            assert_eq!(date_stamp(secs), want, "secs = {secs}");
        }
    }
}
