# CODEBUDDY.md

This file provides guidance to CodeBuddy Code when working with code in this repository.

## 项目概览

VoiceKey 是 Windows 上的语音输入工具。**音频来源是手机**：手机扫码打开一个网页，按住按钮说话，音频经 WebRTC 送到电脑，识别结果打进电脑当前前台窗口。电脑本身不录音。

- 只有一个平台（Windows），没有移动端 App
- 页面的 https 靠 Cloudflare Quick Tunnel 提供——浏览器只在安全上下文里给麦克风权限，局域网 http 拿不到
- 音频走 WebRTC 直连（隧道不转 UDP），打不通时自动回落到同一条 WebSocket 上传 PCM
- 识别全部在电脑上跑，手机只是个麦克风

## 目录

```
app/                 Tauri 外壳 + Svelte 界面
  src/               设置页、浮层
  src-tauri/         会话状态机、托盘、手机接入装配
    binaries/        cloudflared.exe 放这里（不进 git，打包时映射到资源目录）
crates/
  core/              七家识别协议、音频分帧、Opus 编码
  remote/            本地服务、WS 信令、WebRTC 收流、隧道、手机页面
  platform/          上屏（剪贴板 + Ctrl+V）
  wtlocal/           微信离线推理
tools/
  probe/             不用手机就能验证协议层的小工具，workspace 成员
docs/phone-mic-plan.md  手机接入方案的设计与验收记录
package.cmd          打包出 piik 式便携目录 dist\VoiceKey（exe + runtime\tunnel\cloudflared.exe）
```

## 常用命令

```bash
# 构建与运行（在 app/ 下）
npm ci
npx tauri dev                      # 开发运行（前端 vite 固定 1420 端口）
npx tauri build                    # 出 msi / nsis 安装包

# 打包前必须有这个文件：CI 会下载，本地从 cloudflared 官方 Release 取一份放这里
app/src-tauri/binaries/cloudflared.exe

# 测试
cargo test --workspace             # 引擎协议、音频分帧、隧道地址解析、丢包统计、会话状态机
cargo test -p voicekey             # 只跑会话状态机（controller）
cargo test -p voicekey-remote      # 只跑接入层（rtc / tunnel）

# 命令行验证识别链路（WAV 需 16kHz 单声道）
cargo run -p voicekey-core --bin vk-test -- doubao|wetype|qwen|baidu|sogou|iflytek|chatterfly file.wav

# 手机接入的独立验证器：不用手机就能跑通整条链路
cargo run -p voicekey-probe
node tools/probe/pcm-test.mjs "ws://127.0.0.1:47800/ws?t=<token>" <某个 16k 单声道 wav>
```

`tools/probe` 是 `voicekey-remote` 的最小外壳，不参与正式构建，用来在没有手机的情况下验证协议层。

## 架构

### crate 划分

| crate | 职责 |
|---|---|
| `crates/core` | 七个云端引擎（豆包、微信、千问、百度、搜狗、讯飞、Chatterfly）+ 音频重采样分帧 + Opus 编码 |
| `crates/remote` | 手机接入：axum 服务、WS 信令、WebRTC 收流、Opus 解码、隧道 |
| `crates/opus-sys` | libopus 预编译静态库 + 最小 FFI（7 个函数），编码（发云端）与解码（收手机音频）共用 |
| `crates/platform` | 只做一件事：把文字打进当前前台窗口（剪贴板 + Ctrl+V），以及边说边上屏的 `Typer` |
| `crates/wtlocal` | 微信离线：纯 Rust 加载官方 xnet 模型 |
| `app/src-tauri` | 桌面壳：会话状态机、托盘、浮层、设置、更新 |
| `app/src` | Svelte 界面：设置、浮层 |

### 音频链路

```
手机浏览器 <--WSS--> cloudflared --> 127.0.0.1:47800
   │                                        │
   └──── SRTP/UDP 直连 ──> webrtc Receiver ──┴──> Inbox ──> 泵(20ms 一帧) ──> Audio ──> Engine
```

两条通道都写进同一个 `Inbox`，下游无感知；泵每 10ms 把整帧取走送给引擎。

- WebRTC：`opus_decoder_create(16000, 1)` 直接解到 16k，20ms 正好 320 样本，不需要重采样
- 回落：页面把 16k Int16 按 320 样本打包成二进制发过来；只在 `iceConnectionState` 起不来或丢包 >5% 时用
- 丢包率按 RTP seq 缺口算。**超过 5% 就会让页面下次改走回落**——Opus 丢包不会断连，只会悄悄污染解码器状态，文字看着正常但其实是错的

### 会话状态机（`app/src-tauri/src/controller.rs`）

`Ctl` 是唯一的状态持有者，所有输入都是 `Msg`：

- `Msg::Remote(Press/Release/Abort/Commit/Erase, Option<Audio>)` —— 手机端事件，Press 时附带本句音频；
  `Erase(n)` 是手机退格键，`recording` 时忽略（流式 `Typer` 记着已打字数，外部退格会让它失准）
- `Mode`：`Idle / Single(ch) / Pick`（多渠道候选）
- `gen`：每句自增，回传的 partial/done 带着它回来比对，不匹配直接丢
- `Segments`：支持「识别中再按一次接着说」，各段按顺序拼接，全部定稿才算完成
- 松手后起 15 秒超时，超时按 abort 处理并保留已识别文字
- `emit_rows` 33ms 节流，结果同时推给手机（`rows` / `partial`）和桌面浮层

时序上有一点必须记住：**服务端先停泵、再发 `Release`**。所以 controller 收到 Release 时音频流已经结束了，它只需要更新界面状态，不要去关麦。

### 多设备

连接按设备区分，但**识别会话全局唯一**：`Hub.hold` 里存当前持有者，非持有者的二进制音频在入口就被丢弃。否则两台手机同时说话，音频会混进同一个 `Inbox`，出来的字是两个人拼在一起的。忙时新设备收到 `busy`。

### 安全

- token 128 位，放在 URL 里，WS 与信令都校验，不匹配 403
- 只绑 `127.0.0.1`，只有 cloudflared 能进
- 隧道随应用启动，公网入口常开；**地址加 token 落到别人手里等于把输入通道交出去**，日志里不要打完整 URL

## 约定与易错点

- **构建零外部工具**：libopus 是预编译静态库，随 `crates/opus-sys/lib/` 分发，build.rs 只输出链接指令。整条链路只需要 rustc + MSVC linker + node，没有 cmake / bindgen / pkg-config。换 MSVC 大版本或升级 libopus 才需要重新生成 `opus.lib`（方法见 `crates/opus-sys/README.md`）。
- `cloudflared.exe` 放在 `app/src-tauri/binaries/`（不进 git，CI 下载）。运行时按四处查找：资源目录 → 程序目录的 `runtime/tunnel/` → **编译期记下的 `app/src-tauri/binaries/`** → PATH。所以 `cargo run` 或直接跑 `target/debug/voicekey.exe` 都不需要先拷文件。
- 隧道启动参数不能省：`--protocol auto`（默认只走 QUIC，屏蔽 UDP 的网络起不来）、`metrics: 127.0.0.1:0`（否则第二个实例抢端口）、stdout 与 stderr 都要读（只读一个会把 cloudflared 卡死）。
- 从 cloudflared 日志里抠域名时要求名称含连字符，否则会把 `api.trycloudflare.com` 当成隧道地址。
- 端口固定 47800，二维码里带的就是它。
- 上屏不切窗口：往当前前台粘。所以用户必须先在电脑上点好目标输入框。Windows 有前台锁，后台进程 `SetForegroundWindow` 会被拒，别试图「先激活再粘贴」。
- `Typer`（边说边上屏）与 `paste` 是两条路径：开流式时不走剪贴板，靠退格 + 补打；关流式才一次性粘贴。
- 版本号仍有两处：`Cargo.toml` 的 `workspace.package.version` 与 `app/src-tauri/tauri.conf.json`。
- 设置持久化靠 `Settings` 上的 `#[serde(default)]` + `sanitize()`：新增字段必须能在旧配置上反序列化。
