//! 文件日志。
//!
//! 改成 Windows GUI 子系统后没有控制台，stderr 无从查看；出问题只能靠用户描述。
//! 这里把关键事件写进 `%LOCALAPPDATA%\VoiceKey\logs\voicekey-YYYYMMDD.log`，
//! 按天（UTC）切分、只保留 7 个文件，量很小。常驻跨天时，写入那一刻发现日期变了就换文件并清理旧的。
//!
//! 只记录事件与原因，不记录 token（URL 里带 token，日志里一律只打 origin）。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
/// 当前日志文件对应的日期；与写入时的日期不同就换文件
static DAY: Mutex<String> = Mutex::new(String::new());

pub fn init() {
    let _ = LOG_DIR.set(dir().filter(|d| fs::create_dir_all(d).is_ok()));
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
fn prune(dir: &Path) {
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
    let Some(Some(dir)) = LOG_DIR.get() else { return };
    let secs = now_secs();
    let today = date_stamp(secs);
    // 锁持到写完：并发写入不会交错成半行，跨天时也只清理一次
    let mut day = DAY.lock().unwrap();
    let path = dir.join(format!("voicekey-{today}.log"));
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) else { return };
    let _ = writeln!(f, "[{}] {:5} {}", clock(secs), level.to_uppercase(), message);
    // 今天的文件已经建出来了再清理：按名字排序时它一定在保留的 7 个里
    if *day != today {
        *day = today;
        prune(dir);
    }
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
    voicekey_core::util::civil_from_days((secs / 86_400) as i64)
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

    /// 文件名写错不只是难看：按名字排序裁剪旧文件时会误删当天的日志，所以拿已知时间点钉住。
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
