# Windows 单平台 + 手机当麦克风：实施计划

> 状态：待评审。评审通过后按 P0→P7 执行，每个阶段独立可验收。
> 相关事实来源：`Cargo.toml`、`app/src-tauri/src/*`、`crates/*`、`ios/PROTOCOL.md`、`.github/workflows/release.yml`，以及 `D:\Work\Projects\Piik\crates\server\src\tunnel.rs`、`crates\server\src\launch.rs`。

## 1. 目标

只保留 Windows 桌面端；音频来源从电脑麦克风换成手机浏览器；手机扫码打开网页，按住说话，电脑负责识别并把文字打到当前前台窗口。

**做**

- 手机端单页网页（无需安装 App），PWA 可加到主屏
- 本地服务 + Cloudflare Quick Tunnel，二维码分享
- WebRTC 传音频为主路径，WSS PCM 为回落
- 识别结果上屏到 PC 当前前台窗口，多渠道候选在手机上选
- 保留现有全部识别渠道（7 家云端 + 微信离线）

**不做**

- 不做 iOS / Android 原生 App（现有代码整体删除）
- 不做 TURN / 自建中继（只留接口，需要时再接）
- 不做多用户、不做音频存档、不做语音以外的输入
- 不追求手机端离线识别（离线引擎仍跑在 PC 上）

## 2. 现状盘点

| 处置 | 内容 | 规模 |
|---|---|---|
| 保留 | `crates/core`（7 家协议 + 音频分帧 + Opus 编码） | 2,429 行 |
| 保留 | `crates/wtlocal`（微信离线，全平台） | 1,008 行 |
| 保留 | `crates/platform` 的 `paste` / `Typer` / 键名表 | ~120 行 |
| 保留 | `app/src` 的设置页骨架、`Hud.svelte` | ~500 行 |
| 删除 | `ios/`、`android/`、`Sources/` + `Package.swift`、`build.sh`、`Scripts/opus.sh`、`Scripts/icon.swift` | 21,000 行 |
| 删除 | `crates/platform/src/mac.rs`、`crates/hanbao`、`app/src-tauri/src/offline.rs` | 812 行 |
| 删除 | `app/src-tauri/src/hotkey.rs`（全局快捷键状态机） | 439 行 |
| 删除 | `crates/core/src/audio.rs` 的 `Recorder` / `microphones` / `watch_microphones` / `devices.rs` | ~360 行 |
| 删除 | `Pick.svelte`、`ui.rs` 的 pick 面板与 `caret()`、`platform::caret/secure_input/front_app` | ~380 行 |
| 删除 | 设置页 `Keys` / `Audio` / `Perms` / `Compare` | 276 行 |
| 新增 | `crates/remote`（服务端 + 信令 + WebRTC + 隧道 + 页面） | ~1,100 行 || 新增 | `Qr.svelte`、设置页「连接」页 | ~200 行 |
| 改造 | `controller.rs` 输入源、`ui.rs` 窗口装配、`settings.rs` 字段 | 净减约 200 行 |

结论：仓库 28k → 约 9k；但**桌面这一个应用**从 3.9k 变 4.3k 左右，复杂度是平移不是下降。真正的收益是平台矩阵 4→1。

## 3. 目标架构

```
手机浏览器 (https 页面)
  │  ① 页面、信令：HTTP + WSS  ──►  Cloudflare Quick Tunnel  ──►  127.0.0.1:47800
  │  ② 音频：Opus over SRTP/UDP ──► 直连（同网 host 候选 / 跨网 srflx 打洞）
  ▼                                                              
┌──────────────────────────── Tauri 进程（Windows） ────────────────────────────┐
│  axum Server ──┬── GET  /            页面（include_str! 单文件）              │
│                ├── WS   /ws?t=        信令 + 回落音频 + 结果下行               │
│                └── GET  /qr.svg        二维码（供手机无法安装时备用）          │
│                                                                              │
│  RTC Receiver (webrtc 0.17.2)  on_track → RTP 解包 → opus_decode(16k)        │
│                                     └──────────────┐                         │
│  PCM Fallback (WS 二进制)  →  Framer(16k) ─────────┤                         │
│                                                    ▼                         │
│                                          Audio (mpsc<Vec<i16>>)             │
│                                                    ▼                         │
│                                    Ctl（现有会话状态机，改造输入源）          │
│                                                    ▼                         │
│                                    Engine → partial / final                  │
│                                       │                     │                │
│                                       ▼                     ▼                │
│                        WS 下行给手机（实时文字、候选行）   pf::paste 上屏      │
└──────────────────────────────────────────────────────────────────────────────┘
```

**关键设计：输入源抽象。** 手机的信令事件映射到现有 `HotEvent`，`controller.rs` 的 `Mode` / `Segments` / `gen` / `timeout_token` / `emit_rows` 全部不动：

| 手机事件 | 映射为 | 现有语义 |
|---|---|---|
| `{op:"press"}` | `HotEvent::Press` | `arm()` 开始缓冲（缓冲源从麦克风换成 WS/WebRTC） |
| 首帧音频到达 | `HotEvent::LongPress` | 直接 `begin()`，不需要 300ms 阈值 |
| `{op:"release"}` | `HotEvent::Release` | `end()`，等各引擎定稿 |
| 连接断开 | `HotEvent::Abort`（新增） | 保留已识别文字 |

这样能留下 `Segments`（接着说）、`gen`（作废迟到结果）、15 秒超时、33ms 节流这些已经调试好的逻辑，只删掉长按/点按/静音判定那条最绕的路径。

**结果下行直接复用 `Row` / `RowState`**：它们已经 `Serialize` 成 `listen|wait|final|error|skip`，手机端照原样渲染即可，不需要为手机另造一套数据结构。

## 4. 传输设计

### 4.1 WebRTC（主路径）

- 浏览器作为 offerer：`getUserMedia` 成功后 `addTransceiver('audio', {direction:'sendonly'})` → `createOffer` → `setLocalDescription` → 发 `{op:"offer", sdp}`
- PC 作为 answerer：`set_remote_description` → `create_answer` → `set_local_description` → **等 gathering 完成**（不做 trickle，减少状态机）→ 回 `{op:"answer", sdp}`
- 首连延迟用「页面加载即建连」抵消，按下时只切 `track.enabled`
- `iceServers` 由服务端在 `{op:"hello"}` 里下发，PC 侧用同一份；顺序：`stun.chat.bilibili.com` → `stun.cloudflare.com`（本机实测 37ms vs 167ms）
- 必须启用 mDNS 候选解析（`webrtc-mdns`），否则同网 `xxxx.local` 候选不可用，看起来像「WebRTC 坏了」
- 编码：`opus/48000/2`，PC 侧 `opus_decoder_create(16000, 1)` 直接解到 16kHz 单声道，20ms 正好 320 样本，无需重采样
- 丢包：Opus 是预测编码，丢包会污染解码器状态。先按到达顺序写入，不做重排，但**统计丢包率**；超过阈值就本次会话切回落

### 4.2 WSS PCM（回落）

- 页面把 16k Int16 的 320 样本包二进制发到同一个 WS
- 与 WebRTC 汇合在 `Audio` 通道，下游无感知
- 触发条件：3 秒内 `iceConnectionState` 未到 `connected`，或 WebRTC 中途断开
- 决策粒度是「每次按住」，不在一句话中途切换
- 带宽约 32 kB/s，只在按住时产生

### 4.3 页面音频参数

`echoCancellation: false`、`noiseSuppression: false`、`autoGainControl: true`。现有 PC 端走 cpal 是裸音频，浏览器默认开的降噪会把小声说话的开头吃掉，识别效果对不上。

注意两条路径的页面逻辑不同：WebRTC 由浏览器内部编码，页面**不做**重采样；回落路径才需要 `AudioContext({sampleRate:16000})` + AudioWorklet + JS 重采样兜底。

## 5. 安全

| 项 | 做法 |
|---|---|
| 认证 | 启动生成 128 位随机 token，URL 带 `?t=`，WS 与信令都校验，不匹配返回 403 |
| 监听地址 | 只绑 `127.0.0.1`，仅 cloudflared 可达，不触发防火墙 |
| 设备数 | 允许任意多台手机同时连接，共用同一个 token（一码多扫） |
| 会话数 | 同一时刻只允许一句话：忙时新设备收到 `busy`，松手重按即可（不排队、不抢占，避免两台手机的文字串在一起） |
| 缓存 | `Cache-Control: no-store`、`Referrer-Policy: no-referrer` |
| Origin | 校验同源，不做 CORS |
| 隧道生命周期 | **默认随应用启动**，常驻；关闭应用即杀子进程。因为公网入口常开，token 熵与会话互斥是仅有屏障，日志里不打印含 token 的完整 URL |
| 风险声明 | Quick Tunnel 域名公开可取；文档需写明「地址与 token 同时泄露等于把输入通道交出去」 |

## 6. 隧道

- 启动参数沿用已验证的组合：`--config <临时 yml>`（`metrics: 127.0.0.1:0` 避免端口冲突）、`--no-autoupdate`、`--protocol auto`（QUIC 被挡时走 TCP）、`--output json`、Windows 下 `CREATE_NO_WINDOW`
- stdout/stderr 双管道都要 drain，否则写满管道会把 cloudflared 卡死
- 从任一行日志里抠 `https://<name>.trycloudflare.com`，名称必须含连字符（排除 `api.trycloudflare.com`）
- 子进程退出自动重启；地址变化则推送事件刷新二维码
- `cloudflared.exe` **随安装包内置**（Tauri `bundle.resources`，从 `app.path().resource_dir()` 解析），固定版本、不做自动更新；升级跟随应用版本走，避免「隧道悄悄换了版本导致行为变化」
- 增量代价：安装包与更新包各增约 55MB；需在 `THIRD-PARTY-NOTICES` 里补 cloudflared 的 Apache-2.0 声明
- 保留「自定义 base url / 自己的隧道 token」字段，供有域名的用户切换成稳定地址

## 7. 手机端页面

- 首屏一次「启用麦克风」（必须在用户手势里），之后全程可用
- 主界面：大按钮按住说话、上方实时文字、渠道切换
- 多渠道时松手后列出候选，点一条 `{op:"commit", channel}`
- 断线自动重连并提示；重连期间保留已识别文字
- 多台手机同时在线时，非活动设备在大按钮上显示「另一台设备正在说话」，按键仍可按下但会立即收到 `busy` 并提示松手重试；活动设备结束时向其余设备广播 `idle`

**多设备会话模型**：连接按设备区分（每台一个 WS + 一个 WebRTC peer），但**识别会话全局唯一**。`controller.rs` 的 `Mode` 本来就是单例状态，因此不为多设备引入并发，只引入「谁持有会话」：

- 设备按下时若 `Mode == Idle`，它成为 `holder`，后续所有 `Msg` 带上 `holder` id，`gen` 校验照旧
- 非 holder 按下立刻回 `busy`，不进入 `arm()`
- holder 松手并定稿后回到 `Idle`，此时任何设备都能再按
- holder 断线按 `Abort` 处理，保留已识别文字并释放会话

这样多设备的代价只是「一个 holder 字段 + 一条广播」，不动 `Segments`、`gen`、超时和节流。

手势必须做全，否则 iOS 上会被系统抢走：

- `pointerdown/up/cancel` + `setPointerCapture`，不用原生 long-press
- `touch-action: none`、`user-select: none`、`-webkit-touch-callout: none`、`overscroll-behavior: none`
- `contextmenu` 全部 preventDefault
- 来电 / Siri / 切后台：`visibilitychange` 与 `blur` 时主动 stop 并告知服务端

## 8. 阶段与验收

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **P0 工具链** | cmake（已用 `pip install cmake` 装进 WinPython，`Lib/site-packages/cmake/data/bin` 需在 PATH）；确认 MSVC 可用；`cargo test` 全绿；`npx tauri build` 出 Windows 包 | 本机能编译主项目、能出安装包 |
| **P1 传输** | probe 接入 WebRTC：页面 offer → PC answer → 收 Opus → 打印样本数 | 手机同网 `connectionState=connected`，样本数按实时速率增长 |
| **P2 识别** | probe 接 `voicekey-core` 的 `DoubaoEngine` | 说一句话，终端打印 partial 与 final 文字 |
| **P3 上屏** | probe 调 `pf::paste` 打到记事本 | 记事本出现文字 |
| **P4 回落** | WSS PCM 通道 + 自动切换 + 丢包统计 | 断掉 UDP 后仍能出字；日志能看到切换原因 |
| **P5 并入主仓库** | 抽出 `crates/remote`；改 `controller` 输入源与 `holder` 归属；删 hotkey/Recorder/pick 面板；设置页加「连接」页与 `Qr.svelte`；`cloudflared.exe` 进 `bundle.resources` | 应用启动即出二维码，多台手机都能扫，候选在手机 |
| **P6 删平台分支** | 删 `ios/`、`android/`、`mac.rs`、`hanbao`、`offline.rs`、`Sources/`、根 `Package.swift`/`build.sh`、CI 只留 Windows；版本号收敛到一处 | `cargo test` 绿；安装包可装可更新 |
| **P7 打磨** | 长期空闲自动关隧道、日志与诊断页、PWA 图标、错误文案 | 长时间挂着不泄漏、异常有可读原因 |

P0–P4 全部在 `tools/probe/` 里做，不碰主仓库；P5 才开始动正式代码。

### 进度

| 阶段 | 状态 | 依据 |
|---|---|---|
| P0 工具链 | 完成 | `cargo test -p voicekey-core`（2 项）与 `cargo test -p voicekey`（7 项）在本机全绿，audiopus 已从源码编过 |
| P1 传输 | 完成 | `probe.mjs` 拿到 answer，含 6 条候选（host + srflx），两个 STUN 都命中 |
| P2 识别 | 服务端完成 | WAV 灌入同一链路，16 次中间结果 + 正确定稿；手机端待真机验证 |
| P3 上屏 | 已实现，待真机 | `final` 直接调 `voicekey-platform::paste`，`VKSPIKE_NO_PASTE=1` 可关；未做自动化验证（会向当前焦点窗口注入 Ctrl+V） |
| P4 回落 | 服务端完成 | `pcm-test.mjs` 走 WS 二进制，230 帧 / 144 kB / 4.6 秒，定稿正确；丢包 >5% 时服务端下发 `prefer` 让页面改走回落 |
| P5 并入 | 完成 | `crates/remote` 接入 `app/src-tauri`；`controller` 改为 Press/Release 驱动；删 `hotkey.rs`、`offline.rs`、pick 面板与光标定位；设置页换成「手机接入/识别/输出/通用」 |
| P6 删分支 | 完成 | 删 `ios/`、`android/`、`Sources/`、`Package.swift`、`build.sh`、`Scripts/`、`crates/hanbao`、`mac.rs`、`cpal` 采集；CI 只剩 Windows；`cargo test --workspace` 10 项通过 |
| P7 打磨 | 大部分 | 已做：二维码、断开提示、丢包切换、`busy` 互斥、托盘暂停、退格键、隧道四态可见（starting/ready/failed/仅限局域网）。未做：named tunnel（稳定域名）、PWA 图标、诊断页 |

`crates/remote` 里目前已有：axum 服务、cloudflared 拉起与地址解析、WS 信令、WebRTC 收流、Opus 解到 16k、`Inbox` 合流、泵任务、`busy` 互斥、丢包驱动的通道切换。`tools/probe/` 只剩「接线 + 上屏 + 二维码」三件事，以及 `pcm-test.mjs` 这个不要手机的验证入口。

`crates/remote` 的对外接口只有五个：

```rust
Service::start(Config) -> Service
service.url()                  // 扫码地址
service.next_event()           // Press / Release / Abort
service.take_audio()           // 本句的 16k 帧流（Audio），Press 之后取
service.handle() -> Handle     // 跨任务的 publish / push_pcm / inbox
```

## 9. 整体验收

- 一台干净的 Windows 机器：装包 → 扫一次码 → 按住说话 → 记事本出字，不需要装驱动、不需要改防火墙、不需要装浏览器插件
- 手机锁屏、来电、WiFi 切 4G 之后能自行恢复，且不会把半句话丢掉
- 失败必须可见：隧道起不来、麦克风被占、打洞失败，都要在手机上给出人能看懂的原因

## 10. 风险

| 风险 | 影响 | 应对 |
|---|---|---|
| 缺 cmake / MSVC | 主项目编不了 | P0 先解决；`audiopus_sys` 也支持 pkg-config 走系统库 |
| `webrtc` 0.17.2 API 与文档不一致 | 接入慢 | 缓存里已有该版本；必要时换 `str0m`（sans-IO，体积更小） |
| mDNS 候选解析未开 | 同网连不上，误判 WebRTC 不可用 | 显式启用并纳入验收项 |
| iOS 后台 / 锁屏停麦 | 长句被打断 | 明确不支持后台采集，页面被动 stop 并提示 |
| Quick Tunnel 域名每次变 | 二维码失效 | 地址变化事件 + 一键刷新；有域名用户可换稳定隧道 |
| Cloudflare 边缘在境外 | 跨网 RTT 100–300ms，边说边上屏打折 | 同网优先；接受跨网体验降级；不为此上 TURN |
| Opus 丢包污染解码器 | 识别变差且难察觉 | 统计丢包率，超阈值切回落 |
| 上屏目标不是预期窗口 | 文字打到别处 | 不切窗口，粘到当前前台；需要时先在 PC 上点好目标框 |

## 11. 已定决策

| 决策 | 结论 | 随之而来的约束 |
|---|---|---|
| 隧道默认 | **随应用启动，常驻** | 公网入口长期存在，依赖 token 熵与会话互斥；日志不打印完整 URL；二维码窗口需常驻可见 |
| 设备数 | **多台**，一码多扫 | 引入 `holder` 归属与 `busy` 提示，识别会话仍全局唯一 |
| 候选挑选 | **手机** | 删 `Pick.svelte`、`ui.rs` 的 pick 面板、`caret()`、`secure_input()`、`front_app().activate()`、macOS `VKPanel` |
| cloudflared | **内置** | 安装包与更新包各增约 55MB；补 Apache-2.0 声明；固定版本随应用升级 |
| PC 麦克风对比 | **删** | 删 `Compare.svelte` 与 `Msg::CompareToggle`；`cpal`、`audio/devices.rs`、`Recorder` 从 `voicekey-core` 移除。**`rubato` 与 `Framer` 保留**——WebRTC 解到 16k 用不上它，但回落路径若遇到浏览器不认 `sampleRate:16000` 仍要靠它兜底 |
