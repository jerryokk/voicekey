//! 官方 .xnet 模型文件：只读算子（权重/常量）记录表位于文件尾部，按名称索引
use anyhow::{bail, Context, Result};
use std::collections::HashMap;

pub const F32: u32 = 11;
pub const I8: u32 = 6;

pub struct Rec {
    pub dtype: u32,
    pub scale: Option<f32>,
    pub dims: Vec<usize>,
    pub off: usize,
    pub len: usize,
}

struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl Cur<'_> {
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let b = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(b)
    }
}

fn elem(dtype: u32) -> Option<usize> {
    match dtype {
        I8 | 2 | 3 => Some(1),
        4 | 5 => Some(2),
        1 | 7 | 8 | F32 => Some(4),
        9 | 10 | 12 => Some(8),
        _ => None,
    }
}

/// 从 p 起逐条解析，必须恰好解析到文件尾（零长度名 + 全零填充）
fn chain(d: &[u8], p: usize) -> Option<HashMap<String, Rec>> {
    let mut c = Cur { d, p };
    let mut out = HashMap::new();
    loop {
        let n = c.u32()? as usize;
        if n == 0 {
            let rest = &d[c.p..];
            return (rest.len() <= 16 && rest.iter().all(|&b| b == 0) && out.len() > 100).then_some(out);
        }
        if n > 256 {
            return None;
        }
        let name = std::str::from_utf8(c.take(n)?).ok()?.to_string();
        let dtype = c.u32()?;
        let es = elem(dtype)?;
        let quant = c.u32()?;
        for _ in 0..3 {
            c.u32()?;
        }
        let ns = c.u32()? as usize;
        c.u32()?;
        if ns > 4096 {
            return None;
        }
        let sc = c.take(ns * 4)?;
        let scale = (quant != 0 && ns > 0).then(|| f32::from_le_bytes(sc[..4].try_into().unwrap()));
        let nz = c.u32()? as usize;
        c.take(nz)?;
        let nd = c.u32()? as usize;
        if nd > 8 {
            return None;
        }
        let dims = (0..nd).map(|_| c.u32().map(|v| v as usize)).collect::<Option<Vec<_>>>()?;
        let len = c.u32()? as usize;
        // 维度来自文件，乘积可能溢出
        let size = dims.iter().try_fold(es, |a, &v| a.checked_mul(v));
        if size != Some(len) {
            return None;
        }
        let off = c.p;
        c.take(len)?;
        out.insert(name, Rec { dtype, scale, dims, off, len });
    }
}

pub fn parse(d: &[u8]) -> Result<HashMap<String, Rec>> {
    if d.get(..4) != Some(b"XNET") {
        bail!("不是 xnet 模型文件");
    }
    // 记录表首条是 u32 长度（1..=128）+ 可打印 ASCII 名称；逐个候选位置尝试完整解析到文件尾
    let mut p = 8;
    while let Some(i) = d[p..].windows(4).position(|w| w[1..] == [0, 0, 0] && (1..=128).contains(&w[0])) {
        let at = p + i;
        let n = d[at] as usize;
        if let Some(name) = d.get(at + 4..).and_then(|r| r.get(..n)) {
            if name.iter().all(|b| b.is_ascii_graphic()) {
                if let Some(m) = chain(d, at) {
                    return Ok(m);
                }
            }
        }
        p = at + 1;
    }
    None.context("找不到 xnet 权重表")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一条 int8、无 scale/零点的记录头：名称、dtype、quant、3 个保留、ns、保留、nz、nd、dims、len
    fn rec(name: &str, dims: &[u32], len: u32) -> Vec<u8> {
        let mut v = (name.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(name.as_bytes());
        for x in [I8, 0, 0, 0, 0, 0, 0, 0, dims.len() as u32].into_iter().chain(dims.iter().copied()).chain([len]) {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    #[test]
    fn dims_overflow_is_rejected() {
        let mut d = rec("w", &[u32::MAX, u32::MAX, u32::MAX], 0);
        d.extend_from_slice(&[0; 4]);
        assert!(chain(&d, 0).is_none());
    }

    #[test]
    fn truncated_record_is_rejected() {
        let d = rec("w", &[4], 4);
        assert!(chain(&d, 0).is_none());
        assert!(chain(&d[..3], 0).is_none());
    }
}
