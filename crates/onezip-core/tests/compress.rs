//! 压缩后再用 OneZip 自己解回来，逐字节比对；以及智能放置、嵌套包。

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use onezip_core::{
    CompressRequest, Event, ExtractRequest, Format, Observer, Outcome, Placement, Source,
};

fn quiet() -> Observer {
    Arc::new(|_| {})
}

fn stop() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// 在 `root/名字/` 下放好样本文件，返回这个文件夹。
fn source_tree(root: &Path, name: &str) -> std::path::PathBuf {
    let dir = root.join(name);
    for (rel, bytes) in common::sample_files() {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
    dir
}

fn extract_keep(archive: &Path, out: &Path, password: Option<&str>) {
    let req = ExtractRequest {
        source: Source::Local(archive.to_path_buf()),
        output: Some(out.to_path_buf()),
        keep_source: true,
        password: password.map(str::to_string),
        keep_free: 0,
        ..Default::default()
    };
    let r = onezip_core::extract(&req, quiet(), &stop(), &mut |_| true).unwrap();
    assert!(matches!(r, Outcome::Finished { .. }));
}

fn round_trip(format: Format, password: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let src = source_tree(dir.path(), "资料");
    let archive = dir.path().join(format!("pack.{}", format.extension()));
    let mut req = CompressRequest::new(vec![src], archive.clone(), format);
    req.password = password.map(str::to_string);
    let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let s = seen.clone();
    let obs: Observer = Arc::new(move |ev| {
        if let Event::Progressed(n) = ev {
            s.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        }
    });
    onezip_core::compress(&req, obs, &stop()).unwrap();
    assert!(archive.exists());
    let total: u64 = common::sample_files().values().map(|v| v.len() as u64).sum();
    assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), total);

    let out = dir.path().join("out");
    extract_keep(&archive, &out, password);
    assert_eq!(common::read_tree(&out.join("资料")), common::sample_files());
}

#[test]
fn zip_round_trip() {
    round_trip(Format::Zip, None);
}

#[test]
fn zip_encrypted_round_trip() {
    round_trip(Format::Zip, Some("密码123"));
}

#[test]
fn sevenz_round_trip() {
    round_trip(Format::SevenZ, None);
}

#[test]
fn sevenz_encrypted_round_trip() {
    round_trip(Format::SevenZ, Some("密码123"));
}

#[test]
fn tar_zst_round_trip() {
    round_trip(Format::TarZst, None);
}

#[test]
fn tar_xz_round_trip() {
    round_trip(Format::TarXz, None);
}

#[test]
fn tar_gz_round_trip() {
    round_trip(Format::TarGz, None);
}

#[test]
fn wrong_password_is_refused_and_archive_kept() {
    let dir = tempfile::tempdir().unwrap();
    let src = source_tree(dir.path(), "资料");
    let archive = dir.path().join("pack.7z");
    let mut req = CompressRequest::new(vec![src], archive.clone(), Format::SevenZ);
    req.password = Some("right".into());
    onezip_core::compress(&req, quiet(), &stop()).unwrap();
    let before = fs::read(&archive).unwrap();

    let ext = ExtractRequest {
        source: Source::Local(archive.clone()),
        output: Some(dir.path().join("out")),
        password: Some("wrong".into()),
        keep_free: 0,
        ..Default::default()
    };
    assert!(onezip_core::extract(&ext, quiet(), &stop(), &mut |_| true).is_err());
    assert_eq!(fs::read(&archive).unwrap(), before);
}

#[test]
fn refuses_to_overwrite_existing_archive() {
    let dir = tempfile::tempdir().unwrap();
    let src = source_tree(dir.path(), "a");
    let archive = dir.path().join("a.zip");
    fs::write(&archive, b"keep me").unwrap();
    let req = CompressRequest::new(vec![src], archive.clone(), Format::Zip);
    assert!(onezip_core::compress(&req, quiet(), &stop()).is_err());
    assert_eq!(fs::read(&archive).unwrap(), b"keep me");
}

#[test]
fn smart_placement_single_root_goes_beside_archive() {
    let dir = tempfile::tempdir().unwrap();
    let src = source_tree(&dir.path().join("src"), "资料");
    let archive = dir.path().join("pack.zip");
    onezip_core::compress(
        &CompressRequest::new(vec![src], archive.clone(), Format::Zip),
        quiet(),
        &stop(),
    )
    .unwrap();
    let req = ExtractRequest {
        source: Source::Local(archive.clone()),
        placement: Placement::Smart,
        keep_source: true,
        keep_free: 0,
        ..Default::default()
    };
    onezip_core::extract(&req, quiet(), &stop(), &mut |_| true).unwrap();
    // 顶层只有一个"资料"文件夹：直接放在压缩包旁边，不再多套一层 pack/。
    assert_eq!(common::read_tree(&dir.path().join("资料")), common::sample_files());
    assert!(!dir.path().join("pack").exists());
}

#[test]
fn smart_placement_many_roots_get_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let a = source_tree(&dir.path().join("src"), "甲");
    let b = source_tree(&dir.path().join("src"), "乙");
    let archive = dir.path().join("pack.7z");
    onezip_core::compress(
        &CompressRequest::new(vec![a, b], archive.clone(), Format::SevenZ),
        quiet(),
        &stop(),
    )
    .unwrap();
    let req = ExtractRequest {
        source: Source::Local(archive.clone()),
        keep_source: true,
        keep_free: 0,
        ..Default::default()
    };
    let r = onezip_core::extract(&req, quiet(), &stop(), &mut |_| true).unwrap();
    let Outcome::Finished { output, .. } = r else {
        panic!()
    };
    assert_eq!(output, dir.path().join("pack"));
    assert_eq!(common::read_tree(&output.join("甲")), common::sample_files());
    assert_eq!(common::read_tree(&output.join("乙")), common::sample_files());
}

#[test]
fn nested_archive_is_unpacked_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let src = source_tree(&dir.path().join("src"), "资料");
    // 先打成 tar.gz，再把这个 tar.gz 单独放进 zip。
    let inner = dir.path().join("src").join("inner.tar.gz");
    onezip_core::compress(
        &CompressRequest::new(vec![src], inner.clone(), Format::TarGz),
        quiet(),
        &stop(),
    )
    .unwrap();
    let outer = dir.path().join("outer.zip");
    onezip_core::compress(
        &CompressRequest::new(vec![inner], outer.clone(), Format::Zip),
        quiet(),
        &stop(),
    )
    .unwrap();

    let out = dir.path().join("out");
    let req = ExtractRequest {
        source: Source::Local(outer.clone()),
        output: Some(out.clone()),
        nested: true,
        keep_free: 0,
        ..Default::default()
    };
    onezip_core::extract(&req, quiet(), &stop(), &mut |_| true).unwrap();
    assert_eq!(common::read_tree(&out.join("资料")), common::sample_files());
    assert!(!out.join("inner.tar.gz").exists(), "中间包应已删除");
    assert!(!outer.exists(), "外层包按边解边删应已删除");
}

#[test]
fn list_and_test() {
    let dir = tempfile::tempdir().unwrap();
    let src = source_tree(dir.path(), "资料");
    let archive = dir.path().join("pack.zip");
    onezip_core::compress(
        &CompressRequest::new(vec![src], archive.clone(), Format::Zip),
        quiet(),
        &stop(),
    )
    .unwrap();
    let entries = onezip_core::list(&archive, None).unwrap();
    let files = entries.iter().filter(|e| !e.is_dir).count();
    assert_eq!(files, common::sample_files().len());
    onezip_core::test(&archive, None, quiet()).unwrap();

    // 弄坏中间的字节，测试应当失败。
    let mut bytes = fs::read(&archive).unwrap();
    let mid = bytes.len() / 2;
    for b in &mut bytes[mid..mid + 64] {
        *b ^= 0xFF;
    }
    fs::write(&archive, bytes).unwrap();
    assert!(onezip_core::test(&archive, None, quiet()).is_err());
}

#[test]
fn tar_zst_extract_and_remove_via_peel() {
    let dir = tempfile::tempdir().unwrap();
    let src = source_tree(&dir.path().join("src"), "资料");
    let archive = dir.path().join("pack.tar.zst");
    onezip_core::compress(
        &CompressRequest::new(vec![src], archive.clone(), Format::TarZst),
        quiet(),
        &stop(),
    )
    .unwrap();
    let out = dir.path().join("out");
    let req = ExtractRequest {
        source: Source::Local(archive.clone()),
        output: Some(out.clone()),
        keep_free: 0,
        ..Default::default()
    };
    let mut asked = false;
    onezip_core::extract(&req, quiet(), &stop(), &mut |_| {
        asked = true;
        true
    })
    .unwrap();
    assert!(asked, "删除原包前应当确认");
    let mut got = common::read_tree(&out);
    got.retain(|k, _| !k.contains(".peel."));
    assert_eq!(
        got.into_iter()
            .filter_map(|(k, v)| k.strip_prefix("资料/").map(|k| (k.to_string(), v)))
            .collect::<std::collections::BTreeMap<_, _>>(),
        common::sample_files()
    );
    assert!(!archive.exists(), "边解边删完成后原包应已删除");
}
