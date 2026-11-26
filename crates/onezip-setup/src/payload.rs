//! 安装程序后面接的文件。
//!
//! 发布的 `OneZip-<版本>-Setup.exe` = 本程序 + 一个 zip + 16 字节尾标（zip 的长度 u64 小端 + [`MAGIC`]）。
//! zip 里是要装到安装目录的文件；其中 `setup\` 文件夹只给安装程序自己用（右键菜单的稀疏包、
//! 签名证书），不复制过去。卸载程序 `uninstall.exe` 就是本程序去掉后面这一截。

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub const MAGIC: &[u8; 8] = b"OZSETUP1";

/// zip 里只给安装程序用的文件夹。
pub const SETUP_DIR: &str = "setup";

/// 本程序后面接的 zip 在文件里的起点和长度。
pub fn locate(exe: &mut File) -> Result<(u64, u64)> {
    let total = exe.seek(SeekFrom::End(0))?;
    if total < 16 {
        bail!("安装程序不完整");
    }
    exe.seek(SeekFrom::End(-16))?;
    let mut tail = [0u8; 16];
    exe.read_exact(&mut tail)?;
    if &tail[8..] != MAGIC {
        bail!("安装程序里没有找到要安装的文件（安装程序不完整，请重新下载）");
    }
    let len = u64::from_le_bytes(tail[..8].try_into().expect("8 字节"));
    if len > total - 16 {
        bail!("安装程序已损坏，请重新下载");
    }
    Ok((total - 16 - len, len))
}

/// 文件中的一段，当作独立的文件来读。
pub struct Slice {
    pub file: File,
    pub start: u64,
    pub len: u64,
    pub pos: u64,
}

impl Read for Slice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        let n = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        if n == 0 {
            return Ok(0);
        }
        self.file.seek(SeekFrom::Start(self.start + self.pos))?;
        let n = self.file.read(&mut buf[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Slice {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "位置越界"))?;
        Ok(self.pos)
    }
}

/// 把 `exe` 后面接的 zip 解到 `to`。
pub fn unpack(exe: &Path, to: &Path) -> Result<()> {
    let mut file = File::open(exe).with_context(|| format!("读取 {}", exe.display()))?;
    let (start, len) = locate(&mut file)?;
    let mut zip = zip::ZipArchive::new(Slice { file, start, len, pos: 0 }).context("安装程序已损坏，请重新下载")?;
    zip.extract(to).context("解出安装文件失败")?;
    Ok(())
}

/// 写出卸载程序：`exe` 去掉后面接的 zip。
pub fn write_stub(exe: &Path, to: &Path) -> Result<()> {
    let mut file = File::open(exe)?;
    let (start, _) = locate(&mut file)?;
    file.seek(SeekFrom::Start(0))?;
    let mut out = File::create(to).with_context(|| format!("写入 {}", to.display()))?;
    io::copy(&mut file.take(start), &mut out)?;
    Ok(())
}

/// 解出的文件里要装到安装目录的那些，相对路径（用 `\` 分隔），排好序。
pub fn program_files(unpacked: &Path) -> Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let p = e.path();
            let rel = p.strip_prefix(root).expect("在根目录下").to_string_lossy().replace('/', "\\");
            if e.file_type()?.is_dir() {
                if !rel.eq_ignore_ascii_case(SETUP_DIR) {
                    walk(root, &p, out)?;
                }
            } else {
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(unpacked, unpacked, &mut out)?;
    out.sort();
    if !out.iter().any(|f| f.eq_ignore_ascii_case("OneZip.exe")) {
        bail!("安装文件里缺少 OneZip.exe");
    }
    Ok(out)
}

/// 安装包里只给安装程序用的某个文件（不存在时为 `None`）。
pub fn setup_file(unpacked: &Path, name: &str) -> Option<PathBuf> {
    Some(unpacked.join(SETUP_DIR).join(name)).filter(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_the_appended_zip_and_writes_the_stub() {
        let dir = std::env::temp_dir().join(format!("onezip-setup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut buf = io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            for name in ["OneZip.exe", "licenses/LICENSE.txt", "setup/OneZip-Shell.msix"] {
                z.start_file(name, o).unwrap();
                z.write_all(name.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        let zip_bytes = buf.into_inner();
        let stub = b"MZ not really an exe";
        let path = dir.join("stub.exe");
        let mut f = File::create(&path).unwrap();
        f.write_all(stub).unwrap();
        f.write_all(&zip_bytes).unwrap();
        f.write_all(&(zip_bytes.len() as u64).to_le_bytes()).unwrap();
        f.write_all(MAGIC).unwrap();
        drop(f);

        assert_eq!(locate(&mut File::open(&path).unwrap()).unwrap(), (stub.len() as u64, zip_bytes.len() as u64));
        let out = dir.join("out");
        unpack(&path, &out).unwrap();
        assert_eq!(program_files(&out).unwrap(), ["OneZip.exe", "licenses\\LICENSE.txt"]);
        assert!(setup_file(&out, "OneZip-Shell.msix").is_some());
        assert!(setup_file(&out, "OneZip-Dev.cer").is_none());

        write_stub(&path, &dir.join("uninstall.exe")).unwrap();
        assert_eq!(std::fs::read(dir.join("uninstall.exe")).unwrap(), stub);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_an_exe_without_payload() {
        let path = std::env::temp_dir().join(format!("onezip-setup-empty-{}", std::process::id()));
        std::fs::write(&path, b"MZ just a stub, nothing appended").unwrap();
        assert!(locate(&mut File::open(&path).unwrap()).is_err());
        std::fs::remove_file(&path).unwrap();
    }
}
