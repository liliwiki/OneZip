//! 同名文件的四种处理（不动 / 替换 / 跳过 / 改名），两个引擎都要一样：
//! - 保留原包解 zip、7z：7z.dll；
//! - 边解边删 zip：hatchzip-core；
//! - 保留原包解 7z 时暂停再继续，结果不变、不多出改名的文件。

#![cfg(windows)]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use onezip_core::{CompressRequest, Conflict, Event, ExtractRequest, Format, Observer, Outcome, Source};

fn quiet() -> Observer {
    Arc::new(|_| {})
}

fn write_zip(path: &Path, files: &BTreeMap<String, Vec<u8>>) {
    let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        w.start_file(name.as_str(), opts).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap();
}

fn write_7z(dir: &Path, files: &BTreeMap<String, Vec<u8>>) -> PathBuf {
    let src = dir.join("src7");
    for (name, bytes) in files {
        let p = src.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
    let out = dir.join("a.7z");
    let inputs: Vec<PathBuf> = fs::read_dir(&src).unwrap().map(|e| e.unwrap().path()).collect();
    let mut req = CompressRequest::new(inputs, out.clone(), Format::SevenZ);
    req.level = 1;
    onezip_core::compress(&req, quiet(), &Arc::new(AtomicBool::new(false))).unwrap();
    fs::remove_dir_all(src).unwrap();
    out
}

fn files() -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    m.insert("a.txt".into(), b"from archive a".to_vec());
    m.insert("sub/b.txt".into(), b"from archive b".to_vec());
    m.insert("c.txt".into(), b"from archive c".to_vec());
    // 改名时不能撞上压缩包里本来就有的名字
    m.insert("a (2).txt".into(), b"from archive a2".to_vec());
    m
}

fn run(archive: &Path, out: &Path, keep: bool, conflict: Conflict) -> anyhow::Result<Outcome> {
    let req = ExtractRequest {
        source: Source::Local(archive.to_path_buf()),
        output: Some(out.to_path_buf()),
        keep_source: keep,
        keep_free: 0,
        conflict,
        ..Default::default()
    };
    onezip_core::extract(&req, quiet(), &Arc::new(AtomicBool::new(false)), &mut |_| true)
}

/// 输出里事先放好 a.txt、sub/b.txt 两个"用户自己的"文件。
fn prepared(out: &Path) {
    fs::create_dir_all(out.join("sub")).unwrap();
    fs::write(out.join("a.txt"), b"mine a").unwrap();
    fs::write(out.join("sub/b.txt"), b"mine b").unwrap();
}

fn expected(conflict: Conflict) -> BTreeMap<String, Vec<u8>> {
    let mut m = files();
    match conflict {
        Conflict::Replace => {}
        Conflict::Skip => {
            m.insert("a.txt".into(), b"mine a".to_vec());
            m.insert("sub/b.txt".into(), b"mine b".to_vec());
        }
        Conflict::Rename => {
            m.insert("a.txt".into(), b"mine a".to_vec());
            m.insert("a (3).txt".into(), b"from archive a".to_vec());
            m.insert("sub/b.txt".into(), b"mine b".to_vec());
            m.insert("sub/b (2).txt".into(), b"from archive b".to_vec());
        }
        Conflict::Refuse => unreachable!(),
    }
    m
}

fn check(make: impl Fn(&Path) -> PathBuf, keep: bool, label: &str) {
    for conflict in [Conflict::Refuse, Conflict::Replace, Conflict::Skip, Conflict::Rename] {
        let dir = tempfile::tempdir().unwrap();
        let archive = make(dir.path());
        let out = dir.path().join("out");
        prepared(&out);
        let r = run(&archive, &out, keep, conflict);
        if conflict == Conflict::Refuse {
            assert!(r.is_err(), "{label}: 有同名文件时应拒绝");
            assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"mine a", "{label}: 拒绝时不应改动");
            assert!(!out.join("c.txt").exists(), "{label}: 拒绝时不应解出任何文件");
            assert!(archive.exists());
            continue;
        }
        assert!(matches!(r.unwrap(), Outcome::Finished { .. }), "{label} {conflict:?}");
        let mut got = common::read_tree(&out);
        got.retain(|k, _| !k.starts_with(".hatchzip"));
        assert_eq!(got, expected(conflict), "{label} {conflict:?}");
        assert_eq!(archive.exists(), keep, "{label} {conflict:?}");
    }
}

#[test]
fn zip_kept_with_7z_dll() {
    check(
        |d| {
            let a = d.join("a.zip");
            write_zip(&a, &files());
            a
        },
        true,
        "zip 保留",
    );
}

#[test]
fn sevenz_kept_with_7z_dll() {
    check(|d| write_7z(d, &files()), true, "7z 保留");
}

#[test]
fn zip_in_place_with_hatchzip() {
    check(
        |d| {
            let a = d.join("a.zip");
            write_zip(&a, &files());
            a
        },
        false,
        "zip 边解边删",
    );
}

/// 保留原包解 7z（固实）：暂停后继续，结果和一次解完一样，没有多出 "(2)" 文件，进度文件清理掉。
#[test]
fn sevenz_kept_pause_and_resume() {
    let dir = tempfile::tempdir().unwrap();
    let want = common::many_files();
    let archive = write_7z(dir.path(), &want);
    let out = dir.path().join("out");
    let req = ExtractRequest {
        source: Source::Local(archive.clone()),
        output: Some(out.clone()),
        keep_source: true,
        keep_free: 0,
        conflict: Conflict::Rename,
        ..Default::default()
    };
    let stop = Arc::new(AtomicBool::new(false));
    let observer: Observer = {
        let stop = stop.clone();
        let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        Arc::new(move |e| {
            if let Event::Progressed(n) = e
                && seen.fetch_add(n, Ordering::Relaxed) + n > 1 << 20
            {
                stop.store(true, Ordering::Relaxed);
            }
        })
    };
    let first = onezip_core::extract(&req, observer, &stop, &mut |_| true).unwrap();
    assert!(matches!(first, Outcome::Paused), "应该暂停");
    let mut progress = archive.file_name().unwrap().to_os_string();
    progress.push(".onezip-progress");
    assert!(archive.with_file_name(&progress).exists(), "暂停时应留下进度");

    let second = onezip_core::extract(&req, quiet(), &Arc::new(AtomicBool::new(false)), &mut |_| true).unwrap();
    assert!(matches!(second, Outcome::Finished { .. }));
    assert_eq!(common::read_tree(&out), want);
    assert!(!archive.with_file_name(&progress).exists());
    assert!(archive.exists());
}

/// 加密的 zip 没给密码：报需要密码，不留进度文件；给了密码就能解。
#[test]
fn encrypted_zip_kept_with_7z_dll() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    for (name, bytes) in files() {
        let p = src.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
    let archive = dir.path().join("x.zip");
    let mut c = CompressRequest::new(vec![src], archive.clone(), Format::Zip);
    c.password = Some("pw".into());
    onezip_core::compress(&c, quiet(), &Arc::new(AtomicBool::new(false))).unwrap();

    let out = dir.path().join("out");
    let err = run(&archive, &out, true, Conflict::Refuse).unwrap_err();
    assert!(onezip_core::needs_password(&err), "{err:#}");
    assert!(!dir.path().join("x.zip.onezip-progress").exists());

    let req = ExtractRequest {
        source: Source::Local(archive.clone()),
        output: Some(out.clone()),
        keep_source: true,
        password: Some("pw".into()),
        ..Default::default()
    };
    let r = onezip_core::extract(&req, quiet(), &Arc::new(AtomicBool::new(false)), &mut |_| true).unwrap();
    assert!(matches!(r, Outcome::Finished { .. }));
    assert_eq!(common::read_tree(&out.join("src")), files());
}
