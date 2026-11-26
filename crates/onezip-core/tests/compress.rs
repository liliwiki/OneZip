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
fn zip_ascii_password_round_trip() {
    round_trip(Format::Zip, Some("pass word!123"));
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
    let r = onezip_core::extract(&req, quiet(), &stop(), &mut |_| true).unwrap();
    // 报告的是解出的那一项，"打开"会定位到它。
    let Outcome::Finished { output, .. } = r else {
        panic!()
    };
    assert_eq!(output, dir.path().join("资料"));
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

/// 边解边删 tar.zst（Peel）也是智能解压：只有一个顶层文件夹时直接放在压缩包旁边。
#[test]
fn tar_zst_in_place_single_root_goes_beside_archive() {
    let dir = tempfile::tempdir().unwrap();
    let src = source_tree(&dir.path().join("src"), "资料");
    let archive = dir.path().join("pack.tar.zst");
    onezip_core::compress(&CompressRequest::new(vec![src], archive.clone(), Format::TarZst), quiet(), &stop()).unwrap();
    fs::remove_dir_all(dir.path().join("src")).unwrap();
    let req = ExtractRequest {
        source: Source::Local(archive.clone()),
        keep_free: 0,
        ..Default::default()
    };
    let r = onezip_core::extract(&req, quiet(), &stop(), &mut |_| true).unwrap();
    let Outcome::Finished { output, .. } = r else { panic!("应该完成") };
    assert_eq!(output, dir.path().join("资料"));
    assert!(!dir.path().join("pack").exists(), "多套了一层文件夹");
    let mut got = common::read_tree(&output);
    got.retain(|k, _| !k.contains(".peel."));
    assert_eq!(got, common::sample_files());
    assert!(!archive.exists());
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

/// 约 96 MB 不太好压的数据（伪随机字节夹杂文本），多线程编码器要压好一会儿。
fn bulky_tree(root: &Path) -> std::path::PathBuf {
    let dir = root.join("大块");
    fs::create_dir_all(&dir).unwrap();
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..3 {
        let mut buf = Vec::with_capacity(32 << 20);
        while buf.len() < 32 << 20 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            if x.is_multiple_of(4) {
                buf.extend_from_slice(b"OneZip progress test line. ");
            } else {
                buf.extend_from_slice(&x.to_le_bytes());
            }
        }
        fs::write(dir.join(format!("part{i}.bin")), buf).unwrap();
    }
    dir
}

/// 多线程 LZMA2 / XZ：进度要随压缩逐步增加，而不是读完输入就跳到 100%。
fn progress_is_gradual(format: Format) {
    use std::sync::Mutex;
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let src = bulky_tree(dir.path());
    let out = dir.path().join(format!("大块.{}", format.extension()));
    let start = Instant::now();
    let samples: Arc<Mutex<Vec<(f64, u64)>>> = Arc::default();
    let total = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let observer: Observer = {
        let samples = samples.clone();
        let total = total.clone();
        let done = Arc::new(std::sync::atomic::AtomicU64::new(0));
        Arc::new(move |ev| match ev {
            Event::Started { total: t, .. } => total.store(t, std::sync::atomic::Ordering::SeqCst),
            Event::Progressed(n) => {
                let d = done.fetch_add(n, std::sync::atomic::Ordering::SeqCst) + n;
                samples.lock().unwrap().push((start.elapsed().as_secs_f64(), d));
            }
            _ => {}
        })
    };
    let mut req = CompressRequest::new(vec![src.clone()], out.clone(), format);
    req.level = 7;
    onezip_core::compress(&req, observer, &stop()).unwrap();
    let elapsed = start.elapsed().as_secs_f64();
    let total = total.load(std::sync::atomic::Ordering::SeqCst);
    let samples = samples.lock().unwrap();
    // 不超过总量，最后正好到总量。
    assert_eq!(samples.last().unwrap().1, total);
    // 用了一半时间时，进度还远没到头（以前读完输入就是 100%）。
    let half = samples
        .iter()
        .take_while(|(t, _)| *t <= elapsed / 2.0)
        .last()
        .map_or(0, |s| s.1);
    assert!(
        half < total * 9 / 10,
        "{format:?}: {half} of {total} after half of {elapsed:.1}s"
    );
    assert!(samples.len() > 20, "only {} progress updates", samples.len());
    let back = dir.path().join("back");
    extract_keep(&out, &back, None);
    for i in 0..3 {
        let name = format!("part{i}.bin");
        assert_eq!(
            fs::read(back.join("大块").join(&name)).unwrap(),
            fs::read(src.join(&name)).unwrap()
        );
    }
}

#[test]
fn sevenz_progress_is_gradual() {
    progress_is_gradual(Format::SevenZ);
}

#[test]
fn tar_xz_progress_is_gradual() {
    progress_is_gradual(Format::TarXz);
}

/// 压缩中途取消：很快返回“已取消”，不留下临时文件或半截压缩包。
#[test]
fn sevenz_cancel_is_prompt_and_clean() {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let src = bulky_tree(dir.path());
    let out = dir.path().join("大块.7z");
    let stop = stop();
    let observer: Observer = {
        let stop = stop.clone();
        let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        Arc::new(move |ev| {
            if let Event::Progressed(n) = ev
                && seen.fetch_add(n, Ordering::SeqCst) + n >= 8 << 20
            {
                stop.store(true, Ordering::SeqCst);
            }
        })
    };
    let mut req = CompressRequest::new(vec![src], out.clone(), Format::SevenZ);
    req.level = 9;
    let start = Instant::now();
    let err = onezip_core::compress(&req, observer, &stop).unwrap_err();
    let took = start.elapsed();
    assert!(err.chain().any(|c| c.is::<onezip_core::Cancelled>()), "{err:#}");
    assert!(took < Duration::from_secs(10), "cancel took {took:?}");
    let left: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!left.iter().any(|n| n.starts_with("大块.7z")), "{left:?}");
}

/// zip 的中文密码：7-Zip 和现在的 OneZip 用系统代码页（中文 Windows 上是 GBK），
/// 旧版 OneZip 用 UTF-8。两种都要能解开：保留原包（7z.dll）和边解压边删（HatchZip）。
#[test]
fn zip_chinese_password_either_encoding() {
    use std::io::Write;
    let pw = "密码123";
    let encodings = hatchzip_core::archive::zip_password_bytes(pw);
    assert_eq!(encodings.last().unwrap(), pw.as_bytes());
    for (i, bytes) in encodings.iter().enumerate() {
        for keep in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let archive = dir.path().join(format!("pw{i}.zip"));
            let mut z = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
            let opts = zip::write::SimpleFileOptions::default().with_aes_encryption_bytes(zip::AesMode::Aes256, bytes);
            z.start_file("说明.txt", opts).unwrap();
            z.write_all("你好".as_bytes()).unwrap();
            z.finish().unwrap();

            let out = dir.path().join("out");
            let req = ExtractRequest {
                source: Source::Local(archive.clone()),
                output: Some(out.clone()),
                keep_source: keep,
                password: Some(pw.into()),
                keep_free: 0,
                ..Default::default()
            };
            let r = onezip_core::extract(&req, quiet(), &stop(), &mut |_| true)
                .unwrap_or_else(|e| panic!("编码 {i}，保留原包 {keep}：{e:#}"));
            assert!(matches!(r, Outcome::Finished { .. }));
            assert_eq!(fs::read_to_string(out.join("说明.txt")).unwrap(), "你好");
        }
    }
}
