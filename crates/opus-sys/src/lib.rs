//! libopus 的最小 FFI：只声明本项目实际用到的 7 个 C 函数和 4 个 ctl 常量。
//!
//! 静态库在 `lib/opus.lib`（MSVC x64，Release），由 build.rs 直接链接——
//! 不需要 cmake、bindgen 或 pkg-config。库的来源与重建方法见本 crate 的 README.md。

use std::os::raw::c_int;

#[repr(C)]
pub struct OpusEncoder {
    _p: [u8; 0],
}

#[repr(C)]
pub struct OpusDecoder {
    _p: [u8; 0],
}

extern "C" {
    pub fn opus_encoder_create(Fs: c_int, channels: c_int, application: c_int, error: *mut c_int) -> *mut OpusEncoder;
    /// ctl 请求是可变参数协议：每个请求的参数见 opus.h 的宏定义
    pub fn opus_encoder_ctl(st: *mut OpusEncoder, request: c_int, ...) -> c_int;
    pub fn opus_encode(st: *mut OpusEncoder, pcm: *const i16, frame_size: c_int, data: *mut u8, max_data_bytes: c_int) -> c_int;
    pub fn opus_encoder_destroy(st: *mut OpusEncoder);

    pub fn opus_decoder_create(Fs: c_int, channels: c_int, error: *mut c_int) -> *mut OpusDecoder;
    pub fn opus_decode(st: *mut OpusDecoder, data: *const u8, len: c_int, pcm: *mut i16, frame_size: c_int, decode_fec: c_int) -> c_int;
    pub fn opus_decoder_destroy(st: *mut OpusDecoder);
}

/// ctl 请求编号，取值来自 opus.h 的宏（与 ABI 无关，纯协议约定）
pub const OPUS_SET_BITRATE_REQUEST: c_int = 4002;
pub const OPUS_SET_COMPLEXITY_REQUEST: c_int = 4010;
pub const OPUS_SET_VBR_REQUEST: c_int = 4008;
pub const OPUS_GET_FINAL_RANGE_REQUEST: c_int = 4031;

#[cfg(test)]
mod tests {
    use super::*;

    /// 编码→解码往返：证明链接进来的静态库真的能干活，而不只是符号能找到
    #[test]
    fn roundtrip() {
        unsafe {
            let mut err: c_int = 0;
            let enc = opus_encoder_create(16000, 1, 2048 /* VOIP */, &mut err);
            assert!(!enc.is_null() && err == 0);
            opus_encoder_ctl(enc, OPUS_SET_BITRATE_REQUEST, 24000i32);

            // 320 样本（20ms @16k）的 440Hz 正弦
            let pcm: Vec<i16> = (0..320).map(|i| ((i as f32) * 0.1373f32).sin() * 8000.0).map(|v| v as i16).collect();
            let mut out = [0u8; 4000];
            let n = opus_encode(enc, pcm.as_ptr(), 320, out.as_mut_ptr(), 4000);
            assert!(n > 0, "encode 失败: {n}");

            let mut range: u32 = 0;
            opus_encoder_ctl(enc, OPUS_GET_FINAL_RANGE_REQUEST, &mut range);
            assert_ne!(range, 0, "编码器没有给出 range（bitstream 校验值）");
            opus_encoder_destroy(enc);

            let dec = opus_decoder_create(16000, 1, &mut err);
            assert!(!dec.is_null() && err == 0);
            let mut back = [0i16; 5760];
            let m = opus_decode(dec, out.as_ptr(), n, back.as_mut_ptr(), 5760, 0);
            assert_eq!(m, 320, "解码样本数不对: {m}");
            opus_decoder_destroy(dec);

            // Opus 有约 6.5ms 的前向延迟：解码波形整体比原始滞后一段，
            // 在重叠范围内找最优对齐后再比误差
            let mut best = i32::MAX;
            for lag in 0..250usize {
                let overlap = 320 - lag;
                let e = pcm[..overlap].iter().zip(&back[lag..]).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
                best = best.min(e);
            }
            assert!(best < 3000, "往返误差过大: {best}");
        }
    }
}
