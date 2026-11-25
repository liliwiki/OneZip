//! onezip_shell.dll：Windows 11 资源管理器第一层右键菜单。
//!
//! 实现 `IExplorerCommand`：一个"OneZip"根菜单，下面是各个子命令。
//! 子命令只负责把选中的路径交给同目录下的 OneZip.exe，实际工作都在 OneZip.exe 里做，
//! 这样资源管理器（或承载本 DLL 的 dllhost）里不会跑任何重活，崩了也不影响资源管理器。
//!
//! 这个 DLL 需要通过稀疏包（packaging/sparse/AppxManifest.xml）注册，
//! 资源管理器才会把它放进第一层菜单。

/// 本扩展的 COM 类 ID，与 AppxManifest.xml 里的一致。
///
/// 只注册这一个第一层菜单项：同一个应用注册两项时，Win11 会把它们再套进一层
/// 没有图标的同名菜单里（OneZip ▸ OneZip ▸ …），反而更深。
pub const CLSID_TEXT: &str = "3847becd-6a0c-43a7-b898-e1f2c38be845";

/// 会出现"解压"类菜单项的扩展名（小写，不带点）。
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "tar", "tgz", "gz", "tbz2", "tbz", "bz2", "txz", "xz", "zst", "tzst",
    "lz4", "001",
];

/// 菜单里的一项。参照 7-Zip / NanaZip：解压一组、释放空间与测试一组、压缩一组，
/// 组之间用分隔线隔开；"解压到 / 添加到"直接写出目标名字。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Separator(Show),
    Command(Command),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// 选中的全是压缩包时才显示。
    Archives,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    SmartExtract,
    ExtractToFolder,
    ExtractHere,
    ExtractTo,
    ExtractAndDelete,
    Test,
    AddTo,
    Add7z,
    AddZip,
    /// 校验值，参数是 [`HASH_ITEMS`] 里的序号。
    ///
    /// Win11 新式右键菜单不显示应用菜单里的第三层（实测：资源管理器会取走各项，
    /// 但弹出的是空框），所以像 NanaZip 一样平铺在 OneZip 子菜单里。
    Hash(usize),
}

/// 校验值各项：（菜单文字，传给 `--hash` 的参数）。只列最常用的三种，
/// 其余（CRC-64、SHA-1、SHA-512）在"全部算法"里一起算。
pub const HASH_ITEMS: &[(&str, [&str; 2])] = &[
    ("CRC-32", ["--hash", "crc32"]),
    ("MD5", ["--hash", "md5"]),
    ("SHA-256", ["--hash", "sha256"]),
    ("校验值（全部算法）", ["--hash", "*"]),
];

pub const MENU: &[Entry] = &[
    Entry::Command(Command::SmartExtract),
    Entry::Command(Command::ExtractToFolder),
    Entry::Command(Command::ExtractHere),
    Entry::Command(Command::ExtractTo),
    Entry::Separator(Show::Archives),
    Entry::Command(Command::ExtractAndDelete),
    Entry::Command(Command::Test),
    Entry::Separator(Show::Archives),
    Entry::Command(Command::AddTo),
    Entry::Command(Command::Add7z),
    Entry::Command(Command::AddZip),
    Entry::Separator(Show::Always),
    Entry::Command(Command::Hash(0)),
    Entry::Command(Command::Hash(1)),
    Entry::Command(Command::Hash(2)),
    Entry::Command(Command::Hash(3)),
];

impl Entry {
    pub fn show(self) -> Show {
        match self {
            Entry::Separator(s) => s,
            Entry::Command(c) => c.show(),
        }
    }
}

impl Command {
    pub fn show(self) -> Show {
        match self {
            Command::AddTo | Command::Add7z | Command::AddZip | Command::Hash(_) => Show::Always,
            _ => Show::Archives,
        }
    }

    /// 传给 OneZip.exe 的参数。
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Command::SmartExtract => &["--extract-smart"],
            Command::ExtractToFolder => &["--extract-folder"],
            Command::ExtractHere => &["--extract-here"],
            Command::ExtractTo => &["--extract-to"],
            Command::ExtractAndDelete => &["--reclaim", "--extract-smart"],
            Command::Test => &["--test"],
            Command::AddTo => &["--compress"],
            Command::Add7z => &["--compress-7z"],
            Command::AddZip => &["--compress-zip"],
            Command::Hash(i) => &HASH_ITEMS[i].1,
        }
    }

    /// 菜单文字。`paths` 是选中的项，`is_dir` 判断某个路径是否文件夹。
    pub fn title(self, paths: &[String], is_dir: impl Fn(&str) -> bool) -> String {
        match self {
            Command::SmartExtract => "智能解压".into(),
            Command::ExtractToFolder => match paths {
                [one] => format!("解压到 \"{}\\\"", archive_stem(file_name(one))),
                _ => "解压到各自的同名文件夹".into(),
            },
            Command::ExtractHere => "解压到当前位置".into(),
            Command::ExtractTo => "解压文件…".into(),
            Command::ExtractAndDelete => "解压后删除压缩包（边解压边释放空间）".into(),
            Command::Test => "测试压缩包".into(),
            Command::AddTo => "添加到压缩包…".into(),
            Command::Add7z => format!("添加到 \"{}.7z\"", compress_stem(paths, is_dir)),
            Command::AddZip => format!("添加到 \"{}.zip\"", compress_stem(paths, is_dir)),
            Command::Hash(i) => HASH_ITEMS[i].0.into(),
        }
    }
}

/// 是否显示某一项：解压类只在选中的全是压缩包时出现。
pub fn visible(entry: Entry, paths: &[String]) -> bool {
    !paths.is_empty() && (entry.show() == Show::Always || paths.iter().all(|p| is_archive(p)))
}

fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed)
}

fn parent_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rsplit_once(['\\', '/']) {
        Some((parent, _)) => file_name(parent),
        None => "",
    }
}

/// 压缩包名去掉压缩扩展名，用作解压出的文件夹名（与 OneZip.exe 一致）。
pub fn archive_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for ext in [
        ".tar.zst", ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.lz4", ".7z.001", ".zip.001", ".tgz",
        ".txz", ".tbz2", ".tbz", ".tzst", ".tar", ".zip", ".7z", ".rar", ".zst", ".gz", ".xz",
        ".bz2", ".lz4", ".001",
    ] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    format!("{name}-extracted")
}

/// 压缩包默认名（不含扩展名），与 onezip_core::compress::default_output 一致：
/// 一个文件取主名，一个文件夹取文件夹名，多项取所在文件夹名。
pub fn compress_stem(paths: &[String], is_dir: impl Fn(&str) -> bool) -> String {
    let name = match paths {
        [one] if is_dir(one) => file_name(one).to_string(),
        [one] => {
            let n = file_name(one);
            match n.rsplit_once('.') {
                Some((stem, _)) if !stem.is_empty() => stem.to_string(),
                _ => n.to_string(),
            }
        }
        [first, ..] => parent_name(first).to_string(),
        [] => String::new(),
    };
    // 盘符根目录（如 D:）没有文件夹名。
    if name.is_empty() || name.ends_with(':') {
        "压缩包".into()
    } else {
        name
    }
}

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

    fn not_dir(_: &str) -> bool {
        false
    }

    #[test]
    fn titles_name_the_target() {
        let one = vec![r"D:\下载\资料.tar.gz".to_string()];
        assert_eq!(Command::ExtractToFolder.title(&one, not_dir), "解压到 \"资料\\\"");
        assert_eq!(Command::Add7z.title(&one, not_dir), "添加到 \"资料.tar.7z\"");
        let file = vec![r"D:\下载\报告.docx".to_string()];
        assert_eq!(Command::AddZip.title(&file, not_dir), "添加到 \"报告.zip\"");
        let dir = vec![r"D:\下载\照片 2024".to_string()];
        assert_eq!(Command::Add7z.title(&dir, |_| true), "添加到 \"照片 2024.7z\"");
        let many = vec![r"D:\下载\a.txt".to_string(), r"D:\下载\b.txt".to_string()];
        assert_eq!(Command::Add7z.title(&many, not_dir), "添加到 \"下载.7z\"");
        assert_eq!(Command::ExtractToFolder.title(&many, not_dir), "解压到各自的同名文件夹");
        let root = vec![r"D:\a.txt".to_string(), r"D:\b.txt".to_string()];
        assert_eq!(Command::Add7z.title(&root, not_dir), "添加到 \"压缩包.7z\"");
    }

    #[test]
    fn extraction_only_on_archives() {
        let zip = vec![r"C:\a.zip".to_string()];
        let mixed = vec![r"C:\a.zip".to_string(), r"C:\b.txt".to_string()];
        let extract = Entry::Command(Command::SmartExtract);
        let add = Entry::Command(Command::Add7z);
        assert!(visible(extract, &zip));
        assert!(!visible(extract, &mixed));
        assert!(!visible(Entry::Separator(Show::Archives), &mixed));
        assert!(visible(add, &mixed));
        assert!(!visible(add, &[]));
    }
}
