//! 真的连 GitHub 的下载测试（要联网，默认不跑：`cargo test -p onezip-core --test net -- --ignored`）。

use std::sync::atomic::AtomicBool;

const SHA: &str = "https://github.com/liliwiki/OneZip/releases/download/v0.3.0/OneZip-0.3.0-Setup.exe.sha256";

#[test]
#[ignore]
fn get_reads_the_whole_body() {
    let body = onezip_core::http::get(SHA).unwrap();
    let text = String::from_utf8(body).unwrap();
    assert_eq!(text.split_whitespace().next().unwrap().len(), 64, "{text:?}");
}

#[test]
#[ignore]
fn download_writes_the_whole_file() {
    let dir = std::env::temp_dir().join("onezip-net-test");
    std::fs::create_dir_all(&dir).unwrap();
    let to = dir.join("x.sha256");
    onezip_core::http::download(SHA, &to, &|_, _| {}, &AtomicBool::new(false)).unwrap();
    let text = std::fs::read_to_string(&to).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(text.ends_with("OneZip-0.3.0-Setup.exe\n") || text.trim_end().ends_with("OneZip-0.3.0-Setup.exe"), "{text:?}");
}
