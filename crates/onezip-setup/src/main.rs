//! OneZip 安装程序。
//!
//! 发布的 `OneZip-<版本>-Setup.exe` 是这个程序后面接上一个 zip（发布目录里的全部文件），
//! 最后 16 字节是尾标：zip 的长度（u64 小端）+ `MAGIC`。安装时把 zip 解到临时目录，
//! 再运行其中的 `install.ps1`（复制到 `%LOCALAPPDATA%\Programs\OneZip`、注册右键菜单、
//! 开始菜单快捷方式、"应用和功能"里的卸载项）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

slint::include_modules!();

const MAGIC: &[u8; 8] = b"OZSETUP1";

fn main() -> Result<()> {
    let w = SetupWindow::new()?;
    w.set_version(env!("CARGO_PKG_VERSION").into());
    let dest = install_dir();
    w.set_location(dest.display().to_string().into());
    w.set_upgrade(dest.join("OneZip.exe").exists());
    w.set_win11(is_win11());

    w.on_install({
        let weak = w.as_weak();
        move || {
            let Some(w) = weak.upgrade() else { return };
            w.set_stage("installing".into());
            let weak = weak.clone();
            std::thread::spawn(move || {
                let r = install();
                let _ = weak.upgrade_in_event_loop(move |w| match r {
                    Ok(()) => w.set_stage("done".into()),
                    Err(e) => {
                        w.set_error(format!("{e:#}").into());
                        w.set_stage("failed".into());
                    }
                });
            });
        }
    });
    w.on_finish({
        let weak = w.as_weak();
        move || {
            let Some(w) = weak.upgrade() else { return };
            if w.get_launch() {
                let exe = install_dir().join("OneZip.exe");
                let _ = Command::new(&exe).current_dir(install_dir()).spawn();
            }
            let _ = w.hide();
        }
    });
    w.on_quit({
        let weak = w.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                let _ = w.hide();
            }
        }
    });
    // 安装进行中不允许关窗口，免得只装了一半。
    w.window().on_close_requested({
        let weak = w.as_weak();
        move || match weak.upgrade() {
            Some(w) if w.get_stage() == "installing" => slint::CloseRequestResponse::KeepWindowShown,
            _ => slint::CloseRequestResponse::HideWindow,
        }
    });
    w.run()?;
    Ok(())
}

fn install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join("Programs").join("OneZip")
}

fn is_win11() -> bool {
    // 内部版本 22000 起是 Windows 11。RtlGetVersion 不受兼容性清单影响，总是返回真实版本。
    #[cfg(windows)]
    {
        #[repr(C)]
        struct OsVersionInfoW {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform: u32,
            csd: [u16; 128],
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
        }
        let mut info = OsVersionInfoW {
            size: size_of::<OsVersionInfoW>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            csd: [0; 128],
        };
        // SAFETY: info 是按 RTL_OSVERSIONINFOW 布局初始化好的结构，size 字段已填。
        unsafe { RtlGetVersion(&mut info) == 0 && info.build >= 22000 }
    }
    #[cfg(not(windows))]
    false
}

/// 本程序后面接的 zip 在文件里的起点和长度。
fn payload(exe: &mut File) -> Result<(u64, u64)> {
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
    let len = u64::from_le_bytes(tail[..8].try_into().unwrap());
    if len > total - 16 {
        bail!("安装程序已损坏，请重新下载");
    }
    Ok((total - 16 - len, len))
}

/// 文件中的一段，当作独立的文件来读。
struct Slice {
    file: File,
    start: u64,
    len: u64,
    pos: u64,
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

fn unpack(to: &Path) -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut file = File::open(&exe).with_context(|| format!("读取 {}", exe.display()))?;
    let (start, len) = payload(&mut file)?;
    let slice = Slice { file, start, len, pos: 0 };
    let mut zip = zip::ZipArchive::new(slice).context("安装程序已损坏，请重新下载")?;
    zip.extract(to).context("解出安装文件失败")?;
    Ok(())
}

fn install() -> Result<()> {
    let tmp = std::env::temp_dir().join(format!("OneZip-Setup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let r = unpack(&tmp).and_then(|()| run_script(&tmp.join("install.ps1")));
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn run_script(script: &Path) -> Result<()> {
    if !script.exists() {
        bail!("安装文件里缺少 install.ps1");
    }
    // 输出用 UTF-8，出错时能把 PowerShell 的中文提示原样显示出来。
    let cmd = format!(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $ErrorActionPreference = 'Stop'; & '{}'",
        script.display().to_string().replace('\'', "''")
    );
    let mut c = Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &cmd]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let out = c.output().context("无法运行 PowerShell")?;
    if out.status.success() {
        return Ok(());
    }
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    let err = text(&out.stderr);
    let msg = if err.is_empty() { text(&out.stdout) } else { err };
    bail!("{}", last_lines(&msg, 12));
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_the_appended_zip() {
        let dir = std::env::temp_dir().join(format!("onezip-setup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut buf = io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("install.ps1", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"Write-Host hi").unwrap();
            z.finish().unwrap();
        }
        let zip_bytes = buf.into_inner();
        let path = dir.join("stub.exe");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"MZ not really an exe").unwrap();
        f.write_all(&zip_bytes).unwrap();
        f.write_all(&(zip_bytes.len() as u64).to_le_bytes()).unwrap();
        f.write_all(MAGIC).unwrap();
        drop(f);

        let mut f = File::open(&path).unwrap();
        let (start, len) = payload(&mut f).unwrap();
        assert_eq!((start, len), (20, zip_bytes.len() as u64));
        let mut zip = zip::ZipArchive::new(Slice { file: f, start, len, pos: 0 }).unwrap();
        let out = dir.join("out");
        zip.extract(&out).unwrap();
        assert_eq!(std::fs::read(out.join("install.ps1")).unwrap(), b"Write-Host hi");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_an_exe_without_payload() {
        let path = std::env::temp_dir().join(format!("onezip-setup-empty-{}", std::process::id()));
        std::fs::write(&path, b"MZ just a stub, nothing appended").unwrap();
        assert!(payload(&mut File::open(&path).unwrap()).is_err());
        std::fs::remove_file(&path).unwrap();
    }
}
