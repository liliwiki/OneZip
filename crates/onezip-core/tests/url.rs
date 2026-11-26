//! 边下载边解压的各种格式：本地起一个支持 Range 的 HTTP 服务器。
//!
//! - 7z.dll 的格式（iso、cab、wim、lzh……）：解出的内容必须和本地保留原包解出的一样，
//!   完成后下载用的临时文件夹要清理掉；中途暂停再继续，结果不变。
//! - Peel 的格式（zip、7z）带密码：没给密码时报"需要密码"，给了就能解。

#![cfg(windows)]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use onezip_core::{CompressRequest, Event, ExtractRequest, Format, Observer, Outcome, Source};

fn quiet() -> Observer {
    Arc::new(|_| {})
}

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

/// 本地保留原包解一遍，作为参照。
fn reference(archive: &Path) -> BTreeMap<String, Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join(archive.file_name().unwrap());
    fs::copy(archive, &a).unwrap();
    let out = dir.path().join("ref");
    let req = ExtractRequest {
        source: Source::Local(a),
        output: Some(out.clone()),
        keep_source: true,
        ..Default::default()
    };
    onezip_core::extract(&req, quiet(), &Arc::new(AtomicBool::new(false)), &mut |_| true).unwrap();
    common::read_tree(&out)
}

fn from_url(url: &str, out: &Path, password: Option<&str>, observer: Observer, stop: &Arc<AtomicBool>) -> anyhow::Result<Outcome> {
    let req = ExtractRequest {
        source: Source::parse(url),
        output: Some(out.to_path_buf()),
        password: password.map(str::to_string),
        ..Default::default()
    };
    onezip_core::extract(&req, observer, stop, &mut |_| true)
}

fn work_dir(out: &Path) -> PathBuf {
    let mut n = out.file_name().unwrap().to_os_string();
    n.push(".onezip-download");
    out.with_file_name(n)
}

/// 从 URL 解 `rel`（以 `name` 的名字提供），和本地解的结果比较。
fn same_as_local(rel: &str, name: &str) {
    let src = fixture(rel);
    let want = reference(&src);
    let url = common::http::serve(Arc::new(fs::read(&src).unwrap()), name, 0);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let r = from_url(&url, &out, None, quiet(), &Arc::new(AtomicBool::new(false))).unwrap();
    assert!(matches!(r, Outcome::Finished { .. }), "{rel}");
    assert_eq!(common::read_tree(&out), want, "{rel}");
    assert!(!work_dir(&out).exists(), "{rel}: 下载用的临时文件夹没有清理");
}

#[test]
fn iso_from_url() {
    same_as_local("sevenzip/iso/joliet.iso", "joliet.iso");
    same_as_local("sevenzip/iso/udf.iso", "udf.iso");
}

#[test]
fn cab_wim_lzh_from_url() {
    same_as_local("sevenzip/cab/lzx.cab", "lzx.cab");
    same_as_local("sevenzip/wim/sample.wim", "sample.wim");
    same_as_local("sevenzip/lzh/lh7.lzh", "lh7.lzh");
}

/// 不能边解边删的格式（这里把 ISO 当 .img 提供）：照样边下载边解，解完删掉下载的文件。
#[test]
fn disk_image_from_url() {
    same_as_local("sevenzip/iso/joliet.iso", "disk.img");
}

#[test]
fn pause_and_resume_from_url() {
    let src = fixture("sevenzip/iso/many.iso");
    let want = reference(&src);
    // 限速，好在中途叫停
    let url = common::http::serve(Arc::new(fs::read(&src).unwrap()), "many.iso", 2000);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");

    let stop = Arc::new(AtomicBool::new(false));
    let observer: Observer = {
        let stop = stop.clone();
        Arc::new(move |e| {
            if let Event::Remote { downloaded, .. } = e
                && downloaded > 1 << 20
            {
                stop.store(true, Ordering::SeqCst);
            }
        })
    };
    let first = from_url(&url, &out, None, observer, &stop).unwrap();
    assert!(matches!(first, Outcome::Paused), "应该暂停");
    assert!(work_dir(&out).exists(), "暂停时下载进度要留着");

    let second = from_url(&url, &out, None, quiet(), &Arc::new(AtomicBool::new(false))).unwrap();
    assert!(matches!(second, Outcome::Finished { .. }), "应该完成");
    assert_eq!(common::read_tree(&out), want);
    assert!(!work_dir(&out).exists());
}

fn encrypted(format: Format, dir: &Path) -> Vec<u8> {
    let src = dir.join("src");
    for (name, bytes) in common::sample_files() {
        let p = src.join(&name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
    let output = dir.join(format!("x.{}", format.extension()));
    let req = CompressRequest {
        inputs: vec![src],
        output: output.clone(),
        format,
        level: 1,
        password: Some("pw-123".into()),
        solid: false,
        encrypt_names: true,
        threads: 0,
        overwrite: true,
    };
    onezip_core::compress(&req, quiet(), &Arc::new(AtomicBool::new(false))).unwrap();
    fs::read(output).unwrap()
}

#[test]
fn encrypted_zip_and_7z_from_url() {
    for format in [Format::Zip, Format::SevenZ] {
        let dir = tempfile::tempdir().unwrap();
        let body = encrypted(format, dir.path());
        let url = common::http::serve(Arc::new(body), &format!("secret.{}", format.extension()), 0);
        let stop = Arc::new(AtomicBool::new(false));

        let err = from_url(&url, &dir.path().join("a"), None, quiet(), &stop).unwrap_err();
        assert!(onezip_core::needs_password(&err), "{format:?}: 没给密码应提示需要密码：{err:#}");

        let err = from_url(&url, &dir.path().join("b"), Some("wrong"), quiet(), &stop).unwrap_err();
        assert!(onezip_core::needs_password(&err), "{format:?}: 密码不对应提示重新输入：{err:#}");

        let out = dir.path().join("c");
        let r = from_url(&url, &out, Some("pw-123"), quiet(), &stop).unwrap();
        assert!(matches!(r, Outcome::Finished { .. }), "{format:?}");
        let mut got = common::read_tree(&out.join("src"));
        got.retain(|k, _| !k.contains(".peel."));
        assert_eq!(got, common::sample_files(), "{format:?}");
    }
}

/// 智能解压：压缩包里只有一个顶层文件夹时，不再多套一层以压缩包命名的文件夹。
#[test]
fn single_root_from_url_is_not_nested() {
    for format in [Format::Zip, Format::SevenZ, Format::TarZst] {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("资料");
        for (name, bytes) in common::sample_files() {
            let p = src.join(&name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, bytes).unwrap();
        }
        let archive = dir.path().join(format!("pack.{}", format.extension()));
        onezip_core::compress(&CompressRequest::new(vec![src.clone()], archive.clone(), format), quiet(), &Arc::new(AtomicBool::new(false))).unwrap();
        fs::remove_dir_all(&src).unwrap();
        let url = common::http::serve(Arc::new(fs::read(&archive).unwrap()), &format!("pack.{}", format.extension()), 0);

        let out = dir.path().join("pack");
        let r = from_url(&url, &out, None, quiet(), &Arc::new(AtomicBool::new(false))).unwrap();
        let listing: Vec<String> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        let Outcome::Finished { output, .. } = r else { panic!("{format:?}: 应该完成") };
        assert_eq!(output, src, "{format:?}: {listing:?}");
        assert!(!out.exists(), "{format:?}: 多套了一层 {listing:?}");
        assert_eq!(common::read_tree(&src), common::sample_files(), "{format:?}");
    }
}
