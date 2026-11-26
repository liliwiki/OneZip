//! 浏览压缩包：列出全部项，只解出选中的。
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use onezip_core::browse::{self, PickRequest};
use onezip_core::{CompressRequest, Conflict, Format};

fn make(root: &Path, format: Format, password: Option<&str>) -> PathBuf {
    let src = root.join("资料");
    fs::create_dir_all(src.join("子/空")).unwrap();
    fs::write(src.join("a.txt"), b"aaa").unwrap();
    fs::write(src.join("子/b.txt"), b"bbbb").unwrap();
    fs::write(src.join("子/c.txt"), b"cc").unwrap();
    let out = root.join(format!("pack.{}", format.extension()));
    let mut req = CompressRequest::new(vec![src], out.clone(), format);
    req.password = password.map(str::to_string);
    onezip_core::compress(&req, Arc::new(|_| {}), &Arc::new(AtomicBool::new(false))).unwrap();
    out
}

fn pick(archive: &Path, picks: &[&str], base: &str, output: &Path, conflict: Conflict) -> anyhow::Result<PathBuf> {
    let req = PickRequest {
        archive: archive.to_path_buf(),
        password: None,
        picks: picks.iter().map(|s| s.to_string()).collect(),
        base: base.into(),
        output: output.to_path_buf(),
        conflict,
    };
    browse::extract(&req, Arc::new(|_| {}), &Arc::new(AtomicBool::new(false)))
}

#[test]
fn list_and_pick() {
    for format in [Format::SevenZ, Format::Zip] {
        let t = tempfile::tempdir().unwrap();
        let arc = make(t.path(), format, None);
        let items = browse::items(&arc, None).unwrap();
        let mut names: Vec<_> = items.iter().map(|i| (i.path.as_str(), i.is_dir)).collect();
        names.sort();
        assert!(names.contains(&("资料/子/b.txt", false)), "{format:?} {names:?}");
        assert!(names.contains(&("资料/子/空", true)), "{format:?} {names:?}");
        assert!(items.iter().all(|i| i.is_dir || i.modified.is_some()));

        // 在"资料"文件夹里选了"子"：解出的是 out/子/…，不带"资料"这一层。
        let out = t.path().join("out");
        assert_eq!(pick(&arc, &["资料/子"], "资料", &out, Conflict::Refuse).unwrap(), out);
        assert_eq!(fs::read(out.join("子/b.txt")).unwrap(), b"bbbb");
        assert_eq!(fs::read(out.join("子/c.txt")).unwrap(), b"cc");
        assert!(out.join("子/空").is_dir());
        assert!(!out.join("a.txt").exists());

        // 再来一次：同名文件
        fs::write(out.join("子/b.txt"), b"MINE").unwrap();
        let e = pick(&arc, &["资料/子/b.txt"], "资料", &out, Conflict::Refuse).unwrap_err();
        let same = onezip_core::same_names(&e).expect("same names");
        assert_eq!(same.files, [PathBuf::from("子\\b.txt")]);
        assert_eq!(fs::read(out.join("子/b.txt")).unwrap(), b"MINE");
        pick(&arc, &["资料/子/b.txt"], "资料", &out, Conflict::Rename).unwrap();
        assert_eq!(fs::read(out.join("子/b (2).txt")).unwrap(), b"bbbb");
        pick(&arc, &["资料/子/b.txt"], "资料", &out, Conflict::Skip).unwrap();
        assert_eq!(fs::read(out.join("子/b.txt")).unwrap(), b"MINE");
        pick(&arc, &["资料/子/b.txt"], "资料", &out, Conflict::Replace).unwrap();
        assert_eq!(fs::read(out.join("子/b.txt")).unwrap(), b"bbbb");
    }
}

#[test]
fn encrypted_needs_password() {
    let t = tempfile::tempdir().unwrap();
    // 7z 默认加密文件名：不给密码连列表都打不开
    let arc = make(t.path(), Format::SevenZ, Some("pw"));
    assert!(onezip_core::needs_password(&browse::items(&arc, None).unwrap_err()));
    assert!(onezip_core::needs_password(&browse::items(&arc, Some("bad".into())).unwrap_err()));
    assert!(!browse::items(&arc, Some("pw".into())).unwrap().is_empty());

    // zip 只加密内容：能列出，解出时要密码
    let t2 = tempfile::tempdir().unwrap();
    let zip = make(t2.path(), Format::Zip, Some("pw"));
    assert!(!browse::items(&zip, None).unwrap().is_empty());
    let out = t2.path().join("out");
    let e = pick(&zip, &["资料/a.txt"], "资料", &out, Conflict::Refuse).unwrap_err();
    assert!(onezip_core::needs_password(&e), "{e:#}");
    assert!(!out.join("a.txt").exists());
}
