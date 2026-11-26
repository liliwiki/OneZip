//! 选引擎：一个解压请求交给谁，全在这里决定，别处不再按扩展名判断。
//!
//! | 来源 | 条件 | 引擎 |
//! |---|---|---|
//! | 本地 | .zst / .tzst / .lz4 | Peel |
//! | 本地 | iso、cab、wim 等 7z.dll 格式 | 7z.dll |
//! | 本地 | zip / 7z / rar，保留原包 | 7z.dll（更快）；上次由 hatchzip-core 开始的仍交给它 |
//! | 本地 | 其余（含边解边删的 zip / 7z / rar、tar 系列、.7z.001 分卷） | hatchzip-core |
//! | 链接 | 7z.dll 格式 | 下载镜像 + 7z.dll |
//! | 链接 | 其余 | Peel |

use std::path::{Path, PathBuf};

use onezip_formats::Engine;

use crate::ExtractRequest;

/// 本地压缩包交给谁。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Local {
    Peel,
    HatchZip,
    /// 7z.dll，打开这一卷（分卷时是第一卷）。
    SevenZip(PathBuf),
}

/// 链接交给谁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Url {
    Peel,
    SevenZip,
}

fn name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub(crate) fn local(archive: &Path, req: &ExtractRequest) -> Local {
    match onezip_formats::engine(&name(archive)) {
        Engine::Peel => Local::Peel,
        Engine::SevenZip if cfg!(windows) => Local::SevenZip(archive.to_path_buf()),
        _ => match keep_with_7z(archive, req) {
            Some(primary) => Local::SevenZip(primary),
            None => Local::HatchZip,
        },
    }
}

pub(crate) fn url(url: &str) -> Url {
    match crate::source::url_file_name(url) {
        Some(n) if cfg!(windows) && onezip_formats::engine(&n) == Engine::SevenZip => Url::SevenZip,
        _ => Url::Peel,
    }
}

/// 保留原包解 zip / 7z / rar 时用 7z.dll：返回要交给它的第一卷。
/// 分卷 7z（.7z.001）和上次由 hatchzip-core 开始、还没做完的解压仍交给 hatchzip-core。
/// 设了环境变量 `ONEZIP_NO_7Z_KEEP` 时不用（对比测试用）。
fn keep_with_7z(archive: &Path, req: &ExtractRequest) -> Option<PathBuf> {
    if !cfg!(windows) || !req.keep_source || std::env::var_os("ONEZIP_NO_7Z_KEEP").is_some() {
        return None;
    }
    let (primary, paths) = hatchzip_core::session::locate(archive).ok()?;
    if paths.exists() {
        return None;
    }
    matches!(onezip_formats::ext(&name(&primary)).as_str(), "zip" | "7z" | "rar").then_some(primary)
}

/// 7z.dll 说 zip 密码不对时，要不要换 hatchzip-core 再试：7-Zip 按系统代码页处理
/// zip 的非 ASCII 密码，而 OneZip 等按 UTF-8 加密。
pub(crate) fn retry_zip_password_with_hatchzip(primary: &Path, req: &ExtractRequest) -> bool {
    onezip_formats::ext(&name(primary)) == "zip" && req.password.as_deref().is_some_and(|p| !p.is_ascii())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(url("https://h.example/a/disk.iso?x=1"), if cfg!(windows) { Url::SevenZip } else { Url::Peel });
        assert_eq!(url("https://h.example/a/data.tar.zst"), Url::Peel);
    }
}
