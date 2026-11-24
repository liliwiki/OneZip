//! 本地边解压边删除：完整跑完、中途暂停再恢复、保留原包、拒绝确认。

mod common;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use onezip_core::{Event, ExtractRequest, Observer, Outcome, Source};
use zip::write::SimpleFileOptions;

fn make_zip(path: &Path) {
    write_zip(path, common::sample_files());
}

fn write_zip(path: &Path, files: std::collections::BTreeMap<String, Vec<u8>>) {
    let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        w.start_file(name, opts).unwrap();
        w.write_all(&bytes).unwrap();
    }
    w.finish().unwrap();
}

fn request(archive: &Path, out: &Path) -> ExtractRequest {
    ExtractRequest {
        source: Source::Local(archive.to_path_buf()),
        output: Some(out.to_path_buf()),
        keep_free: 0,
        ..Default::default()
    }
}

fn quiet() -> Observer {
    Arc::new(|_| {})
}

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("sample.zip");
    make_zip(&archive);
    let out = dir.path().join("out");
    (dir, archive, out)
}

#[test]
fn extracts_and_removes_archive() {
    let (_dir, archive, out) = setup();
    let left = Arc::new(Mutex::new(Vec::new()));
    let l = left.clone();
    let observer: Observer = Arc::new(move |ev| {
        if let Event::ArchiveLeft(n) = ev {
            l.lock().unwrap().push(n);
        }
    });
    let stop = Arc::new(AtomicBool::new(false));
    let outcome =
        onezip_core::extract(&request(&archive, &out), observer, &stop, &mut |_| true).unwrap();

    assert!(matches!(outcome, Outcome::Finished { .. }));
    assert_eq!(common::read_tree(&out), common::sample_files());
    assert!(!archive.exists(), "边解边删完成后原压缩包应已删除");
    // 压缩包是逐步变短的，而不是最后一次性删除。
    let left = left.lock().unwrap();
    assert!(
        left.windows(2).any(|w| w[1] < w[0]),
        "压缩包长度应逐步减少：{left:?}"
    );
}

#[test]
fn pause_then_resume_gives_identical_output() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("many.zip");
    write_zip(&archive, common::many_files());
    let out = dir.path().join("out");
    let original_len = fs::metadata(&archive).unwrap().len();

    // 第一次：第一批提交、压缩包被截短后叫停，模拟用户暂停 / 进程被打断。
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let observer: Observer = Arc::new(move |ev| {
        if let Event::ArchiveLeft(n) = ev
            && n < original_len
        {
            s.store(true, Ordering::Relaxed);
        }
    });
    let first =
        onezip_core::extract(&request(&archive, &out), observer, &stop, &mut |_| true).unwrap();
    assert!(matches!(first, Outcome::Paused), "应在中途暂停");
    let mid_len = fs::metadata(&archive).unwrap().len();
    assert!(mid_len < original_len, "暂停时压缩包应已被截短一部分");

    // 第二次：同样的请求继续。不应再询问确认（已经开始了）。
    let stop = Arc::new(AtomicBool::new(false));
    let second = onezip_core::extract(&request(&archive, &out), quiet(), &stop, &mut |_| {
        panic!("恢复时不应再次确认")
    })
    .unwrap();
    assert!(matches!(second, Outcome::Finished { .. }));
    assert_eq!(common::read_tree(&out), common::many_files());
    assert!(!archive.exists());
}

#[test]
fn keep_source_leaves_archive_untouched() {
    let (_dir, archive, out) = setup();
    let before = fs::read(&archive).unwrap();
    let mut req = request(&archive, &out);
    req.keep_source = true;
    let stop = Arc::new(AtomicBool::new(false));
    let outcome = onezip_core::extract(&req, quiet(), &stop, &mut |_| {
        panic!("保留原包时不需要确认")
    })
    .unwrap();
    assert!(matches!(outcome, Outcome::Finished { .. }));
    assert_eq!(common::read_tree(&out), common::sample_files());
    assert_eq!(fs::read(&archive).unwrap(), before);
}

#[test]
fn declining_confirmation_changes_nothing() {
    let (dir, archive, out) = setup();
    let before = fs::read(&archive).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let err = onezip_core::extract(&request(&archive, &out), quiet(), &stop, &mut |_| false)
        .unwrap_err();
    assert!(format!("{err:#}").contains("已取消"));
    assert_eq!(fs::read(&archive).unwrap(), before);
    // 不应留下日志等任何副产物。
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("sample.zip")]);
}
