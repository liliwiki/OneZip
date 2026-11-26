//! 文件关联：双击压缩包用 OneZip 打开（浏览窗口）。只写当前用户的注册表，不需要管理员权限。
//!
//! 每种扩展名一个 ProgID（`OneZip.zip`），`HKCU\Software\Classes\.zip` 的默认值指向它，
//! 原来的默认值记在 `OneZip.Previous` 里，取消时还原。另外在"默认应用"里登记 OneZip
//! （`RegisteredApplications` + `Capabilities`），用户可以在 Windows 设置里选它。
//!
//! 默认程序最终由用户在 Windows 里确认：用户以前选过别的程序（"始终使用此应用打开"），
//! 或者 Windows 发现这种文件有了新的打开方式，双击时会先弹窗问。程序不能也不该绕过这一步：
//! [`overridden`] 查出双击时还不会直接用 OneZip 的扩展名，界面引导用户去确认。


/// 压缩包。
pub const ARCHIVES: &[&str] = &[
    "7z", "zip", "rar", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "tzst", "lz4", "001", "cab", "arj",
    "lzh", "lha", "z", "taz", "lzma", "cpio", "rpm", "deb", "xar",
];
/// 磁盘镜像等：Windows 自己能打开或挂载，关联后双击就改成用 OneZip 看里面的文件。
pub const IMAGES: &[&str] = &["iso", "img", "vhd", "vhdx", "vmdk", "dmg", "wim", "esd", "swm"];
/// "选择常用的"勾上的那些。
pub const COMMON: &[&str] = &["7z", "zip", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst"];

/// 在"默认应用"里登记的名字（也用在 `ms-settings:defaultapps?registeredAppUser=`）。
pub const APP: &str = "OneZip";

fn progid(ext: &str) -> String {
    format!("OneZip.{ext}")
}

#[cfg(windows)]
mod imp {
    use onezip_i18n::t;
    use std::io;
    use std::path::Path;

    use windows::Win32::UI::Shell::{
        ASSOCF_NONE, ASSOCSTR_PROGID, AssocQueryStringW, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify,
    };
    use windows::core::{HSTRING, PWSTR};
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    use super::{APP, ARCHIVES, IMAGES, progid};

    const CLASSES: &str = r"Software\Classes";
    const CAPABILITIES: &str = r"Software\OneZip\Capabilities";
    const REGISTERED: &str = r"Software\RegisteredApplications";
    const PREVIOUS: &str = "OneZip.Previous";

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    fn not_found_ok(r: io::Result<()>) -> io::Result<()> {
        match r {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    /// `.ext` 的默认值指向 OneZip。
    pub fn associated(ext: &str) -> bool {
        hkcu()
            .open_subkey_with_flags(format!(r"{CLASSES}\.{ext}"), KEY_READ)
            .and_then(|k| k.get_value::<String, _>(""))
            .is_ok_and(|v| v.eq_ignore_ascii_case(&progid(ext)))
    }

    /// 关联了 OneZip，但双击时 Windows 还不会直接用它（用户选过别的程序，或 Windows 要先问）。
    pub fn overridden(ext: &str) -> bool {
        if !associated(ext) {
            return false;
        }
        let name = HSTRING::from(format!(".{ext}"));
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = unsafe {
            AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_PROGID, &name, None, Some(PWSTR(buf.as_mut_ptr())), &mut len)
        }
        .is_ok();
        if !ok {
            return false;
        }
        let actual = String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]);
        !actual.eq_ignore_ascii_case(&progid(ext))
    }

    /// 关联或取消关联一种扩展名。
    pub fn set(exe: &Path, ext: &str, on: bool) -> io::Result<()> {
        if on { add(exe, ext) } else { remove(ext) }
    }

    fn add(exe: &Path, ext: &str) -> io::Result<()> {
        let id = progid(ext);
        let exe = exe.display();
        let classes = hkcu().create_subkey(CLASSES)?.0;
        let (prog, _) = classes.create_subkey(&id)?;
        let name = t!("{} 压缩包", ext.to_uppercase());
        prog.set_value("", &name)?;
        prog.set_value("FriendlyTypeName", &name)?;
        prog.create_subkey("DefaultIcon")?.0.set_value("", &format!("\"{exe}\",0"))?;
        let (open, _) = prog.create_subkey(r"shell\open")?;
        open.set_value("", &t!("用 OneZip 打开"))?;
        open.create_subkey("command")?.0.set_value("", &format!("\"{exe}\" --open \"%1\""))?;

        let (dot, _) = classes.create_subkey(format!(".{ext}"))?;
        let old: String = dot.get_value("").unwrap_or_default();
        if !old.eq_ignore_ascii_case(&id) {
            if dot.get_raw_value(PREVIOUS).is_err() {
                dot.set_value(PREVIOUS, &old)?;
            }
            dot.set_value("", &id)?;
        }
        dot.create_subkey("OpenWithProgids")?.0.set_value(&id, &"")?;

        let (cap, _) = hkcu().create_subkey(CAPABILITIES)?;
        cap.set_value("ApplicationName", &APP)?;
        cap.set_value("ApplicationDescription", &t!("压缩、解压、浏览压缩包"))?;
        cap.create_subkey("FileAssociations")?.0.set_value(format!(".{ext}"), &id)?;
        hkcu().create_subkey(REGISTERED)?.0.set_value(APP, &CAPABILITIES)?;
        Ok(())
    }

    fn remove(ext: &str) -> io::Result<()> {
        let id = progid(ext);
        if let Ok(dot) = hkcu().open_subkey_with_flags(format!(r"{CLASSES}\.{ext}"), KEY_READ | KEY_WRITE) {
            let now: String = dot.get_value("").unwrap_or_default();
            if now.eq_ignore_ascii_case(&id) {
                match dot.get_value::<String, _>(PREVIOUS) {
                    Ok(old) if !old.is_empty() => dot.set_value("", &old)?,
                    _ => not_found_ok(dot.delete_value(""))?,
                }
            }
            not_found_ok(dot.delete_value(PREVIOUS))?;
            if let Ok(with) = dot.open_subkey_with_flags("OpenWithProgids", KEY_WRITE) {
                not_found_ok(with.delete_value(&id))?;
            }
        }
        not_found_ok(hkcu().delete_subkey_all(format!(r"{CLASSES}\{id}")))?;

        if let Ok(fa) = hkcu().open_subkey_with_flags(format!(r"{CAPABILITIES}\FileAssociations"), KEY_READ | KEY_WRITE) {
            not_found_ok(fa.delete_value(format!(".{ext}")))?;
            if fa.enum_values().next().is_none() {
                drop(fa);
                not_found_ok(hkcu().delete_subkey_all(CAPABILITIES))?;
                if let Ok(r) = hkcu().open_subkey_with_flags(REGISTERED, KEY_WRITE) {
                    not_found_ok(r.delete_value(APP))?;
                }
            }
        }
        Ok(())
    }

    /// 取消全部关联（卸载时）。
    pub fn remove_all() -> io::Result<()> {
        for ext in ARCHIVES.iter().chain(IMAGES) {
            remove(ext)?;
        }
        not_found_ok(hkcu().delete_subkey_all(CAPABILITIES))?;
        if let Ok(r) = hkcu().open_subkey_with_flags(REGISTERED, KEY_WRITE) {
            not_found_ok(r.delete_value(APP))?;
        }
        Ok(())
    }

    /// 告诉资源管理器关联变了（刷新图标和双击行为）。
    pub fn notify() {
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::Path;

    pub fn associated(_ext: &str) -> bool {
        false
    }
    pub fn overridden(_ext: &str) -> bool {
        false
    }
    pub fn set(_exe: &Path, _ext: &str, _on: bool) -> io::Result<()> {
        Ok(())
    }
    pub fn remove_all() -> io::Result<()> {
        Ok(())
    }
    pub fn notify() {}
}

pub use imp::{associated, notify, overridden, remove_all, set};

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    /// 用一个不存在的扩展名试：关联、还原原来的默认值、全部清掉。
    #[test]
    fn add_and_remove_restores_previous() {
        let ext = "onezip-assoc-test";
        let hk = RegKey::predef(HKEY_CURRENT_USER);
        let dot = format!(r"Software\Classes\.{ext}");
        hk.create_subkey(&dot).unwrap().0.set_value("", &"Someone.Else").unwrap();
        let exe = std::path::Path::new(r"C:\x\OneZip.exe");

        set(exe, ext, true).unwrap();
        assert!(associated(ext));
        let cmd: String = hk
            .open_subkey(format!(r"Software\Classes\OneZip.{ext}\shell\open\command"))
            .unwrap()
            .get_value("")
            .unwrap();
        assert_eq!(cmd, r#""C:\x\OneZip.exe" --open "%1""#);

        set(exe, ext, false).unwrap();
        assert!(!associated(ext));
        let back: String = hk.open_subkey(&dot).unwrap().get_value("").unwrap();
        assert_eq!(back, "Someone.Else");
        assert!(hk.open_subkey(format!(r"Software\Classes\OneZip.{ext}")).is_err());
        hk.delete_subkey_all(&dot).unwrap();
    }
}
