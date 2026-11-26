//! 保存的密码：解压加密的压缩包时先挨个试这些，都不对才弹出密码框。
//!
//! 整个列表用 Windows 的 DPAPI 加密（只有当前 Windows 用户能解开），
//! 存在设置同一个注册表键下的二进制值 `Passwords` 里，所以卸载时选"删除设置"会一起删掉，
//! "恢复全部默认"不动它。读不出、解不开时当作没有保存任何密码。

use std::time::{SystemTime, UNIX_EPOCH};

/// 一个保存的密码。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub password: String,
    /// 最近一次用在哪个压缩包上（给用户认出它是哪个密码），手动添加时为空。
    pub note: String,
    /// 最近一次用上或添加的时间（Unix 秒）。
    pub used: u64,
}

const VALUE: &str = "Passwords";
const MAGIC: &[u8; 4] = b"OZP1";

/// 最近用过的排在前面。
pub fn load() -> Vec<Saved> {
    store::get(VALUE).and_then(|b| store::unprotect(&b)).and_then(|b| decode(&b)).unwrap_or_default()
}

pub fn save(list: &[Saved]) -> std::io::Result<()> {
    if list.is_empty() {
        return store::delete(VALUE);
    }
    store::set(VALUE, &store::protect(&encode(list))?)
}

/// 记下这个密码刚用过（没有就加上），排到最前面。`note` 为空时保留原来的。
pub fn remember(password: &str, note: &str) -> std::io::Result<()> {
    if password.is_empty() {
        return Ok(());
    }
    let mut list = load();
    let old = list.iter().position(|s| s.password == password).map(|i| list.remove(i));
    let note = if note.is_empty() { old.map(|o| o.note).unwrap_or_default() } else { note.to_string() };
    list.insert(0, Saved { password: password.to_string(), note, used: now() });
    save(&list)
}

/// 删掉这个密码。
pub fn forget(password: &str) -> std::io::Result<()> {
    let mut list = load();
    list.retain(|s| s.password != password);
    save(&list)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn encode(list: &[Saved]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    for s in list {
        for text in [&s.password, &s.note] {
            out.extend_from_slice(&(text.len() as u32).to_le_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        out.extend_from_slice(&s.used.to_le_bytes());
    }
    out
}

fn decode(mut b: &[u8]) -> Option<Vec<Saved>> {
    b = b.strip_prefix(MAGIC)?;
    fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
        let (head, rest) = b.split_at_checked(n)?;
        *b = rest;
        Some(head)
    }
    fn text(b: &mut &[u8]) -> Option<String> {
        let n = u32::from_le_bytes(take(b, 4)?.try_into().ok()?) as usize;
        String::from_utf8(take(b, n)?.to_vec()).ok()
    }
    let mut list = Vec::new();
    while !b.is_empty() {
        let password = text(&mut b)?;
        let note = text(&mut b)?;
        let used = u64::from_le_bytes(take(&mut b, 8)?.try_into().ok()?);
        list.push(Saved { password, note, used });
    }
    Some(list)
}

#[cfg(windows)]
mod store {
    use std::io;

    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, REG_BINARY};
    use winreg::{RegKey, RegValue};

    /// 额外的"盐"：别的程序用 DPAPI 解不开这份数据，除非也知道它。
    const ENTROPY: &[u8] = b"OneZip saved passwords";

    pub fn get(name: &str) -> Option<Vec<u8>> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(crate::KEY, KEY_READ).ok()?;
        key.get_raw_value(name).ok().map(|v| v.bytes.into_owned())
    }

    pub fn set(name: &str, bytes: &[u8]) -> io::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(crate::KEY)?;
        key.set_raw_value(name, &RegValue { bytes: bytes.to_vec().into(), vtype: REG_BINARY })
    }

    pub fn delete(name: &str) -> io::Result<()> {
        let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(crate::KEY) else { return Ok(()) };
        match key.delete_value(name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    fn blob(b: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: b.len() as u32, pbData: b.as_ptr() as *mut u8 }
    }

    /// 取出 DPAPI 分配的结果并释放。
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
        v
    }

    pub fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        let entropy = blob(ENTROPY);
        unsafe {
            CryptProtectData(
                &blob(plain),
                windows::core::w!("OneZip"),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(io::Error::other)?;
            Ok(take(out))
        }
    }

    pub fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        let entropy = blob(ENTROPY);
        unsafe {
            CryptUnprotectData(&blob(sealed), None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
                .ok()?;
            Some(take(out))
        }
    }
}

#[cfg(not(windows))]
mod store {
    pub fn get(_name: &str) -> Option<Vec<u8>> {
        None
    }
    pub fn set(_name: &str, _bytes: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
    pub fn delete(_name: &str) -> std::io::Result<()> {
        Ok(())
    }
    pub fn protect(plain: &[u8]) -> std::io::Result<Vec<u8>> {
        Ok(plain.to_vec())
    }
    pub fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
        Some(sealed.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_round_trip_and_garbage() {
        let list = vec![
            Saved { password: "pass word!123".into(), note: "资料.zip".into(), used: 5 },
            Saved { password: "密码\n\t".into(), note: String::new(), used: 0 },
        ];
        assert_eq!(decode(&encode(&list)), Some(list));
        assert_eq!(decode(&encode(&[])), Some(vec![]));
        assert_eq!(decode(b"OZP1\xff\xff\xff\xff"), None);
        assert_eq!(decode(b"nope"), None);
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip() {
        let sealed = store::protect(b"secret").unwrap();
        assert_ne!(sealed, b"secret");
        assert_eq!(store::unprotect(&sealed).unwrap(), b"secret");
        assert_eq!(store::unprotect(b"garbage"), None);
    }
}
