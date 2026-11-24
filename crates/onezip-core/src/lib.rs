//! OneZip 引擎：所有前端（CLI、后台进程、GUI、右键菜单）共用的唯一入口。
//!
//! 按来源分流到两个上游引擎：
//! - 本地文件 → [`hatchzip_core`]：边解压边截断/打洞释放原档空间，
//!   日志落盘后可在断电、被杀后恢复。
//! - HTTP(S) 地址 → [`peel`]：并行分段下载，边下边解，用过的数据打洞释放，
//!   可断点续传。
//!
//! 前端只和这里的 [`ExtractRequest`]、[`Event`]、[`Outcome`] 打交道，不直接接触上游类型，
//! 以后接入 7z.dll 或换引擎不影响前端。

mod local;
mod remote;
mod source;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

pub use source::Source;

/// 字节数的可读形式，如 `1.5 GB`。
pub fn human(bytes: u64) -> String {
    hatchzip_core::human(bytes)
}

/// 一次解压请求。
#[derive(Debug, Clone, Default)]
pub struct ExtractRequest {
    pub source: Source,
    /// 输出目录；`None` 时按"智能解压"规则在源旁边决定。
    pub output: Option<PathBuf>,
    /// 保留原压缩包（不边解边删）。只对本地文件有意义。
    pub keep_source: bool,
    pub password: Option<String>,
    /// 动手前先把整个压缩包校验一遍。对删了就找不回来的本地文件建议打开。
    pub verify_first: bool,
    /// 解压过程中磁盘至少保留的空闲字节数。
    pub keep_free: u64,
    /// 目标位置已有同名文件时替换它们（默认拒绝）。
    pub replace: bool,
}

/// 进度与提示，发给前端显示。
#[derive(Debug, Clone)]
pub enum Event {
    /// 开始（或恢复）。`total` 为本地任务的待写出字节数，远程任务为源文件大小。
    Started { total: u64, resuming: bool },
    /// 本地任务：又写出了这么多字节（增量）。
    Extracted(u64),
    /// 本地任务：原压缩包当前还占着的长度。
    ArchiveLeft(u64),
    /// 远程任务：累计已下载 / 已解出的字节数。
    Remote {
        downloaded: u64,
        extracted: u64,
        total: u64,
    },
    /// 给用户看的一句话。
    Note(String),
}

pub type Observer = Arc<dyn Fn(Event) + Send + Sync>;

/// 在动手删除原压缩包之前向用户确认用的信息。
pub struct Confirm<'a> {
    pub archive: &'a str,
    pub output: &'a Path,
}

#[derive(Debug)]
pub enum Outcome {
    /// 完成。`warning` 是需要转告用户的非致命问题。
    Finished {
        output: PathBuf,
        warning: Option<String>,
    },
    /// 被 `stop` 叫停，进度已落盘，用同样的请求再调一次即可继续。
    Paused,
}

/// 解压一个本地压缩包或一个 URL。
///
/// `stop` 置为 `true` 后会在下一个安全点停下并返回 [`Outcome::Paused`]。
/// `confirm` 只在"首次开始、并且会删除原压缩包"时被调用；返回 `false` 则什么都不动。
pub fn extract(
    req: &ExtractRequest,
    observer: Observer,
    stop: &Arc<AtomicBool>,
    confirm: &mut dyn FnMut(&Confirm) -> bool,
) -> Result<Outcome> {
    match &req.source {
        Source::Local(path) => local::extract(path, req, observer, stop, confirm),
        Source::Url(url) => remote::extract(url, req, observer, stop),
    }
}
