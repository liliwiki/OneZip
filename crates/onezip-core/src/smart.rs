//! 智能解压的后半步，给事先不知道压缩包里有什么的解压用（边下载边解压、
//! Peel 的边解边删）：先解到以压缩包命名的新文件夹里，完成后如果里面只有一项，
//! 就把它挪到外面一层，免得多套一层文件夹。

use std::fs;
use std::path::{Path, PathBuf};

/// 文件夹不存在或是空的。
pub(crate) fn is_empty_dir(dir: &Path) -> bool {
    fs::read_dir(dir).map_or(true, |mut d| d.next().is_none())
}

/// `dir` 里只有一项时把它挪到 `dir` 旁边并删掉 `dir`，返回它的新位置；
/// 不止一项、外面已有同名的、或挪不动时原样不动，返回 `dir`。
pub(crate) fn flatten(dir: &Path) -> PathBuf {
    try_flatten(dir).unwrap_or_else(|| dir.to_path_buf())
}

fn try_flatten(dir: &Path) -> Option<PathBuf> {
    let parent = dir.parent()?;
    let mut entries = fs::read_dir(dir).ok()?;
    let only = entries.next()?.ok()?;
    if entries.next().is_some() {
        return None;
    }
    let name = only.file_name();
    let target = parent.join(&name);
    let same = name.to_string_lossy().to_lowercase() == dir.file_name()?.to_string_lossy().to_lowercase();
    if same {
        // 里面那一项和文件夹同名：先把文件夹改个名，再把那一项挪出来。
        let mut aside = dir.file_name()?.to_os_string();
        aside.push(".onezip-flat");
        let aside = parent.join(aside);
        if aside.exists() || fs::rename(dir, &aside).is_err() {
            return None;
        }
        if fs::rename(aside.join(&name), &target).is_err() {
            let _ = fs::rename(&aside, dir);
            return None;
        }
        let _ = fs::remove_dir(&aside);
        return Some(target);
    }
    if fs::symlink_metadata(&target).is_ok() || fs::rename(only.path(), &target).is_err() {
        return None;
    }
    let _ = fs::remove_dir(dir);
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_folder_moves_up() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("pack");
        fs::create_dir_all(dir.join("资料/子")).unwrap();
        fs::write(dir.join("资料/子/a.txt"), b"a").unwrap();
        assert_eq!(flatten(&dir), t.path().join("资料"));
        assert!(t.path().join("资料/子/a.txt").exists());
        assert!(!dir.exists());
    }

    #[test]
    fn same_name_inside() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("pack");
        fs::create_dir_all(dir.join("Pack")).unwrap();
        fs::write(dir.join("Pack/a.txt"), b"a").unwrap();
        assert_eq!(flatten(&dir), t.path().join("Pack"));
        assert!(t.path().join("Pack/a.txt").exists());
        assert!(!t.path().join("pack.onezip-flat").exists());
    }

    #[test]
    fn single_file_moves_up() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("pack");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), b"a").unwrap();
        assert_eq!(flatten(&dir), t.path().join("a.txt"));
        assert!(!dir.exists());
    }

    #[test]
    fn many_items_or_taken_name_stay() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("pack");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), b"a").unwrap();
        fs::write(dir.join("b.txt"), b"b").unwrap();
        assert_eq!(flatten(&dir), dir);

        let dir2 = t.path().join("pack2");
        fs::create_dir_all(&dir2).unwrap();
        fs::write(dir2.join("taken.txt"), b"new").unwrap();
        fs::write(t.path().join("taken.txt"), b"old").unwrap();
        assert_eq!(flatten(&dir2), dir2);
        assert_eq!(fs::read(t.path().join("taken.txt")).unwrap(), b"old");
    }
}
