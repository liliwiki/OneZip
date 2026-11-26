//! 普通的 HTTPS 下载（检查更新、下载安装包用），复用 Peel 的 HTTP 客户端，不另带一套网络库。

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use peel::http::{Client, ClientConfig, Url};

fn client() -> Result<Client> {
    let config = ClientConfig {
        // 只报程序名和版本，不带任何用户信息（GitHub 的接口要求有 User-Agent）。
        user_agent: Some(format!("OneZip/{}", env!("CARGO_PKG_VERSION"))),
        extra_headers: vec![("Accept".into(), "application/vnd.github+json".into())],
        ..ClientConfig::default()
    };
    Client::with_config(config).map_err(|e| anyhow!(e)).context("创建 HTTP 客户端失败")
}

/// 跟着跳转走到底，返回最后的网址（如 GitHub 的 `releases/latest` 会跳到最新版本的页面）。
pub fn final_url(url: &str) -> Result<String> {
    let url = Url::parse(url).map_err(|e| anyhow!(e))?;
    let head = client()?.head(&url).map_err(|e| anyhow!(e)).with_context(|| format!("连接 {} 失败", url.host()))?;
    if !head.status.is_success() {
        bail!("服务器返回 {} {}", head.status.code, head.status.reason);
    }
    Ok(head.final_url.to_string())
}

/// 下载 `url` 的全部内容到内存（小文件用，最多 4 MB）。
pub fn get(url: &str) -> Result<Vec<u8>> {
    let url = Url::parse(url).map_err(|e| anyhow!(e))?;
    let resp = client()?.get_full(&url).map_err(|e| anyhow!(e)).with_context(|| format!("连接 {} 失败", url.host()))?;
    if !resp.status.is_success() {
        bail!("服务器返回 {} {}", resp.status.code, resp.status.reason);
    }
    let mut body = Vec::new();
    resp.body.take(4 << 20).read_to_end(&mut body)?;
    Ok(body)
}

/// 下载 `url` 到文件 `to`。`progress(已下载, 总大小)`；`stop` 置位时停下并报错。
pub fn download(url: &str, to: &Path, progress: &dyn Fn(u64, Option<u64>), stop: &AtomicBool) -> Result<()> {
    let url = Url::parse(url).map_err(|e| anyhow!(e))?;
    let resp = client()?.get_full(&url).map_err(|e| anyhow!(e)).with_context(|| format!("连接 {} 失败", url.host()))?;
    if !resp.status.is_success() {
        bail!("服务器返回 {} {}", resp.status.code, resp.status.reason);
    }
    let total = resp.headers.get("content-length").and_then(|v| v.trim().parse::<u64>().ok());
    let mut body = resp.body;
    let mut out = BufWriter::new(File::create(to).with_context(|| format!("创建 {}", to.display()))?);
    let mut buf = vec![0u8; 256 << 10];
    let mut done = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            bail!("已取消");
        }
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    out.flush()?;
    if total.is_some_and(|t| t != done) {
        bail!("下载不完整");
    }
    Ok(())
}
