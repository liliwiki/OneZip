//! URL：交给 Peel 的 coordinator，边下载边解压，用过的下载数据打洞释放。
//!
//! 断点续传靠 Peel 放在输出旁边的 `.peel.part` / `.peel.ckpt`：用同一个 URL 和
//! 同一个输出位置再调一次，就会从上次的检查点继续。

use onezip_i18n::t;
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
        bail!(t!("这种格式的加密压缩包暂不支持"));
    }
    let archive = std::path::absolute(archive)?;
    let registry = DecoderRegistry::with_defaults();
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = archive.parent().map(Path::to_path_buf).unwrap_or_default();
    let shape = registry.shape_for_name(&name);
    let stem = onezip_formats::archive_stem(&name);
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
    // 智能解压：事先看不到里面有什么，先解到新文件夹，完成后只有一项就挪出来。
    let smart = req.output.is_none()
        && req.placement == Placement::Smart
        && matches!(output, OutputTarget::Dir(_))
        && (resuming || crate::smart::is_empty_dir(&out_path));
    if !req.keep_source
        && !resuming
        && !confirm(&Confirm {
            archive: &name,
            output: &out_path,
        })
    {
        bail!(t!("已取消，压缩包没有任何改动"));
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
    args.kill_switch = Some(forward_stop(stop, &done));
    let result = run_local(args);
    done.store(true, Ordering::Relaxed);
    let _ = sampler.join();

    match result {
        Ok(_) => Ok(Outcome::Finished {
            output: if smart { crate::smart::flatten(&out_path) } else { out_path },
            warning: None,
        }),
        Err(CoordinatorError::Aborted { .. }) if stop.load(Ordering::Relaxed) => Ok(Outcome::Paused),
        Err(e) => Err(anyhow!(e)).with_context(|| t!("解压 {} 失败", name)),
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
    let registry = DecoderRegistry::with_defaults();
    let output = output_target(url, req.output.as_deref(), &registry)?;
    let out_path = match &output {
        OutputTarget::File(p) | OutputTarget::Dir(p) => p.clone(),
    };
    // 智能解压：解完后文件夹里只有一项就挪出来（只动这次新建的文件夹）。
    let smart = req.placement == Placement::Smart
        && matches!(output, OutputTarget::Dir(_))
        && (peel_checkpoint_exists(&out_path) || crate::smart::is_empty_dir(&out_path));

    let client = Client::new().map_err(|e| anyhow!(e)).context(t!("创建 HTTP 客户端失败"))?;
    let state = ProgressState::new();
    let password = Password::set(req.password.as_deref());

    let (done, sampler) = sample(&state, observer.clone(), 0);
    // Peel 出错时会把传给它的停止开关置位（用来叫停下载线程），所以给它单独一个开关，
    // 只把用户的暂停转发过去；否则出错会被当成暂停，而且用户的开关也被弄脏。
    let kill = forward_stop(stop, &done);

    let started = observer.clone();
    let args = RunArgs {
        url: url.to_string(),
        additional_urls: Vec::new(),
        output,
        config: CoordinatorConfig {
            password_source: password.source(),
            workers: if req.connections > 0 { req.connections } else { CoordinatorConfig::default().workers },
            ..CoordinatorConfig::default()
        },
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
        kill_switch: Some(kill),
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
                output: if smart { crate::smart::flatten(&out_path) } else { out_path },
                warning: None,
            })
        }
        Err(CoordinatorError::Aborted { .. }) if stop.load(Ordering::Relaxed) => Ok(Outcome::Paused),
        Err(e) => Err(anyhow!(e)).with_context(|| t!("从 {} 解压失败", url)),
    }
}

/// 给 Peel 的停止开关：用户叫停时跟着置位，`done` 之后不再转发。
fn forward_stop(stop: &Arc<AtomicBool>, done: &Arc<AtomicBool>) -> Arc<AtomicBool> {
    let kill = Arc::new(AtomicBool::new(stop.load(Ordering::Relaxed)));
    let (stop, done, k) = (stop.clone(), done.clone(), kill.clone());
    thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            if stop.load(Ordering::Relaxed) {
                k.store(true, Ordering::Relaxed);
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
    kill
}

/// 交给 Peel 的密码。Peel 只能从环境变量、文件或终端读密码，这里用一个本次专用的
/// 环境变量（不落盘），结束时删掉。
struct Password(Option<String>);

impl Password {
    fn set(pw: Option<&str>) -> Password {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Password(pw.map(|pw| {
            let name = format!("ONEZIP_PASSWORD_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
            // SAFETY: Windows 上读写环境变量由系统加锁（SetEnvironmentVariableW），没有
            // Unix 上 setenv 与其他线程 getenv 并发的问题；变量名每次都不同，不会互相覆盖。
            unsafe { std::env::set_var(&name, pw) };
            name
        }))
    }

    fn source(&self) -> Option<peel::secret::source::PasswordSource> {
        self.0.as_ref().map(|n| peel::secret::source::PasswordSource::Env(n.into()))
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        if let Some(n) = &self.0 {
            // SAFETY: 同上。
            unsafe { std::env::remove_var(n) };
        }
    }
}

/// 输出位置：用户给了就用；没给就取 URL 的文件名去掉压缩扩展名，放在当前目录。
/// 单流格式（`.zst`、`.gz` 等）输出一个文件，其余输出一个目录。
fn output_target(
    url: &str,
    given: Option<&Path>,
    registry: &DecoderRegistry,
) -> Result<OutputTarget> {
    let name = crate::source::url_file_name(url);
    let shape = name.as_deref().and_then(|n| registry.shape_for_name(n));
    let path = match given {
        Some(p) => p.to_path_buf(),
        None => {
            let stem = name
                .as_deref()
                .map(onezip_formats::archive_stem)
                .ok_or_else(|| anyhow!(t!("无法从 URL 推断输出名，请指定输出位置")))?;
            PathBuf::from(stem)
        }
    };
    let path = std::path::absolute(&path)?;
    Ok(match shape {
        Some(FormatShape::Stream) => OutputTarget::File(path),
        _ => OutputTarget::Dir(path),
    })
}
