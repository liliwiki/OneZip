//! 检查更新（不含界面）：问 GitHub 最新发布、比较版本、下载安装包、核对 SHA-256、启动安装。
//!
//! 每次发布带两个文件：`OneZip-<版本>-Setup.exe` 和 `OneZip-<版本>-Setup.exe.sha256`
//! （第一列是十六进制 SHA-256）。核对不上就不运行。安装包用 `--update` 启动：
//! 装到原来的位置、不问问题，装完重新打开 OneZip。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const API: &str = "https://api.github.com/repos/liliwiki/OneZip/releases/latest";
/// 不走接口的办法：这个网页会跳到最新正式版的页面（`…/releases/tag/v0.2.0`）。
const LATEST_PAGE: &str = "https://github.com/liliwiki/OneZip/releases/latest";
const DOWNLOAD: &str = "https://github.com/liliwiki/OneZip/releases/download";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// 不带 v，如 `0.2.0`。
    pub version: String,
    pub page: String,
    pub notes: String,
    pub setup_url: String,
    pub sha_url: String,
}

/// 当前版本。
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 解析 GitHub `releases/latest` 的回答。
pub fn parse(json: &[u8]) -> Result<Release> {
    let v: Value = serde_json::from_slice(json).context("GitHub 的回答看不懂")?;
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow!("没有版本号"))?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let find = |suffix: &str| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?;
            name.ends_with(suffix).then(|| a["browser_download_url"].as_str().map(str::to_string)).flatten()
        })
    };
    let setup_url = find("-Setup.exe").ok_or_else(|| anyhow!("{tag} 里没有安装包"))?;
    let sha_url = find("-Setup.exe.sha256").ok_or_else(|| anyhow!("{tag} 里没有安装包的 SHA-256"))?;
    Ok(Release {
        version,
        page: v["html_url"].as_str().unwrap_or_default().to_string(),
        notes: v["body"].as_str().unwrap_or_default().to_string(),
        setup_url,
        sha_url,
    })
}

fn parts(v: &str) -> Vec<u64> {
    v.split(['-', '+']).next().unwrap_or(v).split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// `a` 是否比 `b` 新（只比数字部分）。
pub fn newer(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (parts(a), parts(b));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// 问 GitHub 最新的正式发布。先用接口（有更新说明）；接口不让用时（不登录每个 IP
/// 每小时只给 60 次，公司、学校等共用出口很容易用完）改看网页跳转，按命名规则拼出下载地址。
pub fn latest() -> Result<Release> {
    match onezip_core::http::get(API).and_then(|json| parse(&json)) {
        Ok(r) => Ok(r),
        Err(api) => from_page(&onezip_core::http::final_url(LATEST_PAGE)?).with_context(|| format!("{api:#}")),
    }
}

/// 从最新版页面的网址得出发布信息（没有更新说明）。
fn from_page(url: &str) -> Result<Release> {
    let tag = url
        .split_once("/releases/tag/")
        .map(|(_, t)| t.trim_end_matches('/'))
        .filter(|t| !t.is_empty() && !t.contains('/'))
        .ok_or_else(|| anyhow!("还没有正式发布的版本"))?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let setup = format!("OneZip-{version}-Setup.exe");
    Ok(Release {
        page: url.to_string(),
        notes: String::new(),
        setup_url: format!("{DOWNLOAD}/{tag}/{setup}"),
        sha_url: format!("{DOWNLOAD}/{tag}/{setup}.sha256"),
        version,
    })
}

/// 下载安装包到临时文件夹并核对 SHA-256，返回安装包路径。
pub fn download(rel: &Release, progress: &dyn Fn(u64, Option<u64>), stop: &AtomicBool) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("OneZip-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let name = rel.setup_url.rsplit('/').next().unwrap_or("OneZip-Setup.exe");
    let path = dir.join(name);
    let sha_text = String::from_utf8_lossy(&onezip_core::http::get(&rel.sha_url)?).into_owned();
    let want = sha_text.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    if want.len() != 64 {
        bail!("SHA-256 文件格式不对");
    }
    onezip_core::http::download(&rel.setup_url, &path, progress, stop)?;
    let got = sha256(&path)?;
    if got != want {
        let _ = std::fs::remove_file(&path);
        bail!("下载的安装包校验不对（SHA-256 不一致），已删除，没有安装");
    }
    Ok(path)
}

fn sha256(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 << 10];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// 启动安装包（更新模式）。调用方随后退出，安装包会等 OneZip 关掉再替换文件。
pub fn run_installer(setup: &Path) -> Result<()> {
    std::process::Command::new(setup)
        .arg("--update")
        .spawn()
        .with_context(|| format!("启动 {} 失败", setup.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.1"));
        assert!(!newer("0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn parses_github_answer() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/liliwiki/OneZip/releases/tag/v0.2.0",
            "body": "新功能",
            "assets": [
                {"name": "OneZip-0.2.0-Setup.exe.sha256", "browser_download_url": "https://x/s.sha256"},
                {"name": "OneZip-0.2.0-Setup.exe", "browser_download_url": "https://x/s.exe"}
            ]
        }"#;
        let r = parse(json.as_bytes()).unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.setup_url, "https://x/s.exe");
        assert_eq!(r.sha_url, "https://x/s.sha256");
        assert_eq!(r.notes, "新功能");
        assert!(parse(br#"{"tag_name": "v1", "assets": []}"#).is_err());
    }

    #[test]
    fn release_from_latest_page() {
        let r = from_page("https://github.com/liliwiki/OneZip/releases/tag/v0.2.0").unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.setup_url, "https://github.com/liliwiki/OneZip/releases/download/v0.2.0/OneZip-0.2.0-Setup.exe");
        assert_eq!(r.sha_url, format!("{}.sha256", r.setup_url));
        // 没有任何正式版时会跳到发布列表
        assert!(from_page("https://github.com/liliwiki/OneZip/releases").is_err());
    }
}
