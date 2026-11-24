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
use peel::coordinator::local::{LocalRunArgs, run as run_local};
use peel::coordinator::{
    CoordinatorConfig, CoordinatorError, OutputTarget, ProgressEvent, RunArgs, run,
};
use peel::decode::{DecoderRegistry, FormatShape};
use peel::http::Client;
use peel::progress::ProgressState;

use crate::{Confirm, Event, ExtractRequest, Observer, Outcome, Placement};

/// 本地文件里交给 Peel 的格式：hatchzip-core 0.1.10 不支持的那些。
pub(crate) fn handles_local(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    [".zst", ".tzst", ".lz4", ".rar"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// 用 Peel 解本地文件。`keep_source` 为假时边解边打洞，完成后删除原文件。
/// 进度检查点在原文件旁边，同样的请求再调一次即从断点继续。
pub(crate) fn extract_file(
    archive: &Path,
    req: &ExtractRequest,
    observer: Observer,
    stop: &Arc<AtomicBool>,
    confirm: &mut dyn FnMut(&Confirm) -> bool,
) -> Result<Outcome> {
    if req.password.is_some() {
        bail!("这种格式的加密压缩包暂不支持");
    }
    let archive = std::path::absolute(archive)?;
    let registry = DecoderRegistry::with_defaults();
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = archive.parent().map(Path::to_path_buf).unwrap_or_default();
    let shape = registry.shape_for_name(&name);
    let stem = strip_archive_extensions(&name).to_string();
    let output = match (&req.output, shape) {
        (Some(p), Some(FormatShape::Stream)) => OutputTarget::File(std::path::absolute(p)?),
        (Some(p), _) => OutputTarget::Dir(std::path::absolute(p)?),
        (None, Some(FormatShape::Stream)) => OutputTarget::File(parent.join(&stem)),
        (None, _) if req.placement == Placement::Here => OutputTarget::Dir(parent.clone()),
        (None, _) => OutputTarget::Dir(parent.join(if stem.is_empty() || stem == name {
            format!("{name}-extracted")
        } else {
            stem
        })),
    };
    let out_path = match &output {
        OutputTarget::File(p) | OutputTarget::Dir(p) => p.clone(),
    };
    let resuming = peel_checkpoint_exists(&archive);
    if !req.keep_source
        && !resuming
        && !confirm(&Confirm {
            archive: &name,
            output: &out_path,
        })
    {
        bail!("已取消，压缩包没有任何改动");
    }

    let size = std::fs::metadata(&archive)?.len();
    observer(Event::Started {
        total: size,
        resuming,
    });
    let state = ProgressState::new();
    let (done, sampler) = sample(&state, observer.clone(), size);

    let mut args = LocalRunArgs::new(archive.clone(), output);
    args.destructive = !req.keep_source;
    args.registry = registry;
    args.progress_state = Some(state.clone());
    args.kill_switch = Some(stop.clone());
    let result = run_local(args);
    done.store(true, Ordering::Relaxed);
    let _ = sampler.join();

    match result {
        Ok(_) => Ok(Outcome::Finished {
            output: out_path,
            warning: None,
        }),
        Err(CoordinatorError::Aborted { .. }) => Ok(Outcome::Paused),
        Err(e) => Err(anyhow!(e)).with_context(|| format!("解压 {name} 失败")),
    }
}

fn peel_checkpoint_exists(archive: &Path) -> bool {
    let Some(dir) = archive.parent() else {
        return false;
    };
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(&name) && n.contains(".peel.")
        })
    })
}

/// Peel 的进度是一组原子计数器；单开一个线程定期取样转成 Event。
/// 本地解压时用"已解码的源字节数"当进度。
fn sample(
    state: &Arc<ProgressState>,
    observer: Observer,
    local_total: u64,
) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
    let done = Arc::new(AtomicBool::new(false));
    let handle = {
        let (state, done) = (state.clone(), done.clone());
        thread::spawn(move || {
            let mut last = 0;
            while !done.load(Ordering::Relaxed) {
                let s = state.snapshot();
                if local_total > 0 {
                    let now = s.bytes_decoded_input.min(local_total);
                    if now > last {
                        observer(Event::Progressed(now - last));
                        last = now;
                    }
                } else {
                    observer(Event::Remote {
                        downloaded: s.bytes_downloaded,
                        extracted: s.bytes_extracted,
                        total: s.total_size.unwrap_or(0),
                    });
                }
                thread::sleep(Duration::from_millis(200));
            }
        })
    };
    (done, handle)
}

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

    let (done, sampler) = sample(&state, observer.clone(), 0);

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
