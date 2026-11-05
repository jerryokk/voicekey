<p align="center"><img src="app/src-tauri/icons/128x128.png" width="128" alt="VoiceKey"></p>

<h1 align="center">VoiceKey</h1>

<p align="center">Windows 上的语音输入：按住说话，文字打进电脑当前光标处。<br>手机扫码当麦克风和触摸板，也能直接用电脑的麦克风。</p>

## 功能

- **手机当麦克风**：扫码即用，不装 App。笔记本麦克风离嘴远、混着键盘和风扇声，手机离嘴近、自带降噪，识别明显更准
- **手机当触摸板**：不用回到电脑前，在手机上挪光标、点击、滚动
- **本机麦克风**（可选）：手机不在手边时，按住右 Alt 说话
- **发到电脑**：手机上的剪贴板、文字、图片、文件一键发到电脑
- **八个识别渠道**：豆包、微信、千问、百度、搜狗、讯飞、Chatterfly 七家云端，外加完全本地的微信离线（约 100MB，断网可用）；「多渠道」同时跑几家，说完挑一条
- 麦克风只在按住时打开，松手即关

## 使用

### 手机

1. 左键点托盘图标，弹出扫码面板（二维码、在线手机数、识别渠道都在这里）
2. 手机扫码，点「启用麦克风」，浏览器会问一次权限
3. 用手机上半部分的触摸板把光标挪到要输入的地方，点一下
4. 按住下方按钮说话，松手文字自动打进去
   - 说错了：按住时往上滑，按钮变红再松手，这一句丢弃
   - 删字：左边的退格键，单击删一个，按住连删

电脑重启后地址会变，手机页断线一会儿就会露出「扫码」按钮，直接调起相机扫新码。

| 触摸板手势 | 效果 |
|---|---|
| 单指滑动 | 移动光标，划得越快走得越远 |
| 轻点 / 连点两下 | 单击 / 双击 |
| 轻点一下，紧接着按住再滑 | 拖拽（手机轻震表示进入，抬手松开）。单击因此晚约 0.2 秒发出 |
| 双指轻点 / 三指轻点 | 右键 / 中键 |
| 双指滑动 | 滚动（自然方向），松手带惯性 |

位移按电脑屏幕宽的比例换算，同一划在 1080p 和 4K 上走过的视觉距离相同；速度在设置里调（0.4–2.5×）。

### 本机麦克风

在设置窗的「本机」页打开。按住**右 Alt**（可改右 Ctrl）说话，松开识别；按住时按 Esc 取消，轻点不会触发。多渠道的候选出现在屏幕底部的浮层里，点一条上屏。

它不经过网络通道，打开 VoiceKey 就能用。手机和本机同一时间只能有一方在说，另一方按下会提示稍等。

### 发到电脑

手机页右上角的按钮：发剪贴板、文字、图片或文件。电脑右下角弹出收件面板，不抢输入框的焦点：

- 剪贴板：直接写进电脑剪贴板，几秒后自动收起
- 文字：点「复制」
- 图片、文件：存进「下载」文件夹，点「在文件夹中显示」

关掉的卡片在连接页「最近收到」里还能找回，退出即清空。

### 托盘

左键扫码面板，右键「打开 VoiceKey」（完整设置：连接 / 本机 / 渠道 / 设置）和「退出」。

## 隐私与安全

- 二维码里的地址等同于「能往这台电脑输入文字」的凭证，别外传、别截图发群里。地址每次启动都会变；「断开手机」让旧码立即失效
- 音频、触摸板、发到电脑的内容都走 WebRTC 直连（端到端加密），Cloudflare 隧道只承载页面与信令
- 云端渠道会把语音发给对应厂商；选微信离线时音频不出本机
- 收到的文件只会落在「下载」文件夹本身，带「来自网络」标记，电脑端从不自动打开

## 工作原理

```
手机浏览器 ──WSS──▶ Cloudflare 隧道 ──▶ 127.0.0.1:47800（页面、信令）
    │
    └──── WebRTC 直连 ─────▶ 音频 / 触摸板 / 发到电脑 ──┐
                                                       ├──▶ 识别引擎 ──▶ 前台窗口
本机麦克风（WASAPI）+ 右 Alt 键盘钩子 ─────────────────┘
```

- **为什么要隧道**：浏览器只在 https 下给麦克风权限，同一个 WiFi 也绕不过去
- **直连提前建好**：ICE 与 DTLS 握手要几百毫秒，页面就绪就建。音频通道先空着，按下才打开麦克风换进去（`replaceTrack`，不重新协商），松手摘下关掉
- **不吃字头**：每个声源一个 `Inbox`，空闲时保留最近 0.5 秒。手机的 `start` 绕隧道、常比直连的声音晚到；本机要按满 250ms 才算一句。这段声音都接进这一句
- **一个会话**：识别会话全局唯一，由发言权 `Floor` 仲裁（手机连接从 1 编号，本机是 `LOCAL`）。结束一句的一方先发结束事件再交还，下一位的 Press 一定排在后面
- **上屏**：粘到当前前台，不切窗口。流式上屏用退格 + 补打（`Typer`），定稿走剪贴板 + Ctrl+V 并还原剪贴板。管理员窗口收不到模拟输入，这时正文留在剪贴板并明说

## 从源码构建

需要 Rust（stable）、Node 22+、MSVC 生成工具。libopus 是仓库里预编译的静态库，不需要 cmake、bindgen、pkg-config。

```bash
# cloudflared 不进 git，先下载到这里（CI 会自动下载）
mkdir -p app/src-tauri/binaries
curl -fL -o app/src-tauri/binaries/cloudflared.exe \
  https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe

cd app && npm ci
npm run tauri -- dev                     # 开发，vite 热更新
npm run tauri -- build                   # 安装包：target/release/bundle/{msi,nsis}
npm run tauri -- build --bundles msi     # 只出 msi（缺 NSIS 工具链时）
```

界面在编译期嵌进 exe，直接用 cargo 时先构建前端（`cargo test` 同理）：

```bash
cd app && npm run build && cd ..
cargo build --release    # target/release/voicekey.exe
```

免安装包（版本号取自 `tauri.conf.json`，CI 发布用同一个脚本）：

```powershell
.\scripts\build.ps1            # 只编译
.\scripts\build.ps1 -Package   # 再打包：dist\VoiceKey-<版本>-x64-portable.zip
```

cloudflared 只认 `voicekey.exe` 同级的 `runtime\tunnel\cloudflared.exe`，安装版、绿色版、`cargo build` 产物都是这个结构。找不到时扫码面板显示原因和「重试」。

版本号写在根 `Cargo.toml` 与 `tauri.conf.json`，CI 要求两者与 tag 一致。

## 测试

```bash
cargo test --workspace     # 84 项
```

覆盖协议与手机页字段对账、Opus 往返、音频分帧与预录、会话状态机、发言权、快捷键状态机、收文件（文件名清洗、重名、中断清理、限额）、设置校验与 1.0 迁移等。

不用手机验证识别链路（WAV 需 16kHz 单声道），以及离线模型联调：

```bash
cargo run -p voicekey-core --bin vk-test -- doubao|wetype|qwen|baidu|sogou|iflytek|chatterfly file.wav
cargo run -p voicekey-wtlocal --bin wtl -- --unpack pack.apk 模型目录   # 解包官方语音包
cargo run -p voicekey-wtlocal --bin wtl -- 模型目录 a.wav b.wav         # 逐文件计时，--live 按实时推流
```

## 代码结构

| 路径 | 内容 |
|---|---|
| `crates/core` | 各厂商识别协议、音频分帧与预录（`Inbox` / `pump`）、Opus |
| `crates/remote` | 手机接入：本地服务、信令、WebRTC 收流、发言权 `Floor`、发到电脑（`xfer`）、隧道、手机页 |
| `crates/platform` | 上屏与 `Typer`、触摸板 `Pointer`、本机麦克风、键盘钩子、资源管理器与窗口工具 |
| `crates/wtlocal` | 微信离线：解析官方模型、纯 Rust 推理 |
| `crates/opus-sys` | libopus 预编译静态库 + 最小 FFI |
| `app/src-tauri` | 会话状态机、本机声源、收件、托盘、浮层、设置 |
| `app/src` | Svelte 界面 |
| `scripts` | release 构建与打包 |

`crates/remote/src/protocol.rs` 是手机页与电脑的唯一契约，手机发来的消息都 `deny_unknown_fields`；`page_agrees_with_protocol` 测试保证页面里的名字与 Rust 枚举一致，任何一边改名 `cargo test` 直接失败。

### 实现要点

- **会话**：纯状态机，`Mode: Idle / Single / Pick`。识别中再按一次是「接着说」；迟到的引擎结果靠代际票根作废；松手 15 秒没定稿按断线处理，已识别的文字保留。声源先停泵再发 Release；`Cancel` 不产出任何东西，流式已打出的字退回
- **多设备**：有人在说话时只有说话的那台能动鼠标、退格；候选归最后说话的一方，进度也只告诉它
- **键盘钩子**：`WH_KEYBOARD_LL` 吞掉选中的键，否则单按 Alt 会激活菜单栏、补打的字会变成 Alt 组合键。AltGr 合成的左 Ctrl 一并吞掉；注入的键不认；按不满 250ms 算误触；锁屏、UAC 时收不到松开，按松开处理
- **收文件**：先写 `.part` 收完再改名，断线 / 取消即删；文件名清洗（只取最后一段、非法字符、保留名、限长保扩展名），重名按 `名字 (1).扩展名` 编号；写 `Zone.Identifier`；单文件 2GB、文字 1MB，磁盘余量不够直接拒绝
- **不抢焦点的窗口**：收件面板与浮层用 `SW_SHOWNOACTIVATE` 弹出。窗口本身仍可激活，否则 WebView2 吞掉第一下点击；点浮层候选上屏前先把焦点还回去
- **cloudflared**：绑在 kill-on-close 作业对象里，VoiceKey 崩溃、被强杀也会带走它；`--protocol auto`（屏蔽 UDP 的网络）、metrics 绑 `127.0.0.1:0`（多实例抢端口）、stdout 与 stderr 都要读（只读一个会卡死）
- **本地服务**：端口 47800，只绑 `127.0.0.1`；URL 里的 128 位 token 在 WS 与信令上都校验
- **设置**：与当前结构严格对应（`deny_unknown_fields`），损坏就整份回到默认；1.0.0 的文件自动迁移

## 声明

七家在线渠道通过分析输入法客户端协议实现，仅供学习与个人使用，与字节跳动、腾讯、阿里巴巴等公司无关。接口可能随时变更或失效；使用在线渠道时语音会发送到对应厂商的服务器。
