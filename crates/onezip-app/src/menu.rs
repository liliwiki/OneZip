//! 传统右键菜单（按用户写注册表，不需要管理员权限）。
//!
//! Windows 10 上它直接出现在右键菜单里；Windows 11 上会被收进"显示更多选项"，
//! 第一层菜单由 `onezip-shell`（IExplorerCommand + 稀疏包）负责，见 packaging/。
//!
//! 只写 OneZip 自己命名的键，[`unregister`] 只删这些键。

use std::path::Path;

use anyhow::Result;

/// 出现在压缩包上的扩展名。`.001` 是分卷的第一卷。
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "tar", "tgz", "gz", "tbz2", "tbz", "bz2", "txz", "xz", "zst", "tzst",
    "lz4", "001",
];

pub struct Item {
    pub id: &'static str,
    pub label: &'static str,
    pub args: &'static str,
}

/// 压缩包上的菜单。
pub const EXTRACT_ITEMS: &[Item] = &[
    Item {
        id: "1smart",
        label: "智能解压",
        args: "--extract-smart",
    },
    Item {
        id: "2reclaim",
        label: "智能解压并释放空间（边解边删原包）",
        args: "--reclaim --extract-smart",
    },
    Item {
        id: "3here",
        label: "解压到当前文件夹",
        args: "--extract-here",
    },
    Item {
        id: "4folder",
        label: "解压到同名文件夹",
        args: "--extract-folder",
    },
    Item {
        id: "5to",
        label: "解压到…",
        args: "--extract-to",
    },
    Item {
        id: "6test",
        label: "测试压缩包",
        args: "--test",
    },
];

/// 任意文件和文件夹上的菜单。
pub const COMPRESS_ITEMS: &[Item] = &[
    Item {
        id: "7add7z",
        label: "添加到 7z 压缩包",
        args: "--compress-7z",
    },
    Item {
        id: "8addzip",
        label: "添加到 zip 压缩包",
        args: "--compress-zip",
    },
    Item {
        id: "9add",
        label: "添加到压缩包…",
        args: "--compress",
    },
];

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const ROOT: &str = r"Software\Classes";
    const VERB: &str = "OneZip";

    fn add(base: &str, exe: &Path, items: &[Item]) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (verb, _) = hkcu.create_subkey(format!(r"{ROOT}\{base}\shell\{VERB}"))?;
        verb.set_value("MUIVerb", &"OneZip")?;
        verb.set_value("SubCommands", &"")?;
        verb.set_value("Icon", &format!("\"{}\",0", exe.display()))?;
        for item in items {
            let (k, _) = verb.create_subkey(format!(r"shell\{}", item.id))?;
            k.set_value("MUIVerb", &item.label)?;
            let (cmd, _) = k.create_subkey("command")?;
            cmd.set_value("", &format!("\"{}\" {} \"%1\"", exe.display(), item.args))?;
        }
        Ok(())
    }

    fn remove(base: &str) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(format!(r"{ROOT}\{base}\shell\{VERB}"));
    }

    fn bases() -> Vec<(String, bool)> {
        let mut v: Vec<(String, bool)> = ARCHIVE_EXTENSIONS
            .iter()
            .map(|e| (format!(r"SystemFileAssociations\.{e}"), true))
            .collect();
        v.push(("*".into(), false));
        v.push(("Directory".into(), false));
        v
    }

    pub fn register(exe: &Path) -> Result<()> {
        unregister();
        for (base, archive) in bases() {
            if archive {
                add(&base, exe, EXTRACT_ITEMS)?;
            } else {
                add(&base, exe, COMPRESS_ITEMS)?;
            }
        }
        notify();
        Ok(())
    }

    pub fn unregister() {
        for (base, _) in bases() {
            remove(&base);
        }
        notify();
    }

    pub fn is_registered() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(format!(r"{ROOT}\*\shell\{VERB}"))
            .is_ok()
    }

    /// 安装程序是否已注册 Windows 11 第一层菜单用的稀疏包（OneZip.ShellExtension）。
    pub fn shell_package_installed() -> bool {
        const PACKAGES: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(PACKAGES)
            .map(|k| {
                k.enum_keys()
                    .flatten()
                    .any(|name| name.starts_with("OneZip.ShellExtension_"))
            })
            .unwrap_or(false)
    }

    /// 让资源管理器重新读取文件关联。
    fn notify() {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SHChangeNotify(event: i32, flags: u32, a: *const u8, b: *const u8);
        }
        const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null()) };
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn register(_exe: &Path) -> Result<()> {
        anyhow::bail!("右键菜单只支持 Windows")
    }
    pub fn unregister() {}
    pub fn is_registered() -> bool {
        false
    }
    pub fn shell_package_installed() -> bool {
        false
    }
}

pub use imp::{is_registered, register, shell_package_installed, unregister};
