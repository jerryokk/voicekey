<p align="center"><img src="app/src-tauri/icons/128x128.png" width="128" alt="VoiceKey"></p>

<h1 align="center">VoiceKey</h1>

<p align="center">Windows 上的语音输入：手机当麦克风。手机扫码，按住说话，文字打进电脑当前光标处。</p>

## 为什么是手机

笔记本内置麦克风离嘴远，键盘声和风扇声都在里面；手机是阵列麦、离嘴近，还自带降噪。把收音放在手机上，识别准确率明显更好，也不用抢电脑的音频设备。

## 怎么用

1. 打开 VoiceKey，设置页「手机接入」里有一个二维码
2. 手机扫码，点「启用麦克风」（浏览器会问一次权限）
3. 在电脑上点好要输入的输入框，让光标停在那里
4. 回到手机按住按钮说话，松手后文字自动打进去
5. 说错了：按住时往上滑，按钮变红再松手，这一句直接丢弃；已经打出去的上句用卡片右下角的「撤回」
6. 想删字用左边的退格键：单击删一个字，按住连续删

勾选两个以上渠道时是「多渠道」模式：说完手机会列出各家的结果和耗时，点一条上屏。

## 端到端怎么走

```
手机浏览器 ──WSS──▶ Cloudflare 隧道 ──▶ 本机 127.0.0.1:47800
    │                                        │
    └── SRTP/UDP 直连 ──▶ 本机 WebRTC 接收 ──┴──▶ 识别引擎 ──▶ 打进前台窗口
```

- 页面必须走 https：浏览器只在安全上下文里给麦克风权限。这就是需要隧道的原因，哪怕手机和电脑在同一个 WiFi
- 音频走 WebRTC 直连（隧道只转 HTTP，不转 UDP）。打不通时自动回落到同一条 WebSocket 上传 PCM，不会因为打洞失败就用不了
- 丢包率超过 5% 会自动切到回落通道——Opus 丢包不会断连，只会让识别悄悄变差，这个切换是有意的

## 识别渠道

豆包、微信、千问、百度、搜狗、讯飞、Chatterfly 七家云端识别，外加完全本地的微信离线（约 100MB，本机推理，断网可用，音频不外传）。

## 从源码构建

需要 Rust（stable）、Node 22+、MSVC 生成工具。libopus 是仓库里预编译好的静态库，构建链只有 rustc、MSVC linker 和 node，没有 cmake、bindgen、pkg-config。

```bash
# cloudflared 放这里（不进 git，CI 会下载）
mkdir -p app/src-tauri/binaries
curl -L -o app/src-tauri/binaries/cloudflared.exe \
  https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe

cd app
npm ci
npm run tauri -- dev                          # 开发：vite 热更新
npm run tauri -- build                       # 安装包：target/release/bundle/{msi,nsis}
npm run tauri -- build --bundles msi         # 只出 msi（本机缺 NSIS 工具链时用这个）
```

界面在编译期嵌进 exe，所以不打安装包时也要先 `npm run build` 再 `cargo build`（`cargo test` 同理）：

```bash
cd app && npm run build && cd ..
cargo build              # target/debug/voicekey.exe，双击即用
cargo build --release    # target/release/voicekey.exe
```

要一个免安装的绿色目录，把这两份放一起就是完整的：`voicekey.exe` 与同级的 `runtime\tunnel\cloudflared.exe`。程序启动时按「资源目录 → 程序目录的 `runtime/tunnel/` → 编译期记录的位置 → PATH」找 cloudflared，放对一处即可。

## 测试与联调

```bash
cargo test --workspace             # 32 项：协议↔手机页字段对账、Opus 往返、音频分帧、
                                   # 隧道地址解析、丢包统计、会话状态机、设置兼容
```

不用手机也能验证识别链路（WAV 需 16kHz 单声道）：

```bash
cargo run -p voicekey-core --bin vk-test -- doubao|wetype|qwen|baidu|sogou|iflytek|chatterfly file.wav
```

离线模型联调（解包官方语音包 / 逐文件计时 / `--live` 按实时节奏推流）：

```bash
cargo run -p voicekey-wtlocal --bin wtl -- --unpack pack.apk 模型目录
cargo run -p voicekey-wtlocal --bin wtl -- 模型目录 a.wav b.wav
```

## 架构

| 路径 | 内容 |
|---|---|
| `crates/core` | 七家厂商的识别协议、音频分帧与 Opus 编码（发云端编码、收手机解码共用一份 FFI） |
| `crates/remote` | 手机接入：本地服务、WS 信令、WebRTC 收流与解码、Cloudflare 隧道、手机页 |
| `crates/platform` | 上屏（剪贴板 + Ctrl+V）与边说边上屏的 `Typer` |
| `crates/wtlocal` | 微信离线：解析官方模型、纯 Rust 推理 |
| `crates/opus-sys` | libopus 预编译静态库 + 最小 FFI |
| `app/src-tauri` | 会话状态机、托盘、浮层、设置、离线模型管理 |
| `app/src` | Svelte 界面 |
| `app/src-tauri/icons` | 图标，`logo.py` 一次生成全部尺寸 |

两条通道都写进同一个 `Inbox`，泵每 10ms 取整帧（20ms 一帧）送引擎，下游不感知走的是哪条路。

会话是一个纯状态机，输入统一为 `Msg`：

- 手机端事件：`Press / Release / Cancel / Abort / Commit / Erase / Undo`，Press 附带本句音频流
- `Mode: Idle / Single(ch) / Pick`（多渠道候选）；识别中再按一次是「接着说」，各段按顺序拼接
- 迟到的引擎结果靠代际票根作废；松手后 15 秒超时按断线处理，已识别的文字保留
- 时序要点：服务端**先停泵、再发 Release**，controller 收到时音频已结束；`Cancel` 则停泵后不发 Release，什么都不产出，流式已打出的字要退回
- 多设备：识别会话全局唯一，非持有者的音频在入口就被丢弃；忙时新设备收到 `busy`

WS 协议（`crates/remote/src/protocol.rs`）是手机页与电脑之间的唯一契约，`page_agrees_with_protocol` 测试保证页面里的字段名和 Rust 枚举一致——任何一边改名，`cargo test` 直接失败。

## 实现要点

- **上屏不切窗口**：粘到当前前台。用户要先点好目标输入框；高权限窗口收不到模拟输入，这时自动改走剪贴板并在手机与浮层明说
- 流式 `Typer`（退格 + 补打）与一次性 `paste` 是两条路径，开流式时不走剪贴板；外部退格会让 Typer 失准，录音中手机退格键被忽略
- 撤销只开 8 秒窗口且要求当前没有句子在走：撤错东西比撤不了更糟
- cloudflared 启动参数不能省：`--protocol auto`（屏蔽 UDP 的网络起不来）、metrics 绑 `127.0.0.1:0`（第二个实例抢端口）、stdout 与 stderr 都要读（只读一个会卡死）；从日志抠域名时要求行内含连字符，避免把 `api.trycloudflare.com` 当地址
- 端口固定 47800；token 128 位放在 URL 里，WS 与信令都校验；服务只绑 `127.0.0.1`，只有 cloudflared 能进
- 版本号唯一来源是根 `Cargo.toml` 的 `workspace.package.version`；`tauri.conf.json` 不写 version，发版时 CI 按标签覆盖
- 设置持久化靠 `Settings` 的 `#[serde(default)]` + `sanitize()`：新增字段必须能在旧配置上反序列化

## 安全

- 二维码里的地址等同于「能往这台电脑输入文字」的凭证，别外传、别截图发群里
- 地址每次启动都会变，重启后需要重新扫码
- 音频经 Cloudflare 到本机，再由本机发给所选识别厂商；选微信离线时不出本机

## 声明

七家在线渠道通过分析输入法客户端协议实现，仅供学习与个人使用，与字节跳动、腾讯、阿里巴巴等公司无关。接口可能随时变更或失效；使用在线渠道时语音会发送到对应厂商的服务器。
