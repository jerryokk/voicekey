use anyhow::{bail, Result};
use opus_sys as ffi;

pub const VOIP: i32 = 2048;
pub const AUDIO: i32 = 2049;
const OPUS_OK: i32 = 0;

/// 16kHz 单声道 Opus 编码器
pub struct Opus {
    enc: *mut ffi::OpusEncoder,
    buf: Vec<u8>,
}

// 编码器状态只经 &mut self 访问
unsafe impl Send for Opus {}

impl Opus {
    pub fn new(application: i32, bitrate: i32, complexity: i32) -> Result<Self> {
        let mut err = 0;
        let enc = unsafe { ffi::opus_encoder_create(16000, 1, application, &mut err) };
        if enc.is_null() || err != OPUS_OK {
            bail!("Opus 编码器创建失败 {err}");
        }
        // 先接管指针，后续 ctl 失败时由 Drop 释放
        let o = Self { enc, buf: vec![0; 4000] };
        o.set(ffi::OPUS_SET_BITRATE_REQUEST, bitrate)?;
        o.set(ffi::OPUS_SET_COMPLEXITY_REQUEST, complexity)?;
        Ok(o)
    }

    /// 百度输入法：CBR 32kbps
    pub fn baidu() -> Result<Self> {
        let o = Self::new(AUDIO, 32000, 10)?;
        o.set(ffi::OPUS_SET_VBR_REQUEST, 0)?;
        Ok(o)
    }

    fn set(&self, request: i32, value: i32) -> Result<()> {
        let r = unsafe { ffi::opus_encoder_ctl(self.enc, request, value) };
        if r != OPUS_OK {
            bail!("Opus 设置 {request} 失败 {r}");
        }
        Ok(())
    }

    fn encode_raw(&mut self, pcm: &[i16]) -> Result<&[u8]> {
        let n = unsafe {
            ffi::opus_encode(self.enc, pcm.as_ptr(), pcm.len() as i32, self.buf.as_mut_ptr(), self.buf.len() as i32)
        };
        if n <= 0 {
            bail!("Opus 编码失败 {n}");
        }
        Ok(&self.buf[..n as usize])
    }

    pub fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>> {
        self.encode_raw(pcm).map(<[u8]>::to_vec)
    }

    /// 百度帧：BE32(len) + BE32(OPUS_GET_FINAL_RANGE) + opus
    pub fn encode_bd(&mut self, pcm: &[i16]) -> Result<Vec<u8>> {
        let n = self.encode_raw(pcm)?.len();
        let mut range: u32 = 0;
        let r = unsafe { ffi::opus_encoder_ctl(self.enc, ffi::OPUS_GET_FINAL_RANGE_REQUEST, &mut range as *mut u32) };
        if r != OPUS_OK {
            bail!("Opus 取 final range 失败 {r}");
        }
        let mut v = Vec::with_capacity(8 + n);
        v.extend_from_slice(&(n as u32).to_be_bytes());
        v.extend_from_slice(&range.to_be_bytes());
        v.extend_from_slice(&self.buf[..n]);
        Ok(v)
    }
}

impl Drop for Opus {
    fn drop(&mut self) {
        unsafe { ffi::opus_encoder_destroy(self.enc) }
    }
}
