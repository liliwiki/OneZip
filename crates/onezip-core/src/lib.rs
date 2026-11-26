//! OneZip 引擎：所有前端（命令行、图形界面、右键菜单）共用的唯一入口。
//!
//! - 解压本地压缩包 → [`hatchzip_core`]：边解压边截断/打洞释放原包空间，
//!   日志落盘后可在断电、被杀后恢复。
//! - 解压 HTTP(S) 地址 → [`peel`]：并行分段下载，边下边解，用过的数据打洞释放，
//!   可断点续传。
//! - 压缩 → [`compress`]：zip / 7z / tar.zst / tar.xz / tar.gz。
//!
//! 前端只和这里的类型打交道，不直接接触上游类型，以后换引擎或加格式不影响前端。

pub mod browse;
pub mod compress;
pub mod hash;
pub mod http;
mod local;
#[cfg(windows)]
mod sevenzip;
mod remote;
mod route;
mod smart;
mod source;

use onezip_i18n::t;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

pub use compress::{Cancelled, CompressRequest, Format, compress};
pub use hash::{HashKind, HashReport, hash};
pub use local::{Entry, list, test};

/// 压缩包的全部分卷（`a.part1.rar`、`a.part2.rar`…，`a.7z.001`、`a.7z.002`…），
/// 不分卷时就是它自己。`path` 可以是其中任何一卷；只列存在的。
pub fn archive_volumes(path: &Path) -> Vec<PathBuf> {
    use hatchzip_core::archive::storage::{first_volume, volume_path};
    let first = first_volume(path);
    let mut out = Vec::new();
    while let Some(p) = volume_path(&first, out.len()).filter(|p| p.exists()) {
        out.push(p);
    }
    if out.is_empty() {
        out.push(path.to_path_buf());
    }
    out
}

/// 能边解压边删除原压缩包的格式（按扩展名）。前端据此决定是否显示"边解压边删除压缩包"。
pub fn can_extract_in_place(path: &Path) -> bool {
    onezip_formats::in_place(&path.to_string_lossy())
}
pub use source::Source;

/// 这个错误是不是"需要密码 / 密码不对"：前端据此弹出密码框，而不是报失败。
pub fn needs_password(e: &anyhow::Error) -> bool {
    use hatchzip_core::archive::{PasswordNeeded, WrongPassword};
    use peel::encryption::EncryptionError;
    e.chain().any(|c| {
        c.is::<PasswordNeeded>()
            || c.is::<WrongPassword>()
            || matches!(
                c.downcast_ref::<EncryptionError>(),
                Some(EncryptionError::PasswordMissing | EncryptionError::PasswordIncorrect)
            )
            || {
            #[cfg(windows)]
            {
                c.is::<sevenzip::NeedPassword>() || c.is::<sevenzip::BadPassword>()
            }
            #[cfg(not(windows))]
            {
                false
            }
        }
    })
}

/// 解压目标里已经有同名的文件，而请求是 [`Conflict::Refuse`]：什么都没改动，
/// 前端可以问用户怎么办，再按选择重新解压。
#[derive(Debug, Clone)]
pub struct SameNames {
    pub output: PathBuf,
    /// 已存在的那些文件，相对 `output`。
    pub files: Vec<PathBuf>,
}

impl std::fmt::Display for SameNames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}", t!("{} 里已有 {} 个同名文件（例如 {}）。什么都没改动；如要替换请选择替换", self.output.display(), self.files.len(), self.files.first().map(|p| self.output.join(p)).unwrap_or_default().display()))
    }
}

impl std::error::Error for SameNames {}

/// 失败是因为目标里已有同名文件（见 [`SameNames`]）时返回详情。
pub fn same_names(e: &anyhow::Error) -> Option<&SameNames> {
    e.chain().find_map(|c| c.downcast_ref::<SameNames>())
}

/// 字节数的可读形式，如 `1.5 GB`。
pub fn human(bytes: u64) -> String {
    hatchzip_core::human(bytes)
}

/// 解压到哪里。给了 [`ExtractRequest::output`] 时以它为准。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// 压缩包里只有一个顶层文件或文件夹时直接放在压缩包旁边，
    /// 否则新建一个以压缩包命名的文件夹。
    #[default]
    Smart,
    /// 直接解到压缩包所在的文件夹。
    Here,
    /// 总是新建一个以压缩包命名的文件夹。
    NewFolder,
}

/// 解压时目标位置已有同名文件怎么办。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conflict {
    /// 什么都不动，报错（让前端问用户）。
    #[default]
    Refuse,
    /// 用压缩包里的替换。
    Replace,
    /// 保留已有的，压缩包里的这个不解出。
    Skip,
    /// 都保留：压缩包里的改名为 `名字 (2).扩展名`。
    Rename,
}

impl Conflict {
    pub fn id(self) -> &'static str {
        match self {
            Conflict::Refuse => "refuse",
            Conflict::Replace => "replace",
            Conflict::Skip => "skip",
            Conflict::Rename => "rename",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Conflict::Refuse, Conflict::Replace, Conflict::Skip, Conflict::Rename]
            .into_iter()
            .find(|c| c.id() == s)
    }
}

/// `dir/name.ext` 已存在时找一个没人用的 `dir/name (2).ext`、`(3)`……
pub fn free_name(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let (base, ext) = match name.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => (b.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|i| path.with_file_name(format!("{base} ({i}){ext}")))
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .unwrap()
}

/// 一次解压请求。
#[derive(Debug, Clone, Default)]
pub struct ExtractRequest {
    pub source: Source,
    /// 输出目录；`None` 时按 [`placement`](Self::placement) 决定。
    /// 链接解压时这里是要新建的文件夹，`placement` 为 [`Placement::Smart`] 时，
    /// 解完里面只有一项就把它挪到外面一层。
    pub output: Option<PathBuf>,
    pub placement: Placement,
    /// 保留原压缩包（不边解边删）。只对本地文件有意义。
    pub keep_source: bool,
    pub password: Option<String>,
    /// 动手前先把整个压缩包校验一遍。对删了就找不回来的本地文件建议打开。
    pub verify_first: bool,
    /// 解压过程中磁盘至少保留的空闲字节数。
    pub keep_free: u64,
    /// 目标位置已有同名文件时怎么办（默认什么都不动、报错）。
    pub conflict: Conflict,
    /// 解出来只有一个压缩包时继续把它解开（中间那个包随后删除）。
    pub nested: bool,
    /// 边下载边解压同时用的连接数；0 用默认值（8）。
    pub connections: u32,
}

/// 进度与提示，发给前端显示。
#[derive(Debug, Clone)]
pub enum Event {
    /// 开始（或恢复）。`total` 为待处理的字节数；远程解压时为源文件大小。
    Started { total: u64, resuming: bool },
    /// 又处理了这么多字节（增量）：解压时是写出的，压缩时是读入的。
    Progressed(u64),
    /// 本地解压：原压缩包当前还占着的长度。
    ArchiveLeft(u64),
    /// 远程解压：累计已下载 / 已解出的字节数。
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
        Source::Url(url) => match route::url(url) {
            #[cfg(windows)]
            route::Url::SevenZip => sevenzip::extract_url(url, req, observer, stop),
            _ => remote::extract(url, req, observer, stop),
        },
    }
}

#[cfg(test)]
mod volume_tests {
    #[test]
    fn archive_volumes_lists_every_part() {
        let dir = tempfile::tempdir().unwrap();
        let touch = |n: &str| {
            let p = dir.path().join(n);
            std::fs::write(&p, b"x").unwrap();
            p
        };
        let plain = touch("a.zip");
        assert_eq!(super::archive_volumes(&plain), [plain]);
        let parts: Vec<_> = ["b.7z.001", "b.7z.002", "b.7z.003"].iter().map(|n| touch(n)).collect();
        assert_eq!(super::archive_volumes(&parts[1]), parts);
        let rar: Vec<_> = ["c.part1.rar", "c.part2.rar"].iter().map(|n| touch(n)).collect();
        assert_eq!(super::archive_volumes(&rar[0]), rar);
    }
}
