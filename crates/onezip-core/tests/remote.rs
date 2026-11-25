//! 边下载边解压：起一个支持 Range 的本地 HTTP 服务器，从它解一个 tar.zst。

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use onezip_core::{Event, ExtractRequest, Observer, Outcome, Source};

fn make_tar_zst() -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, bytes) in common::sample_files() {
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, name, bytes.as_slice()).unwrap();
    }
    let raw = tar.into_inner().unwrap();
    zstd::encode_all(raw.as_slice(), 3).unwrap()
}

#[test]
fn extracts_while_downloading() {
    let body = Arc::new(make_tar_zst());
    let total = body.len() as u64;
    let url = common::http::serve(body, "sample.tar.zst", 0);

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let req = ExtractRequest {
        source: Source::parse(&url),
        output: Some(out.clone()),
        ..Default::default()
    };
    let saw_start = Arc::new(AtomicBool::new(false));
    let s = saw_start.clone();
    let observer: Observer = Arc::new(move |ev| {
        if let Event::Started { total: t, .. } = ev {
            assert_eq!(t, total);
            s.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    let stop = Arc::new(AtomicBool::new(false));
    let outcome = onezip_core::extract(&req, observer, &stop, &mut |_| true).unwrap();

    match outcome {
        Outcome::Finished { output, .. } => assert_eq!(output, out),
        Outcome::Paused => panic!("不应暂停"),
    }
    assert!(saw_start.load(std::sync::atomic::Ordering::Relaxed));
    let mut got = common::read_tree(&out);
    // Peel 的断点文件在成功后应已清理；保险起见只比较样本文件。
    got.retain(|k, _| !k.contains(".peel."));
    assert_eq!(got, common::sample_files());
}
