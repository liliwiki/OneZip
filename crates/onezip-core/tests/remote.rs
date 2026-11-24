//! 边下载边解压：起一个支持 Range 的本地 HTTP 服务器，从它解一个 tar.zst。

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread;

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

/// 最小的 HTTP/1.1 服务器：HEAD、GET、单区间 Range，长连接。
fn serve(body: Arc<Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { break };
            let body = body.clone();
            thread::spawn(move || {
                let _ = handle(conn, &body);
            });
        }
    });
    format!("http://{addr}/files/sample.tar.zst")
}

fn handle(conn: TcpStream, body: &[u8]) -> std::io::Result<()> {
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut out = conn;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let method = line.split_whitespace().next().unwrap_or("").to_string();
        let mut range = None;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h)?;
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':')
                && k.eq_ignore_ascii_case("range")
            {
                range = parse_range(v.trim(), body.len() as u64);
            }
        }
        let len = body.len() as u64;
        let (status, start, end) = match range {
            Some((a, b)) => ("206 Partial Content", a, b),
            None => ("200 OK", 0, len.saturating_sub(1)),
        };
        let n = if len == 0 { 0 } else { end - start + 1 };
        let mut head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {n}\r\nAccept-Ranges: bytes\r\n\
             ETag: \"onezip-test\"\r\nLast-Modified: Mon, 01 Jan 2024 00:00:00 GMT\r\n\
             Content-Type: application/octet-stream\r\n"
        );
        if range.is_some() {
            head.push_str(&format!("Content-Range: bytes {start}-{end}/{len}\r\n"));
        }
        head.push_str("\r\n");
        out.write_all(head.as_bytes())?;
        if method != "HEAD" && n > 0 {
            out.write_all(&body[start as usize..=end as usize])?;
        }
        out.flush()?;
        // 读掉可能的请求体（本测试里没有）。
        let _ = reader.by_ref().take(0).read_to_end(&mut Vec::new());
    }
}

fn parse_range(v: &str, len: u64) -> Option<(u64, u64)> {
    let spec = v.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let (a, b) = match (a.parse::<u64>().ok(), b.parse::<u64>().ok()) {
        (Some(a), Some(b)) => (a, b.min(len - 1)),
        (Some(a), None) => (a, len - 1),
        (None, Some(suffix)) => (len.saturating_sub(suffix), len - 1),
        _ => return None,
    };
    (a <= b && b < len).then_some((a, b))
}

#[test]
fn extracts_while_downloading() {
    let body = Arc::new(make_tar_zst());
    let total = body.len() as u64;
    let url = serve(body);

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
