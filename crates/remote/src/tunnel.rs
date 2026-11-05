//! Cloudflare Quick Tunnel：把本机服务映射成一个公网 https 域名。
//!
//! 隧道只承载页面与 WebSocket 信令——音频、触摸板、发到电脑都走 WebRTC 直连，不经过这里。
//! 之所以还需要它，是因为浏览器只在安全上下文（https）里给麦克风权限，
//! 局域网 http 拿不到麦克风，这个限制绕不过去。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

/// Cloudflare 要给每个快速隧道分配边缘地址和域名，超过这个时间说明网络到不了
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const QUICK_DOMAIN: &str = ".trycloudflare.com";

pub struct Tunnel {
    child: Child,
    origin: String,
    /// Windows 作业对象：句柄在本进程里，本进程无论怎么退（正常退出、崩溃、被强杀），
    /// 内核关闭句柄时都会按 kill-on-close 连带结束 cloudflared。只包含我们自己启动的那个。
    _job: Job,
}

impl Tunnel {
    /// 公网地址，不带结尾斜杠
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// 等 cloudflared 退出，返回给用户看的原因。它一退，二维码上的地址就失效了
    pub async fn exited(&mut self) -> String {
        match self.child.wait().await {
            Ok(status) => format!("隧道进程意外退出（{status}）"),
            Err(e) => format!("隧道进程状态未知：{e}"),
        }
    }
}

/// 把子进程绑进一个 kill-on-close 的作业对象
fn bind_to_job(child: &Child) -> Result<Job, String> {
    let fail = |e: windows::core::Error| format!("无法把 cloudflared 绑进作业对象：{e}");
    let process = HANDLE(child.raw_handle().ok_or("cloudflared 已经退出")?);
    unsafe {
        let job = Job(CreateJobObjectW(None, PCWSTR::null()).map_err(fail)?);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .map_err(fail)?;
        AssignProcessToJobObject(job.0, process).map_err(fail)?;
        Ok(job)
    }
}

struct Job(HANDLE);

// 作业对象句柄只在 Drop 时关闭一次，跨线程移动是安全的
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub async fn start(exe: &Path, local_origin: &str) -> Result<Tunnel, String> {
    let config = Config::write()?;
    let mut cmd = Command::new(exe);
    cmd.arg("tunnel")
        .arg("--config")
        .arg(&config.0)
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
        .arg(local_origin)
        // 隧道建到一半失败时 Tunnel 还不存在，靠它收掉子进程；之后 Tunnel 被丢弃时同样生效
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0);

    let mut child = cmd.spawn().map_err(|e| spawn_error(exe, e))?;
    let job = bind_to_job(&child)?;
    let mut lines = drain(child.stdout.take(), child.stderr.take());
    match tokio::time::timeout(STARTUP_TIMEOUT, first_match(&mut lines)).await {
        Ok(Ok(origin)) => Ok(Tunnel { child, origin, _job: job }),
        Ok(Err(detail)) => Err(early_exit(detail)),
        Err(_) => Err(format!("{} 秒内没连上 Cloudflare，请检查网络", STARTUP_TIMEOUT.as_secs())),
    }
}

/// 临时配置：只放 `metrics: 127.0.0.1:0`（默认端口会和机器上别的 cloudflared 抢）。
/// 显式给 `--config` 还有一个作用：不读用户自己 `~/.cloudflared` 里的配置，
/// 那里的命名隧道设置会让快速隧道起不来。cloudflared 启动时读一次，之后删掉即可
struct Config(PathBuf);

impl Config {
    fn write() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("voicekey-cloudflared-{}.yml", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(b"metrics: 127.0.0.1:0\n"))
            .map_err(|e| format!("无法写隧道配置：{e}"))?;
        Ok(Self(path))
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn spawn_error(exe: &Path, e: std::io::Error) -> String {
    if exe.is_file() {
        format!("cloudflared 启动失败：{e}")
    } else {
        format!("隧道程序不存在：{}", exe.display())
    }
}

fn early_exit(detail: Option<String>) -> String {
    const EXITED: &str = "cloudflared 在分配地址前退出了，请检查网络";
    match detail {
        // 429 是最常见的一种：临时域名有配额，被限流时换个说法用户才知道该怎么办
        Some(d) if d.contains("429") => "Cloudflare 临时域名限流（429），新建隧道太频繁，稍后再重试".into(),
        Some(d) => format!("{EXITED}（{d}）"),
        None => EXITED.into(),
    }
}

/// 两个管道都要一直读到进程退出：cloudflared 会同时往 stdout 和 stderr 写，
/// 哪个管道没人读，写满后就会把它卡死。拿到地址之后的日志直接丢弃
fn drain(
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
) -> mpsc::UnboundedReceiver<String> {
    async fn pipe<R: AsyncRead + Unpin>(reader: R, tx: mpsc::UnboundedSender<String>) {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(line);
        }
    }
    let (tx, rx) = mpsc::unbounded_channel();
    if let Some(s) = stdout {
        tokio::spawn(pipe(s, tx.clone()));
    }
    if let Some(s) = stderr {
        tokio::spawn(pipe(s, tx));
    }
    rx
}

/// 等到第一条带地址的日志。进程先退出就返回它报的第一条错误：
/// 只回「进程退出了」等于什么都没说——真实原因（限流、DNS、鉴权）就在日志里
async fn first_match(lines: &mut mpsc::UnboundedReceiver<String>) -> Result<String, Option<String>> {
    let mut detail = None;
    while let Some(line) = lines.recv().await {
        if let Some(origin) = quick_origin(&line) {
            return Ok(origin);
        }
        if detail.is_none() && looks_like_error(&line) {
            detail = Some(readable(&line));
        }
    }
    Err(detail)
}

fn looks_like_error(line: &str) -> bool {
    line.contains("\"level\":\"error\"") || line.contains("failed") || line.contains(" ERR ")
}

/// json 日志取 msg 字段，纯文本日志截一段：两种情况都别把整行 json 甩给用户
fn readable(line: &str) -> String {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("msg")?.as_str().map(str::to_string))
        .unwrap_or_else(|| line.trim().chars().take(160).collect())
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
    if !label.contains('-') || !label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return None;
    }
    Some(format!("https://{host}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 进程退出前只说了错误：要把第一条错误的 msg 带出来，而不是整行 json
    #[tokio::test]
    async fn reports_first_error_when_no_origin() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        for line in [
            r#"{"level":"info","msg":"Starting"}"#,
            r#"{"level":"error","msg":"status code 429"}"#,
            r#"{"level":"error","msg":"later"}"#,
        ] {
            tx.send(line.to_string()).unwrap();
        }
        drop(tx);
        let detail = first_match(&mut rx).await.unwrap_err();
        assert_eq!(detail.as_deref(), Some("status code 429"));
        assert!(early_exit(detail).contains("429"));
    }
}
