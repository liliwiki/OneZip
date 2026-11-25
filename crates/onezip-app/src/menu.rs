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
    /// 在这一项上方画分隔线。
    pub separator_before: bool,
}

const fn item(id: &'static str, label: &'static str, args: &'static str, sep: bool) -> Item {
    Item {
        id,
        label,
        args,
        separator_before: sep,
    }
}

/// 压缩包上的菜单：和 Windows 11 第一层菜单（onezip-shell）同样的分组和顺序。
/// 注册表菜单是静态的，写不出具体文件名，所以用通用说法。
pub const EXTRACT_ITEMS: &[Item] = &[
    item("01smart", "智能解压", "--extract-smart", false),
    item("02folder", "解压到同名文件夹", "--extract-folder", false),
    item("03here", "解压到当前位置", "--extract-here", false),
    item("04to", "解压文件…", "--extract-to", false),
    item("05reclaim", "解压后删除压缩包（边解压边释放空间）", "--reclaim --extract-smart", true),
    item("06test", "测试压缩包", "--test", false),
    item("07add", "添加到压缩包…", "--compress", true),
    item("08add7z", "添加到 7z 压缩包", "--compress-7z", false),
    item("09addzip", "添加到 zip 压缩包", "--compress-zip", false),
];

/// "CRC SHA" 二级菜单：（键名，菜单文字，算法）。与 onezip-shell 的 HASH_ITEMS 一致。
pub const HASH_ITEMS: &[(&str, &str, &str)] = &[
    ("1crc32", "CRC-32", "crc32"),
    ("2crc64", "CRC-64", "crc64"),
    ("3md5", "MD5", "md5"),
    ("4sha1", "SHA-1", "sha1"),
    ("5sha256", "SHA-256", "sha256"),
    ("6sha512", "SHA-512", "sha512"),
    ("7all", "*（全部）", "*"),
];

/// 其它文件和文件夹上的菜单。
pub const COMPRESS_ITEMS: &[Item] = &[
    item("07add", "添加到压缩包…", "--compress", false),
    item("08add7z", "添加到 7z 压缩包", "--compress-7z", false),
    item("09addzip", "添加到 zip 压缩包", "--compress-zip", false),
];

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const ROOT: &str = r"Software\Classes";
    const VERB: &str = "OneZip";
    /// 旧版本把 CRC SHA 放在第一层用过的键名，卸载时一并清掉。
    const VERB_HASH: &str = "OneZipHash";

    fn add(base: &str, exe: &Path, items: &[Item]) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (verb, _) = hkcu.create_subkey(format!(r"{ROOT}\{base}\shell\{VERB}"))?;
        verb.set_value("MUIVerb", &"OneZip")?;
        verb.set_value("SubCommands", &"")?;
        verb.set_value("Icon", &format!("\"{}\",0", exe.display()))?;
        for item in items {
            let (k, _) = verb.create_subkey(format!(r"shell\{}", item.id))?;
            k.set_value("MUIVerb", &item.label)?;
            if item.separator_before {
                // ECF_SEPARATORBEFORE
                k.set_value("CommandFlags", &0x20u32)?;
            }
            let (cmd, _) = k.create_subkey("command")?;
            cmd.set_value("", &format!("\"{}\" {} \"%1\"", exe.display(), item.args))?;
        }
        Ok(())
    }

    /// OneZip 子菜单最后的 "CRC SHA" 二级菜单（与 Win11 第一层菜单一致）。
    fn add_hash(base: &str, exe: &Path) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (hash, _) =
            hkcu.create_subkey(format!(r"{ROOT}\{base}\shell\{VERB}\shell\99hash"))?;
        hash.set_value("MUIVerb", &"CRC SHA")?;
        hash.set_value("SubCommands", &"")?;
        // ECF_SEPARATORBEFORE
        hash.set_value("CommandFlags", &0x20u32)?;
        for (id, label, algo) in HASH_ITEMS {
            let (k, _) = hash.create_subkey(format!(r"shell\{id}"))?;
            k.set_value("MUIVerb", label)?;
            let (cmd, _) = k.create_subkey("command")?;
            cmd.set_value("", &format!("\"{}\" --hash {algo} \"%1\"", exe.display()))?;
        }
        Ok(())
    }

    fn remove(base: &str) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(format!(r"{ROOT}\{base}\shell\{VERB}"));
        let _ = hkcu.delete_subkey_all(format!(r"{ROOT}\{base}\shell\{VERB_HASH}"));
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
            add_hash(&base, exe)?;
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
