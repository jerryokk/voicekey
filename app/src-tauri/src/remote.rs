//! 手机接入：拉起本地服务与隧道，把设备事件转成会话消息。
//!
//! 服务随应用启动拉起。隧道起不来或中途退出都是失败：界面显示原因并提供重试，
//! 重试会关掉旧服务、整套重新拉起（临时域名也会换一个）。
//! 整个生命周期跑在一个任务里，启动、重试、关闭天然串行，不会两份服务抢端口。

use crate::session::types::Msg;
use crate::state::{AppState, Phase, RemoteStatus};
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use voicekey_remote::{Config, Floor, Service};

/// 固定端口：二维码里带的就是它，改了这个值等于换了接入点
const PORT: u16 = 47800;

static RESTART: OnceLock<mpsc::UnboundedSender<()>> = OnceLock::new();

/// 更新状态并通知前端刷新。事件不带数据：二维码要按当前 token 现画，统一让前端拉 get_state
fn set_status(app: &AppHandle, f: impl FnOnce(&mut RemoteStatus)) {
    f(&mut app.state::<AppState>().remote_status.write().unwrap());
    let _ = app.emit("remote", ());
}

/// cloudflared 只有一个位置：exe 旁边的 runtime/tunnel。
/// 安装版、免安装版、`cargo build` 产物都是这个结构（tauri-build 会把资源拷到 target/<profile>）
fn cloudflared() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("找不到程序路径：{e}"))?;
    let dir = exe.parent().ok_or("程序路径没有上级目录")?;
    Ok(dir.join("runtime").join("tunnel").join("cloudflared.exe"))
}

/// 手机发来的文件存到 Windows 的「下载」文件夹（用户改过位置也跟着走）
fn downloads() -> Result<std::path::PathBuf, String> {
    dirs::download_dir().ok_or_else(|| "找不到「下载」文件夹".into())
}

async fn start(floor: Arc<Floor>) -> Result<Service, String> {
    Service::start(Config { port: PORT, cloudflared: cloudflared()?, floor, downloads: downloads()? }).await
}

/// 重新拉起服务（界面上的「重试」）
pub fn restart() {
    if let Some(tx) = RESTART.get() {
        let _ = tx.send(());
    }
}

/// 等界面上的「重试」。启动期间攒下的重复点击一并消化；false 表示应用在退出
async fn wait_retry(again: &mut mpsc::UnboundedReceiver<()>) -> bool {
    let alive = again.recv().await.is_some();
    while again.try_recv().is_ok() {}
    alive
}

fn fail(app: &AppHandle, what: &str, e: String) {
    crate::log_error!("{what}：{e}");
    set_status(app, |s| {
        s.phase = Phase::Failed;
        s.error = Some(e);
        s.devices = 0;
    });
}

/// `floor` 是手机与电脑本机共用的发言权；手机这边只往里面发事件，不拥有它
pub fn spawn(app: AppHandle, floor: Arc<Floor>) {
    let (tx, mut again) = mpsc::unbounded_channel();
    if RESTART.set(tx).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            set_status(&app, |s| *s = RemoteStatus::STARTING);
            let mut service = match start(floor.clone()).await {
                Ok(s) => s,
                Err(e) => {
                    fail(&app, "手机接入启动失败", e);
                    if !wait_retry(&mut again).await {
                        return;
                    }
                    continue;
                }
            };

            // 只打域名，不打带 token 的完整地址：日志会被截图、被上报
            crate::log_info!("手机接入已就绪：{}", service.origin());
            *app.state::<AppState>().remote.write().unwrap() = Some(service.handle());
            set_status(&app, |s| s.phase = Phase::Ready);

            let mut devices = service.devices();
            let mut logs = service.logs();
            let broken = loop {
                tokio::select! {
                    // 发送端在服务里，服务活着就不会 Closed；Lagged 只是漏记了几条
                    Ok(line) = logs.recv() => crate::log_error!("手机接入：{line}"),
                    reason = service.exited() => break Some(reason),
                    Ok(()) = devices.changed() => {
                        let n = *devices.borrow_and_update();
                        crate::ui::tray_devices(&app, n);
                        set_status(&app, |s| s.devices = n);
                    }
                    _ = again.recv() => break None,
                }
            };
            while again.try_recv().is_ok() {}

            *app.state::<AppState>().remote.write().unwrap() = None;
            crate::ui::tray_devices(&app, 0);
            service.close().await;
            // 旧服务的连接已全部断开：说到一半的那句按断线处理，没松开的鼠标键全部放开
            let reason = broken.clone().unwrap_or_else(|| "手机接入已重启".into());
            let _ = app.state::<AppState>().tx.send(Msg::Reset(reason));

            if let Some(e) = broken {
                fail(&app, "手机接入中断", e);
                if !wait_retry(&mut again).await {
                    return;
                }
            }
        }
    });
}
