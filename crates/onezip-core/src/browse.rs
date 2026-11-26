//! 浏览压缩包（像 7-Zip 的文件管理器）：列出全部条目，只解出选中的那些。都交给 7z.dll。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use anyhow::Result;

use crate::{Conflict, Observer};

/// 压缩包里的一项。
#[derive(Debug, Clone)]
pub struct Item {
    /// 用 `/` 分隔的相对路径（已经清理过不安全的名字）。
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// 解出压缩包里选中的项。
#[derive(Debug, Clone)]
pub struct PickRequest {
    pub archive: PathBuf,
    pub password: Option<String>,
    /// 选中的项（[`Item::path`]）；选中文件夹就是它里面的全部。
    pub picks: Vec<String>,
    /// 浏览到的文件夹（`/` 分隔，根目录为空）：解出时去掉这一段，
    /// 选中的项直接放在 `output` 下面。
    pub base: String,
    pub output: PathBuf,
    /// [`Conflict::Refuse`] 时有同名文件就报 [`crate::SameNames`]，什么都不动。
    pub conflict: Conflict,
}

/// 选中的项都在这个文件夹里（解出后放在这里）。
fn relative<'a>(path: &'a str, base: &str) -> Option<&'a str> {
    if base.is_empty() {
        return Some(path);
    }
    path.strip_prefix(base)?.strip_prefix('/')
}

/// `path` 是不是选中的某一项，或在选中的文件夹里面。
fn picked(path: &str, picks: &[String]) -> bool {
    picks
        .iter()
        .any(|p| path == p || path.strip_prefix(p.as_str()).is_some_and(|rest| rest.starts_with('/')))
}

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::fs;

    use anyhow::{anyhow, bail};

    use super::*;
    use crate::Event;
    use crate::sevenzip::archive::{self, Sink, Stopped, Target};

    pub fn items(archive_path: &Path, password: Option<String>) -> Result<Vec<Item>> {
        let arc = archive::open(archive_path, password)?;
        Ok(arc
            .items
            .iter()
            .filter(|i| !i.skip)
            .map(|i| Item {
                path: i.path.clone(),
                is_dir: i.is_dir,
                size: i.size,
                modified: i.mtime.map(archive::filetime_to_system),
            })
            .collect())
    }

    pub fn extract(req: &PickRequest, observer: Observer, stop: &Arc<AtomicBool>) -> Result<PathBuf> {
        let arc = archive::open(&req.archive, req.password.clone())?;
        let mut files: Vec<(u32, PathBuf, u64)> = Vec::new();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for i in arc.items.iter().filter(|i| !i.skip && picked(&i.path, &req.picks)) {
            let Some(rel) = relative(&i.path, &req.base) else { continue };
            let dest = req.output.join(rel.replace('/', "\\"));
            if i.is_dir {
                dirs.push(dest);
            } else {
                files.push((i.index, dest, i.size));
            }
        }
        if files.is_empty() && dirs.is_empty() {
            bail!("没有选中要解压的项");
        }

        // 同名文件：先全部查出来再决定，什么都没动之前就能停下。
        let clash: Vec<usize> =
            (0..files.len()).filter(|&k| fs::symlink_metadata(&files[k].1).is_ok()).collect();
        match req.conflict {
            Conflict::Refuse if !clash.is_empty() => {
                return Err(anyhow!(crate::SameNames {
                    output: req.output.clone(),
                    files: clash
                        .iter()
                        .map(|&k| files[k].1.strip_prefix(&req.output).unwrap_or(&files[k].1).to_path_buf())
                        .collect(),
                }));
            }
            Conflict::Skip => {
                let mut k = 0;
                files.retain(|_| {
                    k += 1;
                    !clash.contains(&(k - 1))
                });
            }
            Conflict::Rename => {
                for &k in &clash {
                    files[k].1 = crate::free_name(&files[k].1);
                }
            }
            Conflict::Refuse | Conflict::Replace => {}
        }

        fs::create_dir_all(&req.output)?;
        for d in &dirs {
            fs::create_dir_all(d)?;
        }
        observer(Event::Started { total: files.iter().map(|f| f.2).sum(), resuming: false });
        if files.is_empty() {
            return Ok(req.output.clone());
        }

        struct S {
            dest: HashMap<u32, PathBuf>,
            observer: Observer,
        }
        impl Sink for S {
            fn target(&mut self, index: u32) -> Option<Target> {
                self.dest.get(&index).cloned().map(Target::File)
            }
            fn progress(&mut self, bytes: u64) {
                (self.observer)(Event::Progressed(bytes));
            }
            fn done(&mut self, _: u32, _: Option<&Path>) {}
        }
        let indices: Vec<u32> = files.iter().map(|f| f.0).collect();
        let sink = S { dest: files.into_iter().map(|(i, p, _)| (i, p)).collect(), observer };
        match arc.extract(&indices, false, req.password.clone(), stop, Box::new(sink)) {
            Err(e) if e.is::<Stopped>() => Err(anyhow!(crate::Cancelled)),
            r => r.map(|()| req.output.clone()),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn items(_archive: &Path, _password: Option<String>) -> Result<Vec<Item>> {
        anyhow::bail!("浏览压缩包只支持 Windows")
    }

    pub fn extract(_req: &PickRequest, _observer: Observer, _stop: &Arc<AtomicBool>) -> Result<PathBuf> {
        anyhow::bail!("浏览压缩包只支持 Windows")
    }
}

/// 列出压缩包里的全部项（包括文件夹）。文件头加密的要密码（见 [`crate::needs_password`]）。
pub fn items(archive: &Path, password: Option<String>) -> Result<Vec<Item>> {
    imp::items(archive, password)
}

/// 解出选中的项，返回放到的文件夹。被 `stop` 叫停时返回 [`crate::Cancelled`]，
/// 写了一半的那个文件会删掉，已经写完的保留。
pub fn extract(req: &PickRequest, observer: Observer, stop: &Arc<AtomicBool>) -> Result<PathBuf> {
    imp::extract(req, observer, stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_and_relative_paths() {
        let picks = vec!["a/b".to_string(), "c.txt".to_string()];
        assert!(picked("a/b", &picks));
        assert!(picked("a/b/x.txt", &picks));
        assert!(!picked("a/bc/x.txt", &picks));
        assert!(picked("c.txt", &picks));
        assert!(!picked("a/c.txt", &picks));
        assert_eq!(relative("a/b/x.txt", "a"), Some("b/x.txt"));
        assert_eq!(relative("a/b/x.txt", ""), Some("a/b/x.txt"));
        assert_eq!(relative("ab/x.txt", "a"), None);
    }
}
