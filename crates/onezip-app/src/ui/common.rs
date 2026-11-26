//! 提示框、文件对话框、文件名等各窗口共用的小工具。

use std::path::{Path, PathBuf};

use onezip_core::Format;
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageLevel};

use crate::tasks::file_name;

/// 归属 `parent` 的提示框：始终在它上面，不会跑到别的窗口后面让人以为程序卡死。
/// 父窗口隐藏时（例如从右键菜单来、主窗口没开）就不指定父窗口，单独显示。
pub(super) fn message(parent: &slint::Window) -> MessageDialog {
    let d = MessageDialog::new().set_title("OneZip");
    if parent.is_visible() {
        d.set_parent(&parent.window_handle())
    } else {
        d
    }
}

pub(super) fn file_dialog(parent: &slint::Window) -> FileDialog {
    let d = FileDialog::new();
    if parent.is_visible() {
        d.set_parent(&parent.window_handle())
    } else {
        d
    }
}

pub(super) fn error(parent: &slint::Window, msg: &str) {
    message(parent)
        .set_level(MessageLevel::Error)
        .set_description(msg)
        .set_buttons(MessageButtons::Ok)
        .show();
}

/// 去掉压缩扩展名后的名字，用作文件夹名。
pub(super) fn stem(path: &Path) -> String {
    onezip_formats::archive_stem(&file_name(path))
}

/// 已存在时在名字后面加 (2)、(3)…
pub(super) fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let name = file_name(&path);
    let (base, ext) = Format::ALL
        .iter()
        .map(|f| format!(".{}", f.extension()))
        .find(|e| name.ends_with(e.as_str()))
        .map(|e| (name[..name.len() - e.len()].to_string(), e))
        .unwrap_or((name.clone(), String::new()));
    for i in 2.. {
        let p = path.with_file_name(format!("{base} ({i}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

pub(super) fn reveal(path: &Path) {
    let mut cmd = std::process::Command::new("explorer.exe");
    if path.is_file() {
        cmd.arg(format!("/select,{}", path.display()));
    } else {
        cmd.arg(path);
    }
    let _ = cmd.spawn();
}

/// 用默认程序打开链接或文件夹。
pub(super) fn open_url(target: &str) {
    crate::win::shell_open(target);
}
