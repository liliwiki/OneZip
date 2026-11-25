//! 计算文件校验值（右键菜单里的 "CRC SHA"，与 7-Zip 的同名菜单对应）。
//!
//! 文件夹会递归展开；每个文件只读一遍，同时喂给所有选中的算法。

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use sha2::Digest;

use crate::compress::Cancelled;
use crate::{Event, Observer, human};

/// 支持的算法。名字与 7-Zip 一致（另加常用的 MD5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashKind {
    Crc32,
    Crc64,
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl HashKind {
    pub const ALL: [HashKind; 6] = [
        HashKind::Crc32,
        HashKind::Crc64,
        HashKind::Md5,
        HashKind::Sha1,
        HashKind::Sha256,
        HashKind::Sha512,
    ];

    /// 显示用的名字，如 `SHA-256`。
    pub fn name(self) -> &'static str {
        match self {
            HashKind::Crc32 => "CRC-32",
            HashKind::Crc64 => "CRC-64",
            HashKind::Md5 => "MD5",
            HashKind::Sha1 => "SHA-1",
            HashKind::Sha256 => "SHA-256",
            HashKind::Sha512 => "SHA-512",
        }
    }

    /// 命令行和右键菜单用的短名，如 `sha256`。
    pub fn id(self) -> &'static str {
        match self {
            HashKind::Crc32 => "crc32",
            HashKind::Crc64 => "crc64",
            HashKind::Md5 => "md5",
            HashKind::Sha1 => "sha1",
            HashKind::Sha256 => "sha256",
            HashKind::Sha512 => "sha512",
        }
    }

    /// 解析逗号分隔的列表；`*` 或 `all` 表示全部。
    pub fn parse_list(s: &str) -> Option<Vec<HashKind>> {
        let s = s.trim();
        if s == "*" || s.eq_ignore_ascii_case("all") {
            return Some(Self::ALL.to_vec());
        }
        let mut out = Vec::new();
        for part in s.split(',') {
            let p = part.trim().to_ascii_lowercase().replace('-', "");
            let k = Self::ALL.into_iter().find(|k| k.id() == p)?;
            if !out.contains(&k) {
                out.push(k);
            }
        }
        (!out.is_empty()).then_some(out)
    }
}

/// 一个文件的结果。
#[derive(Debug, Clone)]
pub struct FileHash {
    /// 显示用的名字：选中的是文件就是文件名，选中文件夹时是 `文件夹\相对路径`。
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    /// 与请求的算法顺序一致，大写十六进制。
    pub digests: Vec<(HashKind, String)>,
}

/// 全部结果。
#[derive(Debug, Clone)]
pub struct HashReport {
    pub kinds: Vec<HashKind>,
    pub files: Vec<FileHash>,
    pub total_size: u64,
}

impl HashReport {
    /// 给人看的文本（结果窗口和复制到剪贴板都用它）。
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let width = self.kinds.iter().map(|k| k.name().len()).max().unwrap_or(0);
        for f in &self.files {
            s.push_str(&format!("名称：{}\n", f.name));
            s.push_str(&format!("大小：{} 字节（{}）\n", group(f.size), human(f.size)));
            for (k, d) in &f.digests {
                s.push_str(&format!("{:<width$}  {d}\n", k.name()));
            }
            s.push('\n');
        }
        if self.files.len() > 1 {
            s.push_str(&format!(
                "共 {} 个文件，{} 字节（{}）\n",
                self.files.len(),
                group(self.total_size),
                human(self.total_size)
            ));
        }
        s
    }
}

/// `1234567` → `1,234,567`
fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

enum Hasher {
    Crc32(crc32fast::Hasher),
    Crc64(crc::Digest<'static, u64>),
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

static CRC64: crc::Crc<u64> = crc::Crc::<u64>::new(&crc::CRC_64_XZ);

impl Hasher {
    fn new(kind: HashKind) -> Self {
        match kind {
            HashKind::Crc32 => Hasher::Crc32(crc32fast::Hasher::new()),
            HashKind::Crc64 => Hasher::Crc64(CRC64.digest()),
            HashKind::Md5 => Hasher::Md5(md5::Md5::new()),
            HashKind::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
            HashKind::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            HashKind::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Crc32(h) => h.update(data),
            Hasher::Crc64(h) => h.update(data),
            Hasher::Md5(h) => h.update(data),
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
            Hasher::Sha512(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        match self {
            Hasher::Crc32(h) => format!("{:08X}", h.finalize()),
            Hasher::Crc64(h) => format!("{:016X}", h.finalize()),
            Hasher::Md5(h) => hex(&h.finalize()),
            Hasher::Sha1(h) => hex(&h.finalize()),
            Hasher::Sha256(h) => hex(&h.finalize()),
            Hasher::Sha512(h) => hex(&h.finalize()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// 展开选中的路径：文件原样，文件夹递归（按名字排序）。
fn expand(paths: &[PathBuf]) -> Result<Vec<(String, PathBuf, u64)>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf, u64)>) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("读取 {}", dir.display()))?
            .collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = format!("{prefix}\\{}", e.file_name().to_string_lossy());
            let meta = std::fs::symlink_metadata(e.path())?;
            if meta.is_dir() {
                walk(&e.path(), &name, out)?;
            } else if meta.is_file() {
                out.push((name, e.path(), meta.len()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        let meta = std::fs::metadata(p).with_context(|| format!("读取 {}", p.display()))?;
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.display().to_string());
        if meta.is_dir() {
            walk(p, &name, &mut out)?;
        } else {
            out.push((name, p.clone(), meta.len()));
        }
    }
    Ok(out)
}

/// 计算 `paths`（文件或文件夹）的校验值。`stop` 置位时尽快返回 [`Cancelled`]。
pub fn hash(
    paths: &[PathBuf],
    kinds: &[HashKind],
    observer: Observer,
    stop: &Arc<AtomicBool>,
) -> Result<HashReport> {
    if kinds.is_empty() {
        bail!("没有选择算法");
    }
    let files = expand(paths)?;
    let total_size: u64 = files.iter().map(|f| f.2).sum();
    observer(Event::Started {
        total: total_size,
        resuming: false,
    });
    let mut buf = vec![0u8; 1 << 20];
    let mut out = Vec::with_capacity(files.len());
    for (name, path, _) in files {
        let mut file = File::open(&path).with_context(|| format!("打开 {}", path.display()))?;
        let mut hashers: Vec<Hasher> = kinds.iter().map(|&k| Hasher::new(k)).collect();
        let mut size = 0u64;
        loop {
            if stop.load(Ordering::Relaxed) {
                bail!(Cancelled);
            }
            let n = file
                .read(&mut buf)
                .with_context(|| format!("读取 {}", path.display()))?;
            if n == 0 {
                break;
            }
            for h in &mut hashers {
                h.update(&buf[..n]);
            }
            size += n as u64;
            observer(Event::Progressed(n as u64));
        }
        let digests = kinds
            .iter()
            .copied()
            .zip(hashers.into_iter().map(Hasher::finish))
            .collect();
        out.push(FileHash {
            name,
            path,
            size,
            digests,
        });
    }
    Ok(HashReport {
        kinds: kinds.to_vec(),
        total_size: out.iter().map(|f| f.size).sum(),
        files: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(kind: HashKind, data: &[u8]) -> String {
        let mut h = Hasher::new(kind);
        h.update(data);
        h.finish()
    }

    #[test]
    fn known_vectors() {
        let d = b"123456789";
        assert_eq!(digest(HashKind::Crc32, d), "CBF43926");
        // 7-Zip 的 CRC-64 是 ECMA-182（XZ 用的那个）。
        assert_eq!(digest(HashKind::Crc64, d), "995DC9BBDF1939FA");
        assert_eq!(digest(HashKind::Md5, b"abc"), "900150983CD24FB0D6963F7D28E17F72");
        assert_eq!(digest(HashKind::Sha1, b"abc"), "A9993E364706816ABA3E25717850C26C9CD0D89D");
        assert_eq!(
            digest(HashKind::Sha256, b"abc"),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
        assert!(digest(HashKind::Sha512, b"abc").starts_with("DDAF35A193617ABA"));
    }

    #[test]
    fn parse() {
        assert_eq!(HashKind::parse_list("*").unwrap().len(), 6);
        assert_eq!(
            HashKind::parse_list("sha-256, crc32").unwrap(),
            vec![HashKind::Sha256, HashKind::Crc32]
        );
        assert!(HashKind::parse_list("nope").is_none());
        assert_eq!(group(1234567), "1,234,567");
    }
}
