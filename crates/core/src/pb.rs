use anyhow::{bail, Result};

pub fn uvarint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// 极简 protobuf 编码
#[derive(Default)]
pub struct PBuf(pub Vec<u8>);

impl PBuf {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn v(mut self, field: u64, value: u64) -> Self {
        uvarint(&mut self.0, field << 3);
        uvarint(&mut self.0, value);
        self
    }
    pub fn b(mut self, field: u64, value: &[u8]) -> Self {
        uvarint(&mut self.0, field << 3 | 2);
        uvarint(&mut self.0, value.len() as u64);
        self.0.extend_from_slice(value);
        self
    }
    pub fn s(self, field: u64, value: &str) -> Self {
        self.b(field, value.as_bytes())
    }
    pub fn m(self, field: u64, sub: PBuf) -> Self {
        self.b(field, &sub.0)
    }
    /// 子消息 {1: k, 2: v}
    pub fn kv(self, field: u64, k: &str, v: &str) -> Self {
        self.m(field, PBuf::new().s(1, k).s(2, v))
    }
}

/// 定长字段（wire type 1/5）只跳过不保留
pub enum Val {
    Varint(u64),
    Bytes(Vec<u8>),
}

pub struct Fields(pub Vec<(u64, Val)>);

pub fn read_varint(buf: &[u8], i: &mut usize) -> Result<u64> {
    let (mut v, mut shift) = (0u64, 0u32);
    loop {
        if *i >= buf.len() || shift >= 64 {
            bail!("protobuf varint 截断");
        }
        let b = buf[*i];
        *i += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b < 0x80 {
            return Ok(v);
        }
        shift += 7;
    }
}

/// 从 i 前进 n 字节后的位置；越界（含加法溢出）报错
fn advance(buf: &[u8], i: usize, n: usize) -> Result<usize> {
    match i.checked_add(n) {
        Some(end) if end <= buf.len() => Ok(end),
        _ => bail!("protobuf 字段截断"),
    }
}

pub fn parse(buf: &[u8]) -> Result<Fields> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let tag = read_varint(buf, &mut i)?;
        let field = tag >> 3;
        match tag & 7 {
            0 => out.push((field, Val::Varint(read_varint(buf, &mut i)?))),
            1 => i = advance(buf, i, 8)?,
            5 => i = advance(buf, i, 4)?,
            2 => {
                let n = usize::try_from(read_varint(buf, &mut i)?)?;
                let end = advance(buf, i, n)?;
                out.push((field, Val::Bytes(buf[i..end].to_vec())));
                i = end;
            }
            _ => bail!("不支持的 protobuf wire type"),
        }
    }
    Ok(Fields(out))
}

impl Fields {
    pub fn bytes(&self, f: u64) -> Option<&[u8]> {
        self.0.iter().find_map(|(k, v)| match v {
            Val::Bytes(b) if *k == f => Some(b.as_slice()),
            _ => None,
        })
    }
    pub fn varint(&self, f: u64) -> Option<u64> {
        self.0.iter().find_map(|(k, v)| match v {
            Val::Varint(n) if *k == f => Some(*n),
            _ => None,
        })
    }
    pub fn string(&self, f: u64) -> Option<String> {
        self.bytes(f).map(|b| String::from_utf8_lossy(b).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let buf = PBuf::new().v(1, 300).s(2, "你好").m(3, PBuf::new().v(1, 7)).0;
        let f = parse(&buf).unwrap();
        assert_eq!(f.varint(1), Some(300));
        assert_eq!(f.string(2).as_deref(), Some("你好"));
        assert_eq!(parse(f.bytes(3).unwrap()).unwrap().varint(1), Some(7));
    }

    #[test]
    fn huge_length_is_error_not_overflow() {
        // 字段 1、wire type 2，长度 u64::MAX：i + n 会溢出
        let mut buf = vec![0x0a];
        uvarint(&mut buf, u64::MAX);
        assert!(parse(&buf).is_err());
    }

    #[test]
    fn truncated_fields_are_errors() {
        assert!(parse(&[0x0a, 0x05, 1, 2]).is_err());
        assert!(parse(&[0x09, 1, 2, 3]).is_err());
        assert!(parse(&[0x0d, 1, 2, 3]).is_err());
        assert!(parse(&[0x08, 0x80]).is_err());
    }
}
