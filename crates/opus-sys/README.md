# opus-sys

libopus 的最小 FFI + 预编译静态库。没有 cmake，没有 bindgen，没有编译步骤。

## 内容

- `lib/opus.lib` — MSVC x64 Release 静态库（约 1MB），随仓库分发
- `lib/COPYING` — libopus 的 BSD 许可证
- `src/lib.rs` — 7 个 C 函数 + 4 个 ctl 常量的 FFI 声明，以及一个编码→解码往返测试

## 这个库是怎么来的

源码取自 `audiopus_sys 0.2.2` 随包分发的 libopus（`opus/` 目录，1.x 系列），
用 VS 2026 的 MSVC 以 Release 配置编译为 x64 静态库，仅此一次。
普通构建**不需要**任何 C 工具链之外的步骤；只有换 MSVC 大版本或升级 libopus 时
才需要重新生成，方法：

```bash
cmake -S <libopus 源码> -B build -A x64 -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_POLICY_VERSION_MINIMUM=3.5
cmake --build build --config Release
cp build/Release/opus.lib lib/
```

## 换行注意

`opus_encoder_ctl` 是 C 可变参数协议，Rust 侧声明为 `...`，传参必须与
opus.h 中对应宏要求的类型一致（bitrate 是 int，final_range 是 `*mut u32`）。
