//! Cloudflare Quick Tunnel：把本机服务映射成一个公网 https 域名。
//!
//! 隧道只承载 HTTP 与 WebSocket —— 页面的音频走 WebRTC 直连，不经过这里。
//! 之所以还需要它，是因为浏览器只在安全上下文（https）里给麦克风权限，
//! 局域网 http 拿不到麦克风，这个限制绕不过去。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// Cloudflare 要给每个快速隧道分配边缘地址和域名，超过这个时间说明网络到不了
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const QUICK_DOMAIN: &str = ".trycloudflare.com";

pub struct Tunnel {
    child: Child,
    origin: String,
}

impl Tunnel {
    /// 公网地址，不带结尾斜杠
    pub fn origin(&self) -> &str {
        &self.origin
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // 进程退出必须带走隧道，否则公网域名会指着一个已经不属于我们的端口
        let _ = self.child.start_kill();
    }
}

/// 在几个常规位置找 cloudflared：显式指定 → 程序目录的 runtime/tunnel → PATH
pub fn find(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit.filter(|p| p.is_file()) {
        return Some(path);
    }
    let name = if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" };
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            let candidate = dir.join("runtime").join("tunnel").join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

pub async fn start(exe: &Path, local_origin: &str) -> Result<Tunnel, String> {
    let config = temp_config()?;
    let mut cmd = Command::new(exe);
    cmd.arg("tunnel")
        .arg("--config")
        .arg(&config)
        .arg("--no-autoupdate")
        .arg("--loglevel")
        .arg("info")
        .arg("--output")
        .arg("json")
        // 快速隧道默认只走 QUIC；允许 TCP，让屏蔽 UDP 的网络还有一条路
        .arg("--protocol")
        .arg("auto")
        .arg("--max-edge-addr-retries")
        .arg("0")
        .arg("--url")
        .arg(local_origin);
    hide_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| spawn_error(exe, e))?;
    let found = wait_first(child.stdout.take(), child.stderr.take(), |line| quick_origin(line)).await;
    let _ = std::fs::remove_file(&config);
    finish(child, found, "cloudflared 在分配地址前退出了，请检查网络")
}

fn temp_config() -> Result<PathBuf, String> {
    let config = std::env::temp_dir().join(format!("voicekey-cloudflared-{}.yml", std::process::id()));
    std::fs::File::create(&config)
        .and_then(|mut f| f.write_all(b"metrics: 127.0.0.1:0\n"))
        .map_err(|e| format!("无法写隧道配置：{e}"))?;
    Ok(config)
}

fn spawn_error(exe: &Path, e: std::io::Error) -> String {
    if !exe.is_file() {
        format!("隧道程序不存在：{}", exe.display())
    } else {
        format!("cloudflared 启动失败：{e}")
    }
}

fn hide_window(cmd: &mut Command) {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

async fn wait_first<F>(
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    match_line: F,
) -> Result<(Option<String>, Option<String>), tokio::time::error::Elapsed>
where
    F: Fn(&str) -> Option<String>,
{
    tokio::time::timeout(STARTUP_TIMEOUT, first_match(stdout, stderr, match_line)).await
}

fn finish(
    mut child: Child,
    found: Result<(Option<String>, Option<String>), tokio::time::error::Elapsed>,
    early_exit: &str,
) -> Result<Tunnel, String> {
    match found {
        Ok((Some(origin), _)) => Ok(Tunnel { child, origin }),
        Ok((None, detail)) => {
            let _ = child.start_kill();
            let reason = detail.unwrap_or_default();
            // 429 是最常见的一种：临时域名有配额，被限流时换个说法用户才知道该怎么办
            if reason.contains("429") {
                return Err("Cloudflare 临时域名限流（429），新建隧道太频繁，稍后会自动恢复".into());
            }
            Err(if reason.is_empty() { early_exit.into() } else { format!("{early_exit}（{reason}）") })
        }
        Err(_) => {
            let _ = child.start_kill();
            Err(format!("{} 秒内没连上 Cloudflare，请检查网络", STARTUP_TIMEOUT.as_secs()))
        }
    }
}

/// 两个管道都要读：cloudflared 会同时往 stdout 和 stderr 写，
/// 只读一个会让另一个写满管道后把它卡死。
/// 返回 (匹配到的地址, 进程退出前最后一条错误)。
/// 只回「进程退出了」等于什么都没说——真实原因（限流、DNS、鉴权）就在日志里。
async fn first_match<F>(
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    match_line: F,
) -> (Option<String>, Option<String>)
where
    F: Fn(&str) -> Option<String>,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    if let Some(s) = stdout {
        tokio::spawn(drain(s, tx.clone()));
    }
    if let Some(s) = stderr {
        tokio::spawn(drain(s, tx.clone()));
    }
    drop(tx);
    let mut detail = None;
    while let Some(line) = rx.recv().await {
        if let Some(origin) = match_line(&line) {
            return (Some(origin), detail);
        }
        if detail.is_none() && looks_like_error(&line) {
            detail = Some(readable(&line));
        }
    }
    (None, detail)
}

fn looks_like_error(line: &str) -> bool {
    line.contains("\"level\":\"error\"") || line.contains("failed") || line.contains(" ERR ")
}

/// json 日志取 msg 字段，纯文本日志截一段：两种情况都别把整行 json 甩给用户
fn readable(line: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
        if let Some(m) = v.get("msg").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    line.trim().chars().take(160).collect()
}

async fn drain<R>(reader: R, tx: tokio::sync::mpsc::UnboundedSender<String>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tx.send(line).is_err() {
            return;
        }
    }
}

/// 从任意一行日志里抠出 `https://<name>.trycloudflare.com`。
fn quick_origin(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ',' | ')' | ']' | '>' | '<'))
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches('/');
    let host = url.strip_prefix("https://")?.split('/').next()?;
    let label = host.strip_suffix(QUICK_DOMAIN)?;
    // 快速隧道名是几个单词用连字符拼的；这条判断排除 api.trycloudflare.com 之类
    if label.is_empty() || !label.contains('-') {
        return None;
    }
    if !label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return None;
    }
    Some(format!("https://{host}"))
}

#[cfg(test)]
mod tests {
    use super::quick_origin;

    #[test]
    fn finds_origin_in_json_and_plain_text() {
        let json = r#"{"msg":"Visit it at https://brave-tiger-lamp.trycloudflare.com"}"#;
        assert_eq!(quick_origin(json).as_deref(), Some("https://brave-tiger-lamp.trycloudflare.com"));
        let plain = "INF |  https://quiet-river-otter.trycloudflare.com  |";
        assert_eq!(quick_origin(plain).as_deref(), Some("https://quiet-river-otter.trycloudflare.com"));
    }

    #[test]
    fn ignores_service_hostnames() {
        for line in ["https://api.trycloudflare.com/v1/health", "https://example.com", "INF Starting tunnel"] {
            assert_eq!(quick_origin(line), None, "line: {line}");
        }
    }
}
