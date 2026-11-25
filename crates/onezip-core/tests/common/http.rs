//! 测试用的最小 HTTP/1.1 服务器：HEAD、GET、单区间 Range，长连接。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// 起服务器，返回文件的 URL（`http://127.0.0.1:端口/files/<name>`）。
/// `delay_us_per_kb` 不为零时限速。
pub fn serve(body: Arc<Vec<u8>>, name: &str, delay_us_per_kb: u64) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { break };
            let body = body.clone();
            thread::spawn(move || {
                let _ = handle(conn, &body, delay_us_per_kb);
            });
        }
    });
    format!("http://{addr}/files/{name}")
}

fn handle(conn: TcpStream, body: &[u8], delay_us_per_kb: u64) -> std::io::Result<()> {
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
            // 限速：每 64 KB 停一下，模拟慢速网络（测试暂停用）。
            for piece in body[start as usize..=end as usize].chunks(64 << 10) {
                out.write_all(piece)?;
                if delay_us_per_kb > 0 {
                    thread::sleep(Duration::from_micros(delay_us_per_kb * 64));
                }
            }
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

