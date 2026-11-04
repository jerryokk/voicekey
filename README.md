<p align="center"><img src="app/src-tauri/icons/128x128.png" width="128" alt="VoiceKey"></p>

<h1 align="center">VoiceKey</h1>

<p align="center">Windows 上的语音输入：手机当麦克风。手机扫码，按住说话，文字打进电脑当前光标处。</p>

## 为什么是手机

笔记本内置麦克风离嘴远，键盘声和风扇声都在里面；手机是阵列麦、离嘴近，还自带降噪。把收音放在手机上，识别准确率明显更好，也不用抢电脑的音频设备。

## 怎么用

1. 左键点托盘图标，弹出扫码面板：二维码、在线手机数、识别渠道都在这里，点一个渠道即切换
2. 手机扫码，点「启用麦克风」（浏览器会问一次权限）
3. 手机上半部分是触摸板，用它把电脑光标挪到要输入的地方点一下
4. 按住下方按钮说话，松手后文字自动打进去
5. 说错了：按住时往上滑，按钮变红再松手，这一句直接丢弃
6. 想删字用左边的退格键：单击删一个字，按住连续删

托盘右键只有「打开 VoiceKey」（完整设置）和「退出」。选「多渠道」时，说完手机会列出各家的结果，点一条上屏。

### 触摸板

| 手势 | 效果 |
|---|---|
| 单指滑动 | 移动光标，划得越快走得越远 |
| 轻点 | 左键单击；连点两下是双击 |
| 轻点一下，紧接着按住再滑 | 按住左键拖拽（和笔记本触摸板一样），手机轻震一下表示进入，抬手松开。单击因此会晚约 0.2 秒发出 |
| 双指轻点 / 三指轻点 | 右键 / 中键 |
| 双指滑动 | 滚动（自然方向），松手带惯性 |

位移按「电脑屏幕宽的比例」传输：手机不知道电脑分辨率，电脑端按光标所在显示器换算像素，所以同一划在 1080p 和 4K 上走过的视觉距离相同。速度倍率在设置里调（0.4–2.5×）。

## 端到端怎么走

```
手机浏览器 ──WSS──▶ Cloudflare 隧道 ──▶ 本机 127.0.0.1:47800
    │                                        │
    └── SRTP/UDP 直连 ──▶ 本机 WebRTC 接收 ──┴──▶ 识别引擎 ──▶ 打进前台窗口
```

- 页面必须走 https：浏览器只在安全上下文里给麦克风权限。这就是需要隧道的原因，哪怕手机和电脑在同一个 WiFi
- 音频（Opus 音轨）与触摸板（DataChannel `input`，有序可靠）都走 WebRTC 直连；隧道只承载页面与信令。直连建不起来时按钮不可用，页面会自动重建
- 音轨一直开着，电脑端平时只保留最近 0.5 秒作为预录：按下那一刻之前的字头不会被吃掉

## 识别渠道

豆包、微信、千问、百度、搜狗、讯飞、Chatterfly 七家云端识别，外加完全本地的微信离线（约 100MB，本机推理，断网可用，音频不外传）。

## 从源码构建

需要 Rust（stable）、Node 22+、MSVC 生成工具。libopus 是仓库里预编译好的静态库，构建链只有 rustc、MSVC linker 和 node，没有 cmake、bindgen、pkg-config。

```bash
# cloudflared 放这里（不进 git，CI 会下载）
mkdir -p app/src-tauri/binaries
curl -fL -o app/src-tauri/binaries/cloudflared.exe \
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

cloudflared 只认一个位置：`voicekey.exe` 同级的 `runtime\tunnel\cloudflared.exe`。安装版、绿色版、`cargo build` 产物都是这个结构（tauri-build 会把资源拷到 `target/<profile>/runtime/tunnel`）。找不到就是启动失败，扫码面板会显示原因和「重试」。

也可以用脚本一步完成前端构建和 release 编译，加 `-Package` 再打免安装包（版本号取自 `tauri.conf.json`）：

```powershell
.\scripts\build.ps1            # 只编译：target\release\voicekey.exe
.\scripts\build.ps1 -Package   # 再打包：dist\VoiceKey-<版本>-x64-portable.zip，发给别人解压即用
```

打包由 `scripts\package.ps1` 完成，CI 打 tag 发布时用的是同一个脚本，zip 结构与本地一致（CI 里的 cloudflared 是构建时下载的最新版）。

## 测试与联调

```bash
cargo test --workspace             # 58 项：协议↔手机页字段对账、Opus 往返、音频分帧与预录、
                                   # 隧道地址解析、token、指针余量、会话状态机、设置校验
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
| `crates/platform` | 上屏（剪贴板 + Ctrl+V）、边说边上屏的 `Typer`、触摸板的 `Pointer`（SendInput 绝对坐标） |
| `crates/wtlocal` | 微信离线：解析官方模型、纯 Rust 推理 |
| `crates/opus-sys` | libopus 预编译静态库 + 最小 FFI |
| `app/src-tauri` | 会话状态机、托盘、浮层、设置、离线模型管理 |
| `app/src` | Svelte 界面 |
| `app/src-tauri/icons` | 图标，`logo.py` 一次生成全部尺寸 |
| `scripts` | `build.ps1` release 构建（`-Package` 顺带打包）、`package.ps1` 打免安装 zip |

每个连接一个 `Inbox`，WebRTC 解出的 16k 样本写进去；按下后泵每 10ms 取整帧（20ms 一帧）送引擎。

会话是一个纯状态机，输入统一为 `Msg`：

- 手机端事件：`Press / Release / Cancel / Abort / Commit / Erase / Pad / Gone`，Press 附带本句音频流；`Gone` 时替断开的设备放开按住的鼠标键
- `Mode: Idle / Single(ch) / Pick`（多渠道候选）；识别中再按一次是「接着说」，各段按顺序拼接
- 迟到的引擎结果靠代际票根作废；松手后 15 秒超时按断线处理，已识别的文字保留
- 时序要点：服务端**先停泵、再发 Release**，controller 收到时音频已结束；`Cancel` 则停泵后不发 Release，什么都不产出，流式已打出的字要退回
- 多设备：识别会话全局唯一，忙时新设备收到 `busy`。有人在说话时只有说话的那台能动鼠标、退格；候选只能由最后说话的那台来选，它断开后谁都能选

`crates/remote/src/protocol.rs` 是手机页与电脑之间的唯一契约（WS 上的 `Op`/`Out`，DataChannel 上的 `Pad`），手机发来的 `Op`、`Pad` 都 `deny_unknown_fields`。`page_agrees_with_protocol` 测试保证页面里的名字和 Rust 枚举一致——任何一边改名，`cargo test` 直接失败。

## 实现要点

- **cloudflared 绑在 kill-on-close 的作业对象里**：voicekey 无论正常退出、崩溃还是被强杀，内核都会带走它启动的那个 cloudflared，且不影响机器上其他 cloudflared 实例
- **上屏不切窗口**：粘到当前前台。高权限窗口收不到模拟输入，这时正文留在剪贴板并在手机与浮层明说，让用户自己 Ctrl+V
- 流式 `Typer`（退格 + 补打）与一次性 `paste` 是两条路径，开流式时不走剪贴板；外部退格会让 Typer 失准，所以流式上屏进行中电脑忽略手机发来的退格；手机页在录音中也不让按退格
- cloudflared 启动参数不能省：`--protocol auto`（屏蔽 UDP 的网络起不来）、metrics 绑 `127.0.0.1:0`（第二个实例抢端口）、stdout 与 stderr 都要读（只读一个会卡死）；从日志抠域名时要求行内含连字符，避免把 `api.trycloudflare.com` 当地址
- 端口固定 47800；token 128 位放在 URL 里，WS 与信令都校验；服务只绑 `127.0.0.1`，只有 cloudflared 能进
- 版本 1.0.0，写在根 `Cargo.toml` 与 `tauri.conf.json`；CI 要求这两处与 tag 三者一致，不一致直接失败
- 设置文件与当前结构一一对应（`deny_unknown_fields`，无默认字段）：缺字段、多字段都按损坏处理，整份回到默认；`sanitize()` 只收拢越界值

## 安全

- 二维码里的地址等同于「能往这台电脑输入文字」的凭证，别外传、别截图发群里
- 地址每次启动都会变，重启后需要重新扫码。手机页能直接调起相机扫新码（相机画面交给 Worker 里的 jsQR 解码，不依赖多数手机浏览器都没有的 BarcodeDetector；解码库在连上电脑时就取回，旧地址失效后照样能扫，没取到时提示改用手机相机扫；只认本服务的地址）；电脑主动断开时整页提示扫码，断线超过 4 秒时状态栏露出「扫码」小按钮，同时继续重连
- 音频经 Cloudflare 到本机，再由本机发给所选识别厂商；选微信离线时不出本机

## 声明

七家在线渠道通过分析输入法客户端协议实现，仅供学习与个人使用，与字节跳动、腾讯、阿里巴巴等公司无关。接口可能随时变更或失效；使用在线渠道时语音会发送到对应厂商的服务器。
