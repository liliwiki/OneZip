//! RAR：分卷、密码、加密文件名、RAR4 旧式分卷、固实，以及边解边删。
//!
//! `tests/fixtures/rar/` 来自 HatchZip 仓库（MIT OR Apache-2.0），用 RAR 6.24 制作，
//! 密码是 `secret`。每个包里都有下面这些文件（另外几个 `*.pak` 是随机数据，只核对大小）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use onezip_core::{ExtractRequest, Observer, Outcome, Source};

const PW: &str = "secret";

fn quiet() -> Observer {
    Arc::new(|_| {})
}

fn stop() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// 每个 fixture 都有的、内容已知的文件。
fn known_files() -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    m.insert("game/config/settings.ini".into(), b"volume=7\n".repeat(2_000));
    m.insert("game/empty.txt".into(), Vec::new());
    m.insert(
        "game/naïve résumé/日本語.txt".into(),
        b"unicode names survive\n".to_vec(),
    );
    for i in 0..10 {
        m.insert(
            format!("game/many/file{i:02}.txt"),
            format!("file number {i}\n").into_bytes(),
        );
    }
    m
}

/// 把一组 fixture 复制到临时目录（边解边删会改动它们），返回第一卷。
fn copy_set(set: &str, to: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rar").join(set);
    let dir = to.join(set);
    fs::create_dir_all(&dir).unwrap();
    let mut first = None;
    for e in fs::read_dir(&src).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        fs::copy(&p, dir.join(&name)).unwrap();
        if name == "set.rar" || name == "set.part1.rar" {
            first = Some(dir.join(&name));
        }
    }
    first.unwrap()
}

fn extract(archive: &Path, out: &Path, password: Option<&str>, keep: bool) -> anyhow::Result<()> {
    let req = ExtractRequest {
        source: Source::Local(archive.to_path_buf()),
        output: Some(out.to_path_buf()),
        keep_source: keep,
        password: password.map(str::to_string),
        keep_free: 0,
        ..Default::default()
    };
    match onezip_core::extract(&req, quiet(), &stop(), &mut |_| true)? {
        Outcome::Finished { .. } => Ok(()),
        Outcome::Paused => anyhow::bail!("paused"),
    }
}

fn check(out: &Path) {
    let tree = common_tree(out);
    for (name, data) in known_files() {
        assert_eq!(tree.get(&name), Some(&data), "{name}");
    }
    assert_eq!(tree.get("game/d.pak").map(Vec::len), Some(6_000));
}

fn common_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, m: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, m);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                m.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(root, root, &mut m);
    m
}

#[test]
fn rar5_volumes_keep_source() {
    let dir = tempfile::tempdir().unwrap();
    let a = copy_set("rar5", dir.path());
    let out = dir.path().join("out");
    extract(&a, &out, None, true).unwrap();
    check(&out);
    let tree = common_tree(&out);
    for (name, len) in [("a.pak", 30_000), ("b.pak", 20_000), ("c.pak", 36_000), ("e.pak", 6_000)] {
        assert_eq!(tree.get(&format!("game/{name}")).map(Vec::len), Some(len), "{name}");
    }
    assert!(a.exists(), "原压缩包应保留");
}

#[test]
fn rar4_old_style_volumes() {
    let dir = tempfile::tempdir().unwrap();
    let a = copy_set("rar4", dir.path());
    let out = dir.path().join("out");
    extract(&a, &out, None, true).unwrap();
    check(&out);
}

#[test]
fn rar5_volumes_extract_in_place_removes_every_volume() {
    let dir = tempfile::tempdir().unwrap();
    let a = copy_set("rar5", dir.path());
    let out = dir.path().join("out");
    extract(&a, &out, None, false).unwrap();
    check(&out);
    let left: Vec<_> = fs::read_dir(a.parent().unwrap()).unwrap().collect();
    assert!(left.is_empty(), "所有分卷都应在解压过程中删掉，还剩 {}", left.len());
}

#[test]
fn passwords_rar4_and_rar5() {
    for set in ["rar5-pw", "rar4-pw"] {
        let dir = tempfile::tempdir().unwrap();
        let a = copy_set(set, dir.path());
        let out = dir.path().join("out");
        let err = extract(&a, &out, None, true).unwrap_err();
        assert!(onezip_core::needs_password(&err), "{set}: {err:#}");
        let err = extract(&a, &out, Some("wrong"), true).unwrap_err();
        assert!(onezip_core::needs_password(&err), "{set}: {err:#}");
        extract(&a, &out, Some(PW), true).unwrap();
        check(&out);
    }
}

#[test]
fn encrypted_file_names_need_the_password_even_to_list() {
    for set in ["rar5-hp", "rar5-hp-set", "rar4-hp-set"] {
        let dir = tempfile::tempdir().unwrap();
        let a = copy_set(set, dir.path());
        let err = onezip_core::list(&a, None).unwrap_err();
        assert!(onezip_core::needs_password(&err), "{set}: {err:#}");
        let entries = onezip_core::list(&a, Some(PW.into())).unwrap();
        assert!(entries.iter().any(|e| e.path.ends_with("settings.ini")), "{set}");
        let out = dir.path().join("out");
        extract(&a, &out, Some(PW), true).unwrap();
        check(&out);
    }
}

#[test]
fn solid_archives_extract_when_kept() {
    for set in ["rar5-solid", "rar4-solid"] {
        let dir = tempfile::tempdir().unwrap();
        let a = copy_set(set, dir.path());
        let out = dir.path().join("out");
        extract(&a, &out, None, true).unwrap();
        check(&out);
    }
}

#[test]
fn test_reports_a_good_archive() {
    let dir = tempfile::tempdir().unwrap();
    let a = copy_set("rar5", dir.path());
    onezip_core::test(&a, None, quiet()).unwrap();
}
