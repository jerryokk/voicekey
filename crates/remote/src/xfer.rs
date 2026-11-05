//! 「发到电脑」的接收端：把 DataChannel `xfer` 上的帧拼成剪贴板文字、文字、或下载文件夹里的文件。
//!
//! 文件名来自手机，是不可信输入：去掉路径分隔符与 Windows 不认的字符、躲开保留名、限长，
//! 文件只会落在下载文件夹本身，不会跑到别处。先写 `.part`，收完才改名，断线或取消就删掉。
//! 落盘的文件带「来自网络」标记（Zone.Identifier），和浏览器下载的一样，运行前系统会提醒。

use crate::protocol::{XferDown, XferKind, XferUp};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 单个文件的上限
pub const MAX_FILE: u64 = 2 << 30;
/// 文字、剪贴板的上限
pub const MAX_TEXT: u64 = 1 << 20;
/// 下载文件夹至少留这么多空闲：别把用户的盘写满
const DISK_MARGIN: u64 = 256 << 20;
/// 文件名最长（字符）
const NAME_MAX: usize = 120;

/// 收好的一项，交给上层展示
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Clip { text: String },
    Text { text: String },
    File { path: PathBuf, size: u64, mime: Option<String>, thumb: Option<String> },
}

/// 一组里收好的一项。`batch` 相同的归成一张卡片，`of` 是这一组一共几项
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub batch: u32,
    pub of: u32,
    pub item: Item,
}

enum Sink {
    Text(Vec<u8>),
    File { part: PathBuf, file: File, name: String },
}

struct Current {
    id: u32,
    kind: XferKind,
    size: u64,
    got: u64,
    batch: u32,
    of: u32,
    mime: Option<String>,
    thumb: Option<String>,
    sink: Sink,
}

/// 一条 xfer 通道上的接收状态：同一时间只收一项（通道是有序的，手机端也逐个发）
pub struct Receiver {
    dir: PathBuf,
    cur: Option<Current>,
}

/// 处理一帧之后要做的事
#[derive(Debug, Default, PartialEq)]
pub struct Step {
    /// 回给手机
    pub reply: Option<String>,
    /// 交给上层
    pub done: Option<Received>,
}

impl Step {
    fn reply(r: XferDown) -> Self {
        Self { reply: Some(r.json()), done: None }
    }
}

impl Receiver {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, cur: None }
    }

    /// 文本帧
    pub fn on_text(&mut self, text: &str) -> Step {
        let Ok(up) = serde_json::from_str::<XferUp>(text) else {
            return Step::default();
        };
        match up {
            XferUp::Begin { id, kind, name, mime, size, batch, of, thumb } => {
                // 上一项没收完就开始新的：手机端重发了，旧的作废
                self.discard();
                match self.begin(kind, name, size) {
                    Ok(sink) => {
                        self.cur = Some(Current { id, kind, size, got: 0, batch, of, mime, thumb, sink });
                        Step::default()
                    }
                    Err(message) => Step::reply(XferDown::Fail { id, message }),
                }
            }
            XferUp::End { id } => self.end(id),
            XferUp::Abort { id } => {
                if self.cur.as_ref().is_some_and(|c| c.id == id) {
                    self.discard();
                }
                Step::default()
            }
        }
    }

    /// 二进制帧：当前这一项的数据
    pub fn on_binary(&mut self, data: &[u8]) -> Step {
        let Some(c) = self.cur.as_mut() else { return Step::default() };
        c.got += data.len() as u64;
        let id = c.id;
        if c.got > c.size {
            self.discard();
            return Step::reply(XferDown::Fail { id, message: "收到的数据比声明的多".into() });
        }
        let r = match &mut c.sink {
            Sink::Text(buf) => {
                buf.extend_from_slice(data);
                Ok(())
            }
            Sink::File { file, .. } => file.write_all(data).map_err(|e| format!("写入失败：{e}")),
        };
        if let Err(message) = r {
            self.discard();
            return Step::reply(XferDown::Fail { id, message });
        }
        Step::default()
    }

    fn begin(&self, kind: XferKind, name: Option<String>, size: u64) -> Result<Sink, String> {
        match kind {
            XferKind::Clip | XferKind::Text => {
                if size > MAX_TEXT {
                    return Err("文字太长了（上限 1MB）".into());
                }
                Ok(Sink::Text(Vec::with_capacity(size as usize)))
            }
            XferKind::File => {
                if size > MAX_FILE {
                    return Err("文件太大了（上限 2GB）".into());
                }
                std::fs::create_dir_all(&self.dir).map_err(|e| format!("下载文件夹不可用：{e}"))?;
                if let Some(free) = free_space(&self.dir) {
                    if free < size.saturating_add(DISK_MARGIN) {
                        return Err("电脑的磁盘空间不够".into());
                    }
                }
                let name = clean_name(name.as_deref().unwrap_or(""));
                // 临时名用进程内序号：每台手机的 id 都从 1 开始，靠 id 会撞名
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let n = NEXT.fetch_add(1, Ordering::Relaxed);
                let part = self.dir.join(format!(".voicekey-{}-{n}.part", std::process::id()));
                let file = File::create(&part).map_err(|e| format!("无法创建文件：{e}"))?;
                Ok(Sink::File { part, file, name })
            }
        }
    }

    fn end(&mut self, id: u32) -> Step {
        let Some(c) = self.cur.take_if(|c| c.id == id) else { return Step::default() };
        if c.got != c.size {
            let message = format!("文件不完整（{}/{} 字节）", c.got, c.size);
            drop_sink(c.sink);
            return Step::reply(XferDown::Fail { id, message });
        }
        let item = match c.sink {
            Sink::Text(buf) => match String::from_utf8(buf) {
                Ok(text) if c.kind == XferKind::Clip => Item::Clip { text },
                Ok(text) => Item::Text { text },
                Err(_) => return Step::reply(XferDown::Fail { id, message: "文字不是 UTF-8".into() }),
            },
            Sink::File { part, file, name } => {
                let r = file.sync_all().map_err(|e| format!("写入失败：{e}"));
                drop(file);
                match r.and_then(|()| settle(&part, &self.dir, &name)) {
                    Ok(path) => Item::File { path, size: c.size, mime: c.mime, thumb: c.thumb },
                    Err(message) => {
                        let _ = std::fs::remove_file(&part);
                        return Step::reply(XferDown::Fail { id, message });
                    }
                }
            }
        };
        Step {
            reply: Some(XferDown::Ok { id }.json()),
            done: Some(Received { batch: c.batch, of: c.of, item }),
        }
    }

    /// 丢掉正在收的那一项（取消、断线、出错），已写的半个文件删掉
    pub fn discard(&mut self) {
        if let Some(c) = self.cur.take() {
            drop_sink(c.sink);
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.discard();
    }
}

fn drop_sink(sink: Sink) {
    if let Sink::File { part, file, .. } = sink {
        drop(file);
        let _ = std::fs::remove_file(part);
    }
}

/// `.part` 改成正式名字。重名就按浏览器的习惯编号：`名字 (1).扩展名`。
/// 先用 `create_new` 占住名字再改名覆盖它：几台手机同时发同名文件也不会互相顶掉
fn settle(part: &Path, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let (stem, ext) = split_ext(name);
    for n in 0..1000 {
        let candidate = if n == 0 { name.to_string() } else { format!("{stem} ({n}){ext}") };
        let path = dir.join(&candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => {
                std::fs::rename(part, &path).map_err(|e| {
                    let _ = std::fs::remove_file(&path);
                    format!("保存失败：{e}")
                })?;
                mark_from_internet(&path);
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("保存失败：{e}")),
        }
    }
    Err("同名文件太多了".into())
}

/// 和浏览器下载的文件一样标成「来自网络」：别人发来的可执行文件，运行前系统会拦一下。
/// 写不上（比如 FAT32 U 盘不支持备用数据流）就算了，不影响收文件
fn mark_from_internet(path: &Path) {
    let mut ads = path.as_os_str().to_owned();
    ads.push(":Zone.Identifier");
    let _ = std::fs::write(PathBuf::from(ads), b"[ZoneTransfer]\r\nZoneId=3\r\n");
}

/// 手机给的文件名 → 能安全放进下载文件夹的文件名
pub fn clean_name(raw: &str) -> String {
    // 只取最后一段：带目录的名字一律不认目录
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = base
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    // Windows 会悄悄去掉结尾的点和空格，开头的点会让文件变成「隐藏」的样子
    s = s.trim_matches(|c: char| c == ' ' || c == '.').to_string();
    if s.is_empty() {
        s = "来自手机".into();
    }
    let (stem, ext) = split_ext(&s);
    let reserved = ["CON", "PRN", "AUX", "NUL"].iter().any(|r| stem.eq_ignore_ascii_case(r))
        || (stem.len() == 4
            && ["COM", "LPT"].iter().any(|p| stem[..3].eq_ignore_ascii_case(p))
            && stem.as_bytes()[3].is_ascii_digit());
    let stem = if reserved { format!("_{stem}") } else { stem.to_string() };
    // 限长时保住扩展名：截掉扩展名，文件就打不开了
    let ext: String = ext.chars().take(16).collect();
    let keep = NAME_MAX.saturating_sub(ext.chars().count()).max(1);
    let stem: String = stem.chars().take(keep).collect();
    format!("{stem}{ext}")
}

/// `a.tar.gz` → (`a.tar`, `.gz`)；没有扩展名或以点开头的整个当名字
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

#[cfg(windows)]
fn free_space(dir: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(dir.as_os_str()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个测试一个临时目录，测试结束（含失败）自动删掉
    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!("vk-xfer-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Self(d)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn begin(id: u32, kind: &str, name: &str, size: u64) -> String {
        format!(r#"{{"k":"begin","id":{id},"kind":"{kind}","name":"{name}","size":{size},"batch":1,"of":1}}"#)
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(clean_name("photo.jpg"), "photo.jpg");
        assert_eq!(clean_name("..\\..\\Windows\\evil.exe"), "evil.exe", "不认目录");
        assert_eq!(clean_name("a/b/c.txt"), "c.txt");
        assert_eq!(clean_name("报价:v2?.xlsx"), "报价_v2_.xlsx");
        assert_eq!(clean_name("CON.txt"), "_CON.txt");
        assert_eq!(clean_name("com1"), "_com1");
        assert_eq!(clean_name("command.txt"), "command.txt", "只挡保留名本身");
        assert_eq!(clean_name("  ..  "), "来自手机");
        assert_eq!(clean_name(""), "来自手机");
        assert_eq!(clean_name("tail. "), "tail");
        let long = format!("{}.pdf", "长".repeat(300));
        let c = clean_name(&long);
        assert!(c.ends_with(".pdf") && c.chars().count() == NAME_MAX, "限长保住扩展名");
    }

    #[test]
    fn receives_text_and_clipboard() {
        let t = Tmp::new("text");
        let mut r = Receiver::new(t.0.clone());
        assert_eq!(r.on_text(&begin(1, "text", "", 6)), Step::default());
        r.on_binary("你好".as_bytes());
        let s = r.on_text(r#"{"k":"end","id":1}"#);
        assert_eq!(s.reply.as_deref(), Some(r#"{"k":"ok","id":1}"#));
        assert_eq!(s.done.unwrap().item, Item::Text { text: "你好".into() });

        r.on_text(&begin(2, "clip", "", 1));
        r.on_binary(b"x");
        assert_eq!(r.on_text(r#"{"k":"end","id":2}"#).done.unwrap().item, Item::Clip { text: "x".into() });
    }

    #[test]
    fn saves_file_and_numbers_duplicates() {
        let t = Tmp::new("file");
        let dir = t.0.clone();
        let mut r = Receiver::new(dir.clone());
        for want in ["a.txt", "a (1).txt"] {
            r.on_text(&begin(7, "file", "a.txt", 3));
            r.on_binary(b"abc");
            let s = r.on_text(r#"{"k":"end","id":7}"#);
            let Some(Received { item: Item::File { path, size, .. }, .. }) = s.done else { panic!("{s:?}") };
            assert_eq!(path, dir.join(want));
            assert_eq!(size, 3);
            assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        }
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".part")).collect();
        assert!(left.is_empty(), "不留 .part");
    }

    #[test]
    fn aborted_short_and_oversized_transfers_leave_nothing() {
        let t = Tmp::new("abort");
        let dir = t.0.clone();
        let mut r = Receiver::new(dir.clone());
        r.on_text(&begin(1, "file", "x.bin", 4));
        r.on_binary(b"ab");
        r.on_text(r#"{"k":"abort","id":1}"#);

        r.on_text(&begin(2, "file", "y.bin", 4));
        r.on_binary(b"ab");
        let s = r.on_text(r#"{"k":"end","id":2}"#);
        assert!(s.reply.unwrap().contains("fail"), "少了数据要报失败");

        r.on_text(&begin(3, "file", "z.bin", 1));
        assert!(r.on_binary(b"ab").reply.unwrap().contains("fail"), "多了数据也是");

        r.on_text(&begin(4, "file", "w.bin", 4));
        r.on_binary(b"ab");
        drop(r);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "取消、出错、断线都不留文件");
    }

    #[test]
    fn limits_are_enforced() {
        let t = Tmp::new("limit");
        let mut r = Receiver::new(t.0.clone());
        assert!(r.on_text(&begin(1, "text", "", MAX_TEXT + 1)).reply.unwrap().contains("fail"));
        assert!(r.on_text(&begin(2, "file", "a", MAX_FILE + 1)).reply.unwrap().contains("fail"));
        assert_eq!(r.on_binary(b"stray"), Step::default(), "没有开始的项，数据丢掉");
        assert_eq!(r.on_text("not json"), Step::default());
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let t = Tmp::new("utf8");
        let mut r = Receiver::new(t.0.clone());
        r.on_text(&begin(1, "text", "", 2));
        r.on_binary(&[0xff, 0xfe]);
        assert!(r.on_text(r#"{"k":"end","id":1}"#).reply.unwrap().contains("fail"));
    }
}
