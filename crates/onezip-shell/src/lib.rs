//! onezip_shell.dll：Windows 11 资源管理器第一层右键菜单。
//!
//! 实现 `IExplorerCommand`：一个"OneZip"根菜单，下面是各个子命令。
//! 子命令只负责把选中的路径交给同目录下的 OneZip.exe，实际工作都在 OneZip.exe 里做，
//! 这样资源管理器（或承载本 DLL 的 dllhost）里不会跑任何重活，崩了也不影响资源管理器。
//!
//! 这个 DLL 需要通过稀疏包（packaging/sparse/AppxManifest.xml）注册，
//! 资源管理器才会把它放进第一层菜单。

/// 本扩展的 COM 类 ID，与 AppxManifest.xml 里的一致。
pub const CLSID_TEXT: &str = "3847becd-6a0c-43a7-b898-e1f2c38be845";

/// 会出现"解压"类菜单项的扩展名（小写，不带点）。
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "tar", "tgz", "gz", "tbz2", "tbz", "bz2", "txz", "xz", "zst", "tzst",
    "lz4", "001",
];

pub struct Item {
    pub label: &'static str,
    pub args: &'static [&'static str],
    /// 只在选中的全是压缩包时显示。
    pub archives_only: bool,
}

pub const ITEMS: &[Item] = &[
    Item {
        label: "智能解压",
        args: &["--extract-smart"],
        archives_only: true,
    },
    Item {
        label: "智能解压并释放空间（边解边删原包）",
        args: &["--reclaim", "--extract-smart"],
        archives_only: true,
    },
    Item {
        label: "解压到当前文件夹",
        args: &["--extract-here"],
        archives_only: true,
    },
    Item {
        label: "解压到同名文件夹",
        args: &["--extract-folder"],
        archives_only: true,
    },
    Item {
        label: "解压到…",
        args: &["--extract-to"],
        archives_only: true,
    },
    Item {
        label: "测试压缩包",
        args: &["--test"],
        archives_only: true,
    },
    Item {
        label: "添加到 7z 压缩包",
        args: &["--compress-7z"],
        archives_only: false,
    },
    Item {
        label: "添加到 zip 压缩包",
        args: &["--compress-zip"],
        archives_only: false,
    },
    Item {
        label: "添加到压缩包…",
        args: &["--compress"],
        archives_only: false,
    },
];

/// 是否像压缩包（按扩展名）。
pub fn is_archive(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let Some((_, ext)) = lower.rsplit_once('.') else {
        return false;
    };
    ARCHIVE_EXTENSIONS.contains(&ext) || ext.len() == 3 && ext.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(windows)]
mod com;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives() {
        assert!(is_archive(r"C:\a\B.ZIP"));
        assert!(is_archive(r"C:\a\b.tar.zst"));
        assert!(is_archive(r"C:\a\b.7z.001"));
        assert!(!is_archive(r"C:\a\b.txt"));
        assert!(!is_archive(r"C:\a\dir"));
    }
}
