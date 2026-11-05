//! 电脑本机麦克风：WASAPI 共享模式采集。
//!
//! 不引第三方音频库：`AUTOCONVERTPCM` 让系统音频引擎负责下混与重采样，
//! 直接要 16kHz 单声道 16 位——和手机那条路径解码出来的样本一模一样，识别引擎不用知道来源。
//! 采集跑在自己的线程上（COM 要求线程亲和），样本经回调交出去。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::E_ACCESSDENIED;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, WAVEFORMATEX,
    WAVE_FORMAT_PCM,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

const RATE: u32 = 16_000;
/// 共享模式缓冲 200ms：10ms 轮询一次，留足余量不丢包
const BUFFER_HNS: i64 = 2_000_000;
/// 打开设备的上限。正常 100ms 内完成，驱动卡住时不陪它等
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct MicError {
    pub message: String,
    /// Windows 隐私设置不让桌面应用用麦克风：界面要给出「打开系统设置」
    pub privacy: bool,
}

impl std::fmt::Display for MicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn fail(e: windows::core::Error) -> MicError {
    let code = e.code();
    let (message, privacy) = if code == E_ACCESSDENIED {
        ("Windows 隐私设置禁止了桌面应用使用麦克风".to_string(), true)
    } else if code == AUDCLNT_E_DEVICE_IN_USE {
        ("麦克风被别的程序独占了".to_string(), false)
    } else if code == AUDCLNT_E_DEVICE_INVALIDATED {
        ("麦克风已断开".to_string(), false)
    } else {
        (format!("麦克风不可用：{}", e.message()), false)
    };
    MicError { message, privacy }
}

/// 本线程的 COM（MTA），线程退出时收尾。只在本模块自己开的线程里用
struct Com(bool);

impl Com {
    fn init() -> Self {
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

fn take_wide(p: PWSTR) -> String {
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

fn enumerator() -> Result<IMMDeviceEnumerator, MicError> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(fail)
}

fn device_id(d: &IMMDevice) -> Result<String, MicError> {
    unsafe { d.GetId() }.map(take_wide).map_err(fail)
}

fn device_name(d: &IMMDevice) -> Option<String> {
    unsafe {
        let store = d.OpenPropertyStore(STGM_READ).ok()?;
        let mut v = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let name = PropVariantToStringAlloc(&v).ok().map(take_wide);
        let _ = PropVariantClear(&mut v);
        name.filter(|n| !n.is_empty())
    }
}

/// 一个可选的输入设备。`id` 是 WASAPI 端点 id：换 USB 口、改名都不变，存进设置里用它
#[derive(Debug, Clone)]
pub struct Mic {
    pub id: String,
    pub name: String,
}

/// 当前可用的输入设备，以及系统默认的那一个的 id
pub fn list() -> Result<(Vec<Mic>, Option<String>), MicError> {
    // 调用方可能是界面主线程（单线程套间），放到自己的线程里做，不碰它的 COM 状态
    std::thread::spawn(|| {
        let _com = Com::init();
        let en = enumerator()?;
        let all = unsafe { en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }.map_err(fail)?;
        let n = unsafe { all.GetCount() }.map_err(fail)?;
        let mut mics = Vec::with_capacity(n as usize);
        for i in 0..n {
            let d = unsafe { all.Item(i) }.map_err(fail)?;
            let name = device_name(&d).unwrap_or_else(|| "未命名设备".into());
            mics.push(Mic { id: device_id(&d)?, name });
        }
        let default = unsafe { en.GetDefaultAudioEndpoint(eCapture, eConsole) }.ok().and_then(|d| device_id(&d).ok());
        Ok((mics, default))
    })
    .join()
    .unwrap_or_else(|_| Err(MicError { message: "枚举麦克风时出错".into(), privacy: false }))
}

/// 样本回调：每次拿到的是一段 16k 单声道样本，长度不定
pub type Sink = Box<dyn FnMut(&[i16]) + Send>;
/// 采集中途失败（拔掉设备、被独占）：只会调用一次，之后采集线程退出
pub type Lost = Box<dyn FnOnce(MicError) + Send>;

/// 一路正在采集的麦克风。丢弃即停
pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    /// 打开设备开始采集。`device` 为 None 用系统默认；指定的设备不在了也回落到默认，
    /// 返回值的第二项告诉调用方发生了回落
    pub fn open(device: Option<String>, mut sink: Sink, lost: Lost) -> Result<(Capture, bool), MicError> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let quit = stop.clone();
        let thread = std::thread::Builder::new()
            .name("voicekey-mic".into())
            .spawn(move || {
                let _com = Com::init();
                let (client, capture, fell_back) = match start(device.as_deref()) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(fell_back));
                if let Err(e) = run(&capture, &quit, &mut sink) {
                    lost(e);
                }
                let _ = unsafe { client.Stop() };
            })
            .map_err(|e| MicError { message: format!("无法启动采集线程：{e}"), privacy: false })?;
        let capture = Capture { stop, thread: Some(thread) };
        match ready_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(fell_back)) => Ok((capture, fell_back)),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // 驱动卡住：不 join，让线程等回来后自己看到 stop 收尾，否则调用方又陪它卡住
                let mut capture = capture;
                capture.thread = None;
                Err(MicError { message: "麦克风打开超时".into(), privacy: false })
            }
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn start(device: Option<&str>) -> Result<(IAudioClient, IAudioCaptureClient, bool), MicError> {
    let en = enumerator()?;
    let default = || unsafe { en.GetDefaultAudioEndpoint(eCapture, eConsole) }.map_err(fail);
    let (dev, fell_back) = match device {
        Some(id) => match unsafe { en.GetDevice(&HSTRING::from(id)) } {
            Ok(d) if unsafe { d.GetState() }.is_ok_and(|s| s == DEVICE_STATE_ACTIVE) => (d, false),
            _ => (default()?, true),
        },
        None => (default()?, false),
    };
    let client: IAudioClient = unsafe { dev.Activate(CLSCTX_ALL, None) }.map_err(fail)?;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * 2,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_HNS, 0, &format, None) }.map_err(fail)?;
    let capture: IAudioCaptureClient = unsafe { client.GetService() }.map_err(fail)?;
    unsafe { client.Start() }.map_err(fail)?;
    Ok((client, capture, fell_back))
}

/// 10ms 取一次包，直到要求停下或设备出错
fn run(capture: &IAudioCaptureClient, stop: &AtomicBool, sink: &mut Sink) -> Result<(), MicError> {
    let mut buf: Vec<i16> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(10));
        while unsafe { capture.GetNextPacketSize() }.map_err(fail)? > 0 {
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
            unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }.map_err(fail)?;
            buf.clear();
            if data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                buf.resize(frames as usize, 0);
            } else {
                // 单声道 16 位，一帧就是一个 i16
                buf.extend_from_slice(unsafe { std::slice::from_raw_parts(data as *const i16, frames as usize) });
            }
            unsafe { capture.ReleaseBuffer(frames) }.map_err(fail)?;
            sink(&buf);
        }
    }
    Ok(())
}

/// 一段样本的响度，0..=1。给浮层的电平条用：人声 RMS 很少过 0.2，放大到条子能动起来
pub fn level(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum / samples.len() as f64).sqrt() / 32768.0;
    (rms as f32 * 5.0).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::level;

    #[test]
    fn level_is_bounded() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0; 320]), 0.0);
        assert_eq!(level(&[i16::MAX; 320]), 1.0);
        let quiet = level(&[300; 320]);
        assert!(quiet > 0.0 && quiet < 0.1, "{quiet}");
    }
}
