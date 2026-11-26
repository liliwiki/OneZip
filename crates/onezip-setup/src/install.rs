//! 安装：把文件放进安装目录、注册右键菜单、开始菜单快捷方式、"应用和功能"里的卸载项。
//!
//! 安装目录里只有程序本身：OneZip.exe、onezipc.exe、onezip_shell.dll、7z.dll、uninstall.exe
//! 和 licenses 文件夹。装了哪些文件记在卸载项的 `InstalledFiles` 里；升级时上一版有、
//! 这一版没有的文件会删掉，卸载也只删这些文件。

use onezip_i18n::t;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::{payload, sys};

/// 早期版本（PowerShell 安装脚本）放进安装目录、现在不再需要的文件。
const LEGACY: &[&str] = &[
    "installed-files.txt",
    "install.ps1",
    "uninstall.ps1",
    "OneZip-Shell.msix",
    "OneZip-Dev.cer",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "README.md",
    "README.zh-CN.md",
    "7-Zip-License.txt",
    "UnRAR-License.txt",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "NOTICE",
];

/// 换不掉的文件（被别的程序占着）先改成这个后缀，下次安装时删掉。
const OLD_SUFFIX: &str = ".old-onezip";

pub struct Plan {
    pub dest: PathBuf,
    /// 已经装在哪里（升级时）。
    pub old: Option<PathBuf>,
    /// 由 OneZip 的"立即更新"启动：等 OneZip 自己退出。
    pub update: bool,
}

pub fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

/// 只接受安装目录里的相对路径，免得清单被改过时删到别处。
fn safe(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains(':')
        && !rel.starts_with(['\\', '/'])
        && rel.split(['\\', '/']).all(|c| !c.is_empty() && c != "." && c != "..")
}

/// `dir` 里上一次安装放进去的文件：卸载项里的清单（装在这里时）、早期版本的
/// installed-files.txt、以及早期版本的固定文件名。
pub fn previous_files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(key) = sys::uninstall_key() {
        let here = key.get_value::<String, _>("InstallLocation").is_ok_and(|l| same_dir(Path::new(&l), dir));
        if here && let Ok(list) = key.get_value::<Vec<String>, _>("InstalledFiles") {
            out.extend(list);
        }
    }
    if let Ok(text) = std::fs::read_to_string(dir.join("installed-files.txt")) {
        out.extend(text.trim_start_matches('\u{feff}').lines().map(|l| l.trim().to_string()));
    }
    out.extend(LEGACY.iter().map(|s| s.to_string()));
    out.extend(["OneZip.exe", "onezipc.exe", "onezip_shell.dll", "7z.dll", "uninstall.exe"].map(String::from));
    out.retain(|f| safe(f));
    out.sort_by_key(|f| f.to_lowercase());
    out.dedup_by_key(|f| f.to_lowercase());
    out
}

/// 删掉 `dir` 里的这些文件（`keep` 里的除外）和因此空了的子文件夹，以及换文件时留下的旧文件。
pub fn remove_files(dir: &Path, files: &[String], keep: &[String]) {
    let kept = |f: &str| keep.iter().any(|k| k.eq_ignore_ascii_case(f));
    let mut parents = Vec::new();
    for f in files.iter().filter(|f| !kept(f)) {
        let p = dir.join(f);
        if p.is_file() {
            let _ = std::fs::remove_file(&p);
        }
        let mut parent = p.parent();
        while let Some(d) = parent.filter(|d| *d != dir) {
            parents.push(d.to_path_buf());
            parent = d.parent();
        }
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().ends_with(OLD_SUFFIX) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    // 先删深的
    parents.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for d in parents {
        let _ = std::fs::remove_dir(d);
    }
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d)?;
    }
    if std::fs::copy(from, to).is_ok() {
        return Ok(());
    }
    // 被占着的文件（多半是资源管理器还加载着菜单 DLL）不能覆盖，但可以改名。
    let aside = to.with_file_name(format!("{}{OLD_SUFFIX}", to.file_name().unwrap_or_default().to_string_lossy()));
    let _ = std::fs::remove_file(&aside);
    std::fs::rename(to, &aside).with_context(|| t!("{} 正在被使用，无法替换", to.display()))?;
    std::fs::copy(from, to).with_context(|| t!("复制 {}", to.display()))?;
    Ok(())
}

fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".onezip-write-test");
    let ok = std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

/// 装到需要管理员权限的地方（如 C:\Program Files）：请管理员把这个文件夹交给当前用户，
/// 其余步骤仍以当前用户身份进行。
fn ensure_writable(dir: &Path) -> Result<()> {
    if writable(dir) {
        return Ok(());
    }
    let sid = sys::user_sid()?;
    sys::run_elevated(&["--grant".into(), dir.display().to_string(), sid])
        .with_context(|| t!("无法安装到 {}，请换一个位置", dir.display()))?;
    if !writable(dir) {
        bail!(t!("无法写入 {}，请换一个位置", dir.display()));
    }
    Ok(())
}

pub fn install(plan: &Plan, say: &dyn Fn(&str)) -> Result<()> {
    sys::init_com();
    let tmp = std::env::temp_dir().join(format!("OneZip-Setup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let r = steps(plan, &tmp, say);
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn steps(plan: &Plan, tmp: &Path, say: &dyn Fn(&str)) -> Result<()> {
    let dest = &plan.dest;
    let exe = std::env::current_exe()?;
    say(t!("正在解出安装文件…"));
    payload::unpack(&exe, tmp)?;
    let mut files = payload::program_files(tmp)?;
    files.push("uninstall.exe".into());
    files.sort();

    ensure_writable(dest)?;

    say(t!("正在关闭 OneZip…"));
    let mut dirs = vec![dest.clone()];
    dirs.extend(plan.old.clone());
    sys::close_onezip(&dirs, Duration::from_secs(if plan.update { 30 } else { 0 }));
    sys::release_menu_dll();
    // 旧的稀疏包指向旧位置；失败也不要紧，下面注册新包时会强制替换。
    let _ = sys::remove_shell_package();

    // 换了位置：旧位置里装过的文件删掉。
    let before = previous_files(dest);
    if let Some(old) = plan.old.as_deref().filter(|o| !same_dir(o, dest)) {
        remove_files(old, &previous_files(old), &[]);
        let _ = std::fs::remove_dir(old);
    }

    say(t!("正在复制文件…"));
    for f in files.iter().filter(|f| f.as_str() != "uninstall.exe") {
        copy_file(&tmp.join(f), &dest.join(f))?;
    }
    let uninstall = dest.join("uninstall.exe");
    if payload::write_stub(&exe, &uninstall).is_err() {
        let staged = tmp.join("uninstall.exe");
        payload::write_stub(&exe, &staged)?;
        copy_file(&staged, &uninstall)?;
    }
    // 上一版有、这一版没有的文件
    remove_files(dest, &before, &files);

    let onezip = dest.join("OneZip.exe");
    if sys::is_win11()
        && let Some(msix) = payload::setup_file(tmp, "OneZip-Shell.msix")
    {
        say(t!("正在注册右键菜单…"));
        // 自签名的包要先让本机信任签名证书（一次管理员确认，以后升级不再需要）。
        if let Some(cer) = payload::setup_file(tmp, "OneZip-Dev.cer") {
            let der = std::fs::read(&cer)?;
            if !sys::trusted(&der)? {
                sys::run_elevated(&["--trust".into(), cer.display().to_string()])
                    .context(t!("右键菜单需要信任 OneZip 的签名证书"))?;
            }
        }
        sys::add_shell_package(&msix, dest)?;
    }
    // 传统右键菜单按设置写（装了第一层菜单时会删掉传统菜单，免得重复）。
    sys::run_hidden(&onezip, &["--register-menu", "/quiet"])?;

    say(t!("正在创建开始菜单快捷方式…"));
    sys::create_shortcut(&onezip)?;
    let size: u64 = files.iter().filter_map(|f| std::fs::metadata(dest.join(f)).ok()).map(|m| m.len()).sum();
    sys::write_uninstall_key(dest, env!("CARGO_PKG_VERSION"), &files, (size / 1024) as u32)?;
    // 让资源管理器重新加载新的菜单 DLL。
    sys::release_menu_dll();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_relative_names_inside_the_folder() {
        assert!(safe("OneZip.exe"));
        assert!(safe("licenses\\LICENSE.txt"));
        assert!(!safe(""));
        assert!(!safe("..\\x"));
        assert!(!safe("a\\..\\..\\x"));
        assert!(!safe("C:\\Windows\\x"));
        assert!(!safe("\\x"));
    }

    #[test]
    fn removes_listed_files_and_empty_folders() {
        let dir = std::env::temp_dir().join(format!("onezip-setup-rm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("licenses")).unwrap();
        for f in ["OneZip.exe", "install.ps1", "licenses\\a.txt", "mine.txt", "x.dll.old-onezip"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let listed: Vec<String> = ["OneZip.exe", "install.ps1", "licenses\\a.txt"].map(String::from).to_vec();
        remove_files(&dir, &listed, &["OneZip.exe".into()]);
        assert!(dir.join("OneZip.exe").exists());
        assert!(!dir.join("install.ps1").exists());
        assert!(!dir.join("licenses").exists());
        assert!(!dir.join("x.dll.old-onezip").exists());
        assert!(dir.join("mine.txt").exists(), "不在清单里的文件不动");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
