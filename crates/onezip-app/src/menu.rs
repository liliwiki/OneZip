//! 传统右键菜单（按用户写注册表，不需要管理员权限）。
//!
//! Windows 10 上它直接出现在右键菜单里；Windows 11 上会被收进"显示更多选项"，
//! 第一层菜单由 `onezip-shell`（IExplorerCommand + 稀疏包）负责，见 packaging/。
//!
//! 只写 OneZip 自己命名的键，[`unregister`] 只删这些键。

use std::path::Path;

use anyhow::Result;

use onezip_menu::{Command, Entry, MENU, Show};
use onezip_settings::Settings;

/// 注册表菜单里的一项（静态文字：注册表写不出具体文件名）。
struct Item {
    /// 键名：序号在前，资源管理器按键名排序。
    key: String,
    label: &'static str,
    args: String,
    /// 在这一项上方画分隔线。
    separator_before: bool,
}

/// 某类文件上的菜单项，按 onezip-menu 的顺序和分组。校验值另外放进 "CRC SHA" 二级菜单。
/// `archive`：右键的是压缩包；`in_place`：它能边解边删。
fn items(archive: bool, in_place: bool, hash: bool, settings: &Settings) -> Vec<Item> {
    let mut out = Vec::new();
    let mut separator = false;
    for (n, entry) in MENU.iter().enumerate() {
        let shown = match entry.show() {
            Show::Always => true,
            Show::Archives => archive,
            Show::InPlace => archive && in_place,
        } && match entry {
            Entry::Command(c) => settings.menu_shows(c.id()),
            Entry::Separator(_) => true,
        };
        match *entry {
            Entry::Separator(_) => separator |= shown && !out.is_empty(),
            Entry::Command(c) if shown && matches!(c, Command::Hash(_)) == hash => {
                out.push(Item {
                    key: format!("{n:02}{}", c.id()),
                    label: c.label(),
                    args: c.args().join(" "),
                    separator_before: std::mem::take(&mut separator),
                });
            }
            Entry::Command(_) => {}
        }
    }
    out
}

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
        if items.is_empty() {
            return Ok(());
        }
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (verb, _) = hkcu.create_subkey(format!(r"{ROOT}\{base}\shell\{VERB}"))?;
        verb.set_value("MUIVerb", &"OneZip")?;
        verb.set_value("SubCommands", &"")?;
        verb.set_value("Icon", &format!("\"{}\",0", exe.display()))?;
        for item in items {
            let (k, _) = verb.create_subkey(format!(r"shell\{}", item.key))?;
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

    /// OneZip 子菜单最后的 "CRC SHA" 二级菜单（Win11 第一层菜单里是平铺的）。
    fn add_hash(base: &str, exe: &Path, settings: &Settings) -> Result<()> {
        let hash_items = items(false, false, true, settings);
        if hash_items.is_empty() {
            return Ok(());
        }
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (hash, _) =
            hkcu.create_subkey(format!(r"{ROOT}\{base}\shell\{VERB}\shell\99hash"))?;
        hash.set_value("MUIVerb", &"CRC SHA")?;
        hash.set_value("SubCommands", &"")?;
        // ECF_SEPARATORBEFORE
        hash.set_value("CommandFlags", &0x20u32)?;
        for item in hash_items {
            let (k, _) = hash.create_subkey(format!(r"shell\{}", item.key))?;
            k.set_value("MUIVerb", &item.label)?;
            let (cmd, _) = k.create_subkey("command")?;
            cmd.set_value("", &format!("\"{}\" {} \"%1\"", exe.display(), item.args))?;
        }
        Ok(())
    }

    fn remove(base: &str) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(format!(r"{ROOT}\{base}\shell\{VERB}"));
        let _ = hkcu.delete_subkey_all(format!(r"{ROOT}\{base}\shell\{VERB_HASH}"));
    }

    /// （注册表位置，是否压缩包，能否边解边删）
    fn bases() -> Vec<(String, bool, bool)> {
        let mut v: Vec<(String, bool, bool)> = onezip_formats::archive_extensions()
            .map(|e| (format!(r"SystemFileAssociations\.{e}"), true, onezip_formats::in_place(&format!("x.{e}"))))
            .collect();
        v.push(("*".into(), false, false));
        v.push(("Directory".into(), false, false));
        v
    }

    /// 按设置写传统菜单（隐藏的项不写）。
    pub fn register(exe: &Path, settings: &Settings) -> Result<()> {
        unregister();
        for (base, archive, in_place) in bases() {
            add(&base, exe, &items(archive, in_place, false, settings))?;
            add_hash(&base, exe, settings)?;
        }
        notify();
        Ok(())
    }

    pub fn unregister() {
        for (base, _, _) in bases() {
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
    use onezip_i18n::t;
    use super::*;
    pub fn register(_exe: &Path, _settings: &Settings) -> Result<()> {
        anyhow::bail!(t!("右键菜单只支持 Windows"))
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

/// 让右键菜单符合设置。Windows 11 的第一层菜单由 DLL 每次弹出时自己读设置，这里只管传统菜单：
/// 装了第一层菜单时不写传统菜单（免得"显示更多选项"里重复一份）。
pub fn apply(settings: &Settings) -> Result<()> {
    if shell_package_installed() || !settings.menu_enabled {
        unregister();
        Ok(())
    } else {
        register(&std::env::current_exe()?, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_menu_follows_the_model() {
        // 都显示时
        let d = Settings { menu_hidden: Vec::new(), ..Settings::default() };
        let zip: Vec<String> = items(true, true, false, &d).iter().map(|i| i.key.clone()).collect();
        assert_eq!(
            zip,
            ["00smart", "01folder", "02here", "03to", "05deleteafter", "06inplace", "07test", "09add", "10add7z", "11addzip"]
        );
        let sep: Vec<bool> = items(true, true, false, &d).iter().map(|i| i.separator_before).collect();
        assert_eq!(sep, [false, false, false, false, true, false, false, true, false, false]);
        let dmg: Vec<String> = items(true, false, false, &d).iter().map(|i| i.key.clone()).collect();
        assert!(!dmg.iter().any(|k| k.ends_with("inplace")));
        assert!(dmg.iter().any(|k| k.ends_with("deleteafter")));
        let file: Vec<String> = items(false, false, false, &d).iter().map(|i| i.key.clone()).collect();
        assert_eq!(file, ["09add", "10add7z", "11addzip"]);
        assert!(!items(false, false, false, &d)[0].separator_before);
        let hash: Vec<String> = items(false, false, true, &d).iter().map(|i| i.args.clone()).collect();
        assert_eq!(hash, ["--hash crc32", "--hash md5", "--hash sha256", "--hash *"]);

        // 隐藏一些：分隔线不会落在开头或连着
        let d = Settings { menu_hidden: vec!["inplace".into(), "test".into(), "md5".into()], ..Settings::default() };
        let keys: Vec<String> = items(true, true, false, &d).iter().map(|i| i.key.clone()).collect();
        assert_eq!(keys, ["00smart", "01folder", "02here", "03to", "05deleteafter", "09add", "10add7z", "11addzip"]);
        let sep: Vec<bool> = items(true, true, false, &d).iter().map(|i| i.separator_before).collect();
        assert_eq!(sep, [false, false, false, false, true, true, false, false]);
        assert_eq!(items(false, false, true, &d).len(), 3);
    }
}
