//! OneZip 的设置：每一项、它的默认值，以及存放位置 `HKCU\Software\OneZip`（只对当前用户）。
//!
//! 主程序读写全部设置；右键菜单 DLL 只读"菜单显示哪些项"；卸载程序可以整个删掉。
//! 不依赖引擎，DLL 里用也很轻。值读不出或不认识时一律用默认值，旧版本、手改坏了都不会出错。

use std::path::PathBuf;

pub mod assoc;
pub mod passwords;

/// 注册表位置（HKCU 下）。
pub const KEY: &str = r"Software\OneZip";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// 主窗口"解压…"按钮和双击压缩包时的默认解压方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExtractMode {
    #[default]
    Smart,
    NewFolder,
    Here,
}

/// 目标位置已有同名文件时怎么办。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Conflict {
    /// 解压前先查出同名文件，弹窗问一次怎么办。
    Ask,
    Replace,
    Skip,
    /// 都保留，解出的改名为 `名字 (2).扩展名`。不会丢任何东西，所以是默认。
    #[default]
    Rename,
}

/// "解压后删除压缩包"怎么删。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeleteMode {
    #[default]
    RecycleBin,
    Permanent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    // ---- 常规 ----
    pub theme: Theme,

    // ---- 解压 ----
    pub extract_mode: ExtractMode,
    pub conflict: Conflict,
    /// 压缩包里只有一个压缩包时接着解开。
    pub nested: bool,
    pub delete_mode: DeleteMode,
    /// 边解压边删除前弹出说明（用户勾了"以后不再提醒"就关掉）。
    pub confirm_in_place: bool,
    /// 边下载边解压的默认保存位置；`None` 是"下载"文件夹。
    pub download_dir: Option<PathBuf>,
    /// 边下载边解压同时用的连接数。
    pub connections: u32,

    // ---- 压缩 ----
    /// 默认格式的扩展名，如 `7z`。
    pub format: String,
    /// 0（仅存储）到 9。
    pub level: u32,
    /// 0 表示自动（按 CPU 核数）。
    pub threads: u32,
    pub solid: bool,
    /// 设了密码时加密文件名（7z）。
    pub encrypt_names: bool,

    // ---- 右键菜单 ----
    pub menu_enabled: bool,
    /// 隐藏的菜单项（onezip-menu 的 `Command::id`）。
    pub menu_hidden: Vec<String>,

    // ---- 更新 ----
    pub auto_update: bool,
    /// 上次自动检查的时间（Unix 秒）。
    pub last_check: u64,
    /// 用户选了"跳过这个版本"的版本号。
    pub skipped: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            extract_mode: ExtractMode::Smart,
            conflict: Conflict::Rename,
            nested: true,
            delete_mode: DeleteMode::RecycleBin,
            confirm_in_place: true,
            download_dir: None,
            connections: 8,
            format: "7z".into(),
            level: 5,
            threads: 0,
            solid: true,
            encrypt_names: true,
            menu_enabled: true,
            // MD5 很少用，默认不放进菜单
            menu_hidden: vec!["md5".into()],
            auto_update: true,
            last_check: 0,
            skipped: Vec::new(),
        }
    }
}

/// 设置项与注册表里的文字之间的转换。
trait Word: Sized + Copy + PartialEq + 'static {
    const ALL: &'static [(Self, &'static str)];
    fn word(self) -> &'static str {
        Self::ALL.iter().find(|(v, _)| *v == self).map_or("", |(_, w)| w)
    }
    fn from_word(s: &str) -> Option<Self> {
        Self::ALL.iter().find(|(_, w)| *w == s).map(|(v, _)| *v)
    }
}

impl Word for Theme {
    const ALL: &'static [(Self, &'static str)] = &[(Theme::System, "system"), (Theme::Light, "light"), (Theme::Dark, "dark")];
}
impl Word for ExtractMode {
    const ALL: &'static [(Self, &'static str)] =
        &[(ExtractMode::Smart, "smart"), (ExtractMode::NewFolder, "folder"), (ExtractMode::Here, "here")];
}
impl Word for Conflict {
    const ALL: &'static [(Self, &'static str)] =
        &[(Conflict::Ask, "ask"), (Conflict::Replace, "replace"), (Conflict::Skip, "skip"), (Conflict::Rename, "rename")];
}
impl Word for DeleteMode {
    const ALL: &'static [(Self, &'static str)] =
        &[(DeleteMode::RecycleBin, "recycle"), (DeleteMode::Permanent, "permanent")];
}

/// 一组"名字 = 文字"，注册表和测试都用它，换存储方式只改这里。
fn to_pairs(s: &Settings) -> Vec<(&'static str, String)> {
    let b = |v: bool| u8::from(v).to_string();
    vec![
        ("Theme", s.theme.word().into()),
        ("ExtractMode", s.extract_mode.word().into()),
        ("Conflict", s.conflict.word().into()),
        ("Nested", b(s.nested)),
        ("DeleteMode", s.delete_mode.word().into()),
        ("ConfirmInPlace", b(s.confirm_in_place)),
        ("DownloadDir", s.download_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default()),
        ("Connections", s.connections.to_string()),
        ("Format", s.format.clone()),
        ("Level", s.level.to_string()),
        ("Threads", s.threads.to_string()),
        ("Solid", b(s.solid)),
        ("EncryptNames", b(s.encrypt_names)),
        ("MenuEnabled", b(s.menu_enabled)),
        ("MenuHidden", s.menu_hidden.join(",")),
        ("AutoUpdate", b(s.auto_update)),
        ("LastCheck", s.last_check.to_string()),
        ("Skipped", s.skipped.join(",")),
    ]
}

fn from_pairs(get: impl Fn(&str) -> Option<String>) -> Settings {
    let d = Settings::default();
    let flag = |k: &str, dv: bool| match get(k).as_deref().map(str::trim) {
        Some("1") => true,
        Some("0") => false,
        _ => dv,
    };
    let num = |k: &str, dv: u64| get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(dv);
    let list = |k: &str, dv: Vec<String>| {
        get(k).map_or(dv, |v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
    };
    fn word<T: Word>(v: Option<String>, dv: T) -> T {
        v.and_then(|v| T::from_word(v.trim())).unwrap_or(dv)
    }
    Settings {
        theme: word(get("Theme"), d.theme),
        extract_mode: word(get("ExtractMode"), d.extract_mode),
        conflict: word(get("Conflict"), d.conflict),
        nested: flag("Nested", d.nested),
        delete_mode: word(get("DeleteMode"), d.delete_mode),
        confirm_in_place: flag("ConfirmInPlace", d.confirm_in_place),
        download_dir: get("DownloadDir").filter(|s| !s.trim().is_empty()).map(PathBuf::from),
        connections: num("Connections", d.connections.into()).clamp(1, 32) as u32,
        format: get("Format").filter(|s| !s.is_empty()).unwrap_or(d.format),
        level: num("Level", d.level.into()).min(9) as u32,
        threads: num("Threads", 0).min(256) as u32,
        solid: flag("Solid", d.solid),
        encrypt_names: flag("EncryptNames", d.encrypt_names),
        menu_enabled: flag("MenuEnabled", d.menu_enabled),
        menu_hidden: list("MenuHidden", d.menu_hidden),
        auto_update: flag("AutoUpdate", d.auto_update),
        last_check: num("LastCheck", 0),
        skipped: list("Skipped", Vec::new()),
    }
}

impl Settings {
    /// 读出设置；没有保存过的项用默认值。
    pub fn load() -> Settings {
        from_pairs(store::get)
    }

    /// 全部写回。
    pub fn save(&self) -> std::io::Result<()> {
        store::set(&to_pairs(self))
    }

    /// 某个菜单项要不要显示（菜单总开关关了时都不显示）。
    pub fn menu_shows(&self, id: &str) -> bool {
        self.menu_enabled && !self.menu_hidden.iter().any(|h| h == id)
    }
}

/// 只读菜单要用的两项，给右键菜单 DLL：资源管理器每次弹菜单都会问，越少越好。
pub fn menu_filter() -> (bool, Vec<String>) {
    let s = from_pairs(|k| if matches!(k, "MenuEnabled" | "MenuHidden") { store::get(k) } else { None });
    (s.menu_enabled, s.menu_hidden)
}

/// 删除全部设置和保存的密码（卸载时用户勾选删除）。
pub fn delete_all() -> std::io::Result<()> {
    store::delete_all()
}

#[cfg(windows)]
mod store {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

    pub fn get(name: &str) -> Option<String> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(super::KEY, KEY_READ).ok()?;
        if let Ok(s) = key.get_value::<String, _>(name) {
            return Some(s);
        }
        key.get_value::<u32, _>(name).ok().map(|n| n.to_string())
    }

    pub fn set(pairs: &[(&str, String)]) -> std::io::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(super::KEY)?;
        for (name, value) in pairs {
            key.set_value(name, value)?;
        }
        Ok(())
    }

    pub fn delete_all() -> std::io::Result<()> {
        match RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(super::KEY) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

#[cfg(not(windows))]
mod store {
    pub fn get(_name: &str) -> Option<String> {
        None
    }
    pub fn set(_pairs: &[(&str, String)]) -> std::io::Result<()> {
        Ok(())
    }
    pub fn delete_all() -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn round_trip_and_bad_values() {
        let s = Settings {
            theme: Theme::Dark,
            conflict: Conflict::Skip,
            download_dir: Some(PathBuf::from(r"D:\下载")),
            menu_hidden: vec!["md5".into(), "test".into()],
            skipped: vec!["0.2.0".into()],
            level: 9,
            ..Settings::default()
        };
        let map: HashMap<&str, String> = to_pairs(&s).into_iter().collect();
        assert_eq!(from_pairs(|k| map.get(k).cloned()), s);

        // 读不出、手改坏了：用默认值，数字收在范围内
        let bad: HashMap<&str, &str> = [("Theme", "purple"), ("Level", "99"), ("Connections", "0"), ("Solid", "")].into();
        let r = from_pairs(|k| bad.get(k).map(|v| v.to_string()));
        assert_eq!(r.theme, Theme::System);
        assert_eq!(r.level, 9);
        assert_eq!(r.connections, 1);
        assert!(r.solid);
        assert_eq!(from_pairs(|_| None), Settings::default());
    }

    #[test]
    fn menu_filter() {
        let mut s = Settings::default();
        assert!(s.menu_shows("smart") && !s.menu_shows("md5"));
        s.menu_enabled = false;
        assert!(!s.menu_shows("smart"));
    }
}
