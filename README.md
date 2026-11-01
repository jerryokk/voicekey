<p align="center"><img src="docs/icon.png" width="128" alt="VoiceKey"></p>

<h1 align="center">VoiceKey</h1>

<p align="center">Windows 上的语音输入：手机当麦克风。手机扫码，按住说话，文字打进电脑当前光标处。</p>

## 为什么是手机

笔记本内置麦克风离嘴远，键盘声和风扇声都在里面；手机是阵列麦、离嘴近，还自带降噪。把收音放在手机上，识别准确率明显更好，也不用抢电脑的音频设备。

## 怎么用

1. 打开 VoiceKey，设置页「手机接入」里有一个二维码
2. 手机扫码，点「启用麦克风」（浏览器会问一次权限）
3. 在电脑上点好要输入的输入框，让光标停在那里
4. 回到手机按住按钮说话，松手后文字自动打进去

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

需要 Rust（stable）、Node 22+、CMake、MSVC。`audiopus_sys` 会用 CMake 从源码编 libopus。

```bash
# 打包前把 cloudflared 放到这里（CI 会自动下载）
mkdir -p app/src-tauri/binaries
curl -L -o app/src-tauri/binaries/cloudflared.exe \
  https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe

cd app
npm ci
npx tauri dev            # 开发运行
npx tauri build          # 出 msi / nsis 安装包
```

命令行试识别（WAV 需 16kHz 单声道）：

```bash
cargo run -p voicekey-core --bin vk-test -- doubao 你的录音.wav
```

## 安全

- 二维码里的地址等同于「能往这台电脑输入文字」的凭证，别外传、别截图发群里
- 地址每次启动都会变，重启后需要重新扫码
- 音频经 Cloudflare 到本机，再由本机发给所选识别厂商；选微信离线时不出本机

## 结构

| 路径 | 内容 |
|---|---|
| `crates/core` | 七家厂商的识别协议、音频分帧与 Opus 编码 |
| `crates/remote` | 手机接入：本地服务、WS 信令、WebRTC 收流与解码、Cloudflare 隧道 |
| `crates/platform` | 上屏（剪贴板 + Ctrl+V）与边说边上屏的 `Typer` |
| `crates/wtlocal` | 微信离线：解析官方模型、纯 Rust 推理 |
| `app/src-tauri` | 会话状态机、托盘、浮层、设置、更新 |
| `app/src` | Svelte 界面 |
| `tools/probe/` | 不用手机就能验证协议层的小工具，不参与正式构建 |

## 声明

七家在线渠道通过分析输入法客户端协议实现，仅供学习与个人使用，与字节跳动、腾讯、阿里巴巴等公司无关。接口可能随时变更或失效；使用在线渠道时语音会发送到对应厂商的服务器。
