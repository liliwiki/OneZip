//! 卸载：只删安装时放进去的文件，文件夹空了才删；文件关联总是还原。解压、压缩出来的文件、未完成任务的进度都不动；
//! 设置、保存的密码和出错记录只在用户勾选时删除。导入"受信任的人"的签名证书保留。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

use crate::install::{previous_files, remove_files, same_dir};
use crate::sys;

/// 要卸载的安装目录：卸载程序所在的文件夹（"应用和功能"运行的就是它），
/// 否则看卸载项里记的位置。
pub fn target() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let here = exe.parent()?.to_path_buf();
    if here.join("OneZip.exe").exists() {
        return Some(here);
    }
    let key = sys::uninstall_key()?;
    let loc: String = key.get_value("InstallLocation").ok()?;
    Some(PathBuf::from(loc))
}

pub fn uninstall(dest: &Path, delete_settings: bool, say: &dyn Fn(&str)) -> Result<()> {
    sys::init_com();
    say("正在关闭 OneZip…");
    sys::close_onezip(&[dest.to_path_buf()], Duration::ZERO);
    sys::release_menu_dll();

    say("正在移除右键菜单…");
    let _ = sys::remove_shell_package();
    let onezip = dest.join("OneZip.exe");
    if onezip.exists() {
        let _ = sys::run_hidden(&onezip, &["--unregister-menu"]);
    }
    // dllhost 可能刚又加载了 DLL
    sys::release_menu_dll();
    // 文件关联：还原成关联之前的打开方式。
    let _ = onezip_settings::assoc::remove_all();
    onezip_settings::assoc::notify();
    if let Ok(lnk) = sys::shortcut_path() {
        let _ = std::fs::remove_file(lnk);
    }

    say("正在删除文件…");
    let files = previous_files(dest);
    // 正在运行的卸载程序自己删不掉，退出后再删（见 main）。
    let me = std::env::current_exe().ok();
    let keep: Vec<String> = me
        .as_deref()
        .filter(|m| m.parent().is_some_and(|p| same_dir(p, dest)))
        .and_then(|m| m.file_name())
        .map(|n| vec![n.to_string_lossy().into_owned()])
        .unwrap_or_default();
    remove_files(dest, &files, &keep);
    let still = files
        .iter()
        .filter(|f| !keep.iter().any(|k| k.eq_ignore_ascii_case(f)))
        .filter(|f| dest.join(f).exists())
        .cloned()
        .collect::<Vec<_>>();
    let only_this_install = sys::uninstall_key()
        .and_then(|k| k.get_value::<String, _>("InstallLocation").ok())
        .is_none_or(|l| same_dir(Path::new(&l), dest));
    if only_this_install {
        sys::delete_uninstall_key();
    }
    let _ = std::fs::remove_dir(dest);

    if delete_settings {
        say("正在删除设置…");
        let _ = onezip_settings::delete_all();
        if let Some(d) = std::env::var_os("LOCALAPPDATA") {
            let _ = std::fs::remove_dir_all(PathBuf::from(d).join("OneZip"));
        }
    }
    if !still.is_empty() {
        anyhow::bail!("这些文件删不掉（可能正在被使用），可以稍后手动删除：\n{}", still.join("\n"));
    }
    Ok(())
}
