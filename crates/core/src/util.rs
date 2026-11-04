use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;
use std::sync::LazyLock;

static DATA_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let d = dirs::data_dir().expect("取不到 %APPDATA%").join("VoiceKey");
    std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("无法创建数据目录 {}：{e}", d.display()));
    d
});

/// %APPDATA%\VoiceKey，首次访问时创建
pub fn data_dir() -> PathBuf {
    DATA_DIR.clone()
}

pub fn data_file(name: &str) -> PathBuf {
    DATA_DIR.join(name)
}

/// 渠道的设备身份，以 JSON 存在数据目录里，生成一次后复用。
/// 文件不存在或内容解析不了都视为尚未生成
pub(crate) struct IdFile(pub &'static str);

impl IdFile {
    pub fn load<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_slice(&std::fs::read(data_file(self.0)).ok()?).ok()
    }

    pub fn save<T: Serialize>(&self, v: &T) -> Result<()> {
        std::fs::write(data_file(self.0), serde_json::to_vec(v)?)?;
        Ok(())
    }

    pub fn remove(&self) -> Result<()> {
        match std::fs::remove_file(data_file(self.0)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    pub fn get_or_create<T: Serialize + DeserializeOwned>(&self, make: impl FnOnce() -> T) -> Result<T> {
        if let Some(v) = self.load() {
            return Ok(v);
        }
        let v = make();
        self.save(&v)?;
        Ok(v)
    }
}

/// 1970-01-01 起的天数换成公历（年, 月 1-12, 日 1-31）。Howard Hinnant 的 civil_from_days
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

pub(crate) fn rand_lower(n: usize) -> String {
    use rand::Rng;
    let mut r = rand::thread_rng();
    (0..n).map(|_| r.gen_range(b'a'..=b'z') as char).collect()
}

/// RFC 3986 百分号编码：只保留非保留字符
pub(crate) fn urlencode(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_from_days_matches_known_dates() {
        for (days, want) in [
            (0, (1970, 1, 1)),
            (-1, (1969, 12, 31)),
            (19_782, (2024, 2, 29)), // 闰日
            (20_089, (2025, 1, 1)),  // 跨年
            (47_482, (2100, 1, 1)),  // 2100 不是闰年
            (47_541, (2100, 3, 1)),
        ] {
            assert_eq!(super::civil_from_days(days), want, "days = {days}");
        }
    }

    #[test]
    fn urlencode_keeps_unreserved_only() {
        assert_eq!(super::urlencode("aZ0-._~ +/=中"), "aZ0-._~%20%2B%2F%3D%E4%B8%AD");
    }
}
