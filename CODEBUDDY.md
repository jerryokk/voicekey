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
  src/               设置页（pages/）、浮层（Hud）、二维码（Qr）、ui/ 基础组件
  src-tauri/
    main.rs          启动装配（插件、setup、窗口事件）
    ipc.rs           前端能调的全部命令，唯一接口面
    state.rs         AppState + update_settings（改设置的唯一入口）
    controller.rs    会话编排（调引擎、HUD、上屏、广播）
    remote.rs        voicekey-remote 的宿主：服务与隧道随应用启停，事件转成 Msg
    wtoffline.rs     离线模型下载与状态（对接 ipc 的 models）
    settings.rs      Settings 定义 + sanitize（旧配置必须能反序列化）
    log.rs           文件日志（7 天滚动）
    session/         types（Msg / 分段 / 候选数据结构）、machine（选行与代际，可单测）、
                     engines（注册表 + 本句任务句柄）
    ui/              geometry（坐标与缩放）、windows（三个窗口）、overlay（胶囊）、tray（托盘）
    binaries/        cloudflared.exe 放这里（不进 git，打包时映射到资源目录）
    icons/           图标与生成脚本 logo.py
crates/
  core/              七家识别协议、音频分帧、Opus 编码（bin/vk-test 是单渠道联调入口）
  remote/            本地服务、WS 信令（protocol.rs 是唯一契约）、WebRTC、隧道、手机页
  platform/          上屏（剪贴板 + Ctrl+V）
  wtlocal/           微信离线推理（bin/wtl 解包官方语音包、逐文件计时）
opus-sys/            libopus 预编译静态库 + 最小 FFI，见 crates/opus-sys/README.md
```

## 常用命令

```bash
# 前端 + 编译（界面在编译期嵌进 exe，改前端必须先出 app/dist）
cd app && npm run build && cd ..
cargo build                        # target/debug/voicekey.exe，双击即用
cargo build --release              # target/release/voicekey.exe

# Tauri 常规流程（在 app/ 下）
npm ci
npm run tauri -- dev                         # 开发运行（vite 固定 1420 端口）
npm run tauri -- build                       # 出 msi / nsis 安装包

# 免安装介质：dist\VoiceKey\voicekey.exe 与 dist\VoiceKey\runtime\tunnel\cloudflared.exe 放一起
# （运行时按 资源目录 → 程序目录\runtime\tunnel → 编译期路径 → PATH 找 cloudflared）

# 打包前必须有这个文件：CI 会下载，本地从 cloudflared 官方 Release 取一份放这里
app/src-tauri/binaries/cloudflared.exe

# 测试（同样要先出 app/dist：voicekey 这个 crate 编译期就要嵌入它）
cargo test --workspace             # 引擎协议、音频分帧、隧道地址解析、丢包统计、会话状态机
cargo test -p voicekey             # 会话状态机（含 machine.rs 的选行/代际测试）
cargo test -p voicekey-remote      # 只跑接入层（rtc / tunnel）

# 命令行验证识别链路（WAV 需 16kHz 单声道）
cargo run -p voicekey-core --bin vk-test -- doubao|wetype|qwen|baidu|sogou|iflytek|chatterfly file.wav

# 离线模型联调：解包官方语音包 / 逐文件计时 / --live 按实时节奏推流看流式输出
cargo run -p voicekey-wtlocal --bin wtl -- --unpack pack.apk 模型目录
cargo run -p voicekey-wtlocal --bin wtl -- 模型目录 a.wav b.wav
```

## 架构

### crate 划分

| crate | 职责 |
|---|---|
| `crates/core` | 七个云端引擎（豆包、微信、千问、百度、搜狗、讯飞、Chatterfly）+ 音频重采样分帧 + Opus 编码 |
| `crates/remote` | 手机接入：axum 服务、WS 信令、WebRTC 收流、Opus 解码、隧道 |
| `crates/opus-sys` | libopus 预编译静态库 + 最小 FFI（7 个函数），编码（发云端）与解码（收手机音频）共用 |
| `crates/platform` | 只做一件事：把文字打进当前前台窗口（剪贴板 + Ctrl+V），以及边说边上屏的 `Typer` |
| `crates/wtlocal` | 微信离线：纯 Rust 加载官方 xnet 模型 |
| `app/src-tauri` | 桌面壳：会话状态机、托盘、浮层、设置 |
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

### 会话状态机（`session/machine.rs` + `controller.rs`）

不碰 I/O 的判断都在 `session/machine.rs`，可直接单测：

- `Tickets`：代际与超时票号。引擎回结果可能晚于用户开下一句，代号对不上就作废。
- `Picker`：多渠道候选的行构建（排序 / 跳过未下载模型 / 沿用上次选择）、
  定稿后焦点怎么挪、点击一条是「上屏 / 只切焦点 / 抖动提示」。
- `Engines`：渠道→引擎的注册表，**同时持有本句的引擎任务句柄**——
  取消一句话要能把它的任务一起停掉，句柄和「这句属于谁」必须放在一起。

会话的输入统一是 `session::types::Msg`（`controller` 不再自己定义）。

`controller.rs` 只剩编排（调引擎、HUD、上屏、广播）。改这两块的判断逻辑前先看
`machine.rs` 里的测试——它们覆盖了多设备、渠道失败、用户手选与自动切换打架这几种情况。


`Ctl` 是唯一的状态持有者，所有输入都是 `Msg`：

- `Msg::Remote(Press/Release/Cancel/Abort/Commit/Erase/Undo, Option<Audio>)` —— 手机端事件，Press 时附带本句音频；
  `Cancel` 是按住后上滑取消：服务端停泵、丢弃音频、不发 Release，controller 也不上屏，流式已打出去的字要退回来；
  `Erase(n)` 是手机退格键，`recording` 时忽略（流式 `Typer` 记着已打字数，外部退格会让它失准）；
  `Undo` 只在「8 秒内且当前没有句子在走」时才发 Ctrl+Z（撤错东西比撤不了更糟）
- `Mode`：`Idle / Single(ch) / Pick`（多渠道候选）
- 代际与超时票号由 `Tickets` 持有（见上），partial/done 带代号回来比对，不匹配直接丢
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
- 图标全部由 `app/src-tauri/icons/logo.py` 生成（Pillow，超采样后缩到各尺寸），含 ico 与两个托盘图；改设计只改这个脚本，别手工改 png。
- 版本号只有一处：`Cargo.toml` 的 `workspace.package.version`。`tauri.conf.json` 不写 version，
  Tauri 会回落到 Cargo.toml；发版时 CI 用 `--config {"version":"$VER"}` 按标签覆盖。
- 设置持久化靠 `Settings` 上的 `#[serde(default)]` + `sanitize()`：新增字段必须能在旧配置上反序列化。
