//! 手机接入：拉起本地服务与隧道，把设备事件转成会话消息。
//!
//! 服务的生命周期等于应用的生命周期：随启动拉起，随退出消失。临时隧道地址每次启动
//! 都会变，所以拿到地址后要立刻告诉前端刷新二维码。

use crate::controller::Msg;
use crate::{AppState, RemoteStatus};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use voicekey_remote::{Config, Event, Service};

/// 固定端口：二维码里带的就是它，改了这个值等于换了接入点
pub const PORT: u16 = 47800;

/// 更新状态并通知前端刷新。event 数据只带状态本身，详情让前端拉 get_state
fn set_status(app: &AppHandle, status: RemoteStatus) {
    let payload = serde_json::json!({
        "state": status.state,
        "error": status.error,
        "lanOnly": status.lan_only,
    });
    app.state::<AppState>().remote_status.write().unwrap().clone_from(&status);
    let _ = app.emit("remote", payload);
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let config = Config { port: PORT, cloudflared: cloudflared(&app), tunnel: true };
        let mut service = match Service::start(config).await {
            Ok(s) => s,
            Err(e) => {
                crate::log_error!("手机接入启动失败：{e}");
                set_status(&app, RemoteStatus { state: "failed", error: Some(e), lan_only: false });
                return;
            }
        };

        // 只打域名，不打带 token 的完整地址：控制台截图、日志上报都会把它带出去
        crate::log_info!("手机接入已就绪：{}", service.origin());
        let lan_only = service.origin().starts_with("http://127.0.0.1");
        if lan_only {
            crate::log_warn!("隧道不可用，仅本机地址");
        }
        {
            let st = app.state::<AppState>();
            *st.remote.write().unwrap() = Some(service.handle());
            *st.remote_url.write().unwrap() = Some(service.url());
            *st.remote_origin.write().unwrap() = Some(service.origin().to_string());
        }
        let error = service.tunnel_error().map(|e| e.to_string());
        set_status(&app, RemoteStatus { state: "ready", error, lan_only });

        while let Some(event) = service.next_event().await {
            // 音频只在按下这一刻取：取早了是上一句的，取晚了会被下一句覆盖
            let audio = if matches!(event, Event::Press) { service.take_audio().await } else { None };
            let _ = app.state::<AppState>().tx.send(Msg::Remote(event, audio));
        }
    });
}

/// 找 cloudflared，四处依次尝试，都找不到才放弃：
///
/// 1. 资源目录——打包安装后的位置（`bundle.resources` 把 binaries/ 映射到根部）
/// 2. 程序目录下的 runtime/tunnel——把 exe 单独拷出来用的场景
/// 3. 源码树里的 app/src-tauri/binaries——`cargo run` / `tauri dev` 时用，
///    编译期就把路径烧进二进制，所以直接跑 target/debug 里的 exe 也不用先拷文件
/// 4. PATH——自己装了 cloudflared 的用户
fn cloudflared(app: &AppHandle) -> Option<PathBuf> {
    let name = if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" };
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join(name));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("runtime").join("tunnel").join(name));
            candidates.push(dir.join(name));
        }
    }
    // 开发期：编译时记录下来的源码路径
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(name));

    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}
