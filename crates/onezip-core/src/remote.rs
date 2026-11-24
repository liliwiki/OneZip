//! URL：交给 Peel 的 coordinator，边下载边解压，用过的下载数据打洞释放。
//!
//! 断点续传靠 Peel 放在输出旁边的 `.peel.part` / `.peel.ckpt`：用同一个 URL 和
//! 同一个输出位置再调一次，就会从上次的检查点继续。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use peel::coordinator::{
    CoordinatorConfig, CoordinatorError, OutputTarget, ProgressEvent, RunArgs, run,
};
use peel::decode::{DecoderRegistry, FormatShape};
use peel::http::Client;
use peel::progress::ProgressState;

use crate::{Event, ExtractRequest, Observer, Outcome};

pub(crate) fn extract(
    url: &str,
    req: &ExtractRequest,
    observer: Observer,
    stop: &Arc<AtomicBool>,
) -> Result<Outcome> {
    if req.password.is_some() {
        bail!("从 URL 解压加密压缩包暂不支持");
    }
    let registry = DecoderRegistry::with_defaults();
    let output = output_target(url, req.output.as_deref(), &registry)?;
    let out_path = match &output {
        OutputTarget::File(p) | OutputTarget::Dir(p) => p.clone(),
    };

    let client = Client::new().map_err(|e| anyhow!(e)).context("创建 HTTP 客户端失败")?;
    let state = ProgressState::new();

    // Peel 的进度是一组原子计数器；单开一个线程定期取样转成 Event。
    let done = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (state, done, observer) = (state.clone(), done.clone(), observer.clone());
        thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let s = state.snapshot();
                observer(Event::Remote {
                    downloaded: s.bytes_downloaded,
                    extracted: s.bytes_extracted,
                    total: s.total_size.unwrap_or(0),
                });
                thread::sleep(Duration::from_millis(200));
            }
        })
    };

    let started = observer.clone();
    let args = RunArgs {
        url: url.to_string(),
        additional_urls: Vec::new(),
        output,
        config: CoordinatorConfig::default(),
        client,
        registry,
        progress: Some(Box::new(move |ev| {
            if let ProgressEvent::Started {
                total_size,
                resuming,
                ..
            } = ev
            {
                started(Event::Started {
                    total: total_size,
                    resuming,
                });
            }
        })),
        progress_state: Some(state.clone()),
        kill_switch: Some(stop.clone()),
        io_backend: None,
    };

    let result = run(args);
    done.store(true, Ordering::Relaxed);
    let _ = sampler.join();

    match result {
        Ok(stats) => {
            observer(Event::Remote {
                downloaded: stats.download.bytes_downloaded,
                extracted: state.snapshot().bytes_extracted,
                total: stats.total_size,
            });
            Ok(Outcome::Finished {
                output: out_path,
                warning: None,
            })
        }
        Err(CoordinatorError::Aborted { .. }) => Ok(Outcome::Paused),
        Err(e) => Err(anyhow!(e)).with_context(|| format!("从 {url} 解压失败")),
    }
}

/// 输出位置：用户给了就用；没给就取 URL 的文件名去掉压缩扩展名，放在当前目录。
/// 单流格式（`.zst`、`.gz` 等）输出一个文件，其余输出一个目录。
fn output_target(
    url: &str,
    given: Option<&Path>,
    registry: &DecoderRegistry,
) -> Result<OutputTarget> {
    let name = url_file_name(url);
    let shape = name.as_deref().and_then(|n| registry.shape_for_name(n));
    let path = match given {
        Some(p) => p.to_path_buf(),
        None => {
            let stem = name
                .as_deref()
                .map(strip_archive_extensions)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("无法从 URL 推断输出名，请指定输出位置"))?;
            PathBuf::from(stem)
        }
    };
    let path = std::path::absolute(&path)?;
    Ok(match shape {
        Some(FormatShape::Stream) => OutputTarget::File(path),
        _ => OutputTarget::Dir(path),
    })
}

fn url_file_name(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.split(['?', '#']).next()?;
    let (_, path) = path.split_once('/')?;
    let last = path.rsplit('/').next()?;
    (!last.is_empty()).then(|| last.to_string())
}

const EXTENSIONS: &[&str] = &[
    ".tar.zst", ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.lz4", ".tgz", ".txz", ".tbz2", ".tbz",
    ".tar", ".zip", ".7z", ".rar", ".zst", ".gz", ".xz", ".bz2", ".lz4",
];

fn strip_archive_extensions(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    for ext in EXTENSIONS {
        if lower.ends_with(ext) {
            return &name[..name.len() - ext.len()];
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            url_file_name("https://h.example/a/b/data.tar.zst?x=1").as_deref(),
            Some("data.tar.zst")
        );
        assert_eq!(url_file_name("https://h.example/"), None);
        assert_eq!(strip_archive_extensions("Data.TAR.ZST"), "Data");
        assert_eq!(strip_archive_extensions("x.zip"), "x");
    }
}
