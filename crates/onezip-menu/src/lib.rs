//! OneZip 右键菜单的内容：有哪些项、顺序、分隔线、文字、传给 OneZip.exe 的参数。
//!
//! Windows 11 第一层菜单（onezip-shell）和传统注册表菜单（onezip-app 的 menu 模块）
//! 都从这里生成，两边不会对不上。参照 7-Zip / NanaZip：解压一组、删除与测试一组、
//! 压缩一组，组之间用分隔线隔开；"解压到 / 添加到"直接写出目标名字。

use onezip_formats::{archive_stem, compress_stem, file_name, in_place, is_archive};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Separator(Show),
    Command(Command),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// 选中的全是压缩包时才显示。
    Archives,
    /// 选中的全是能边解边删的压缩包时才显示。
    InPlace,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    SmartExtract,
    ExtractToFolder,
    ExtractHere,
    ExtractTo,
    /// 先正常解压，全部成功后再删除压缩包（回收站或直接删除，看设置）。
    ExtractThenDelete,
    /// 边解压边删除压缩包，释放空间（只有部分格式能这样做）。
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
    Entry::Command(Command::ExtractThenDelete),
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
            Command::ExtractAndDelete => Show::InPlace,
            _ => Show::Archives,
        }
    }

    /// 稳定的名字：设置里记"隐藏了哪些项"、注册表菜单的键名都用它。
    pub fn id(self) -> &'static str {
        match self {
            Command::SmartExtract => "smart",
            Command::ExtractToFolder => "folder",
            Command::ExtractHere => "here",
            Command::ExtractTo => "to",
            Command::ExtractThenDelete => "deleteafter",
            Command::ExtractAndDelete => "inplace",
            Command::Test => "test",
            Command::AddTo => "add",
            Command::Add7z => "add7z",
            Command::AddZip => "addzip",
            Command::Hash(0) => "crc32",
            Command::Hash(1) => "md5",
            Command::Hash(2) => "sha256",
            Command::Hash(_) => "hashall",
        }
    }

    /// 传给 OneZip.exe 的参数。
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Command::SmartExtract => &["--extract-smart"],
            Command::ExtractToFolder => &["--extract-folder"],
            Command::ExtractHere => &["--extract-here"],
            Command::ExtractTo => &["--extract-to"],
            Command::ExtractThenDelete => &["--delete-after", "--extract-smart"],
            Command::ExtractAndDelete => &["--reclaim", "--extract-smart"],
            Command::Test => &["--test"],
            Command::AddTo => &["--compress"],
            Command::Add7z => &["--compress-7z"],
            Command::AddZip => &["--compress-zip"],
            Command::Hash(i) => &HASH_ITEMS[i].1,
        }
    }

    /// 不知道选中了什么时的文字（注册表菜单、设置页用）。
    pub fn label(self) -> &'static str {
        match self {
            Command::SmartExtract => "智能解压",
            Command::ExtractToFolder => "解压到同名文件夹",
            Command::ExtractHere => "解压到当前位置",
            Command::ExtractTo => "解压文件…",
            Command::ExtractThenDelete => "解压后删除压缩包",
            Command::ExtractAndDelete => "边解压边删除压缩包",
            Command::Test => "测试压缩包",
            Command::AddTo => "添加到压缩包…",
            Command::Add7z => "添加到 7z 压缩包",
            Command::AddZip => "添加到 zip 压缩包",
            Command::Hash(i) => HASH_ITEMS[i].0,
        }
    }

    /// 菜单文字，写出具体名字。`paths` 是选中的项，`is_dir` 判断某个路径是否文件夹。
    pub fn title(self, paths: &[String], is_dir: impl Fn(&str) -> bool) -> String {
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        match self {
            Command::ExtractToFolder => match paths {
                [one] => format!("解压到 \"{}\\\"", archive_stem(file_name(one))),
                _ => "解压到各自的同名文件夹".into(),
            },
            Command::Add7z => format!("添加到 \"{}.7z\"", compress_stem(&refs, is_dir)),
            Command::AddZip => format!("添加到 \"{}.zip\"", compress_stem(&refs, is_dir)),
            c => c.label().into(),
        }
    }

    /// 所有命令，按菜单顺序。
    pub fn all() -> impl Iterator<Item = Command> {
        MENU.iter().filter_map(|e| match e {
            Entry::Command(c) => Some(*c),
            Entry::Separator(_) => None,
        })
    }
}

/// 是否显示某一项：解压类只在选中的全是压缩包时出现。
/// 去掉用户在设置里隐藏的项（`hidden` 是命令 id），并去掉因此落在开头、结尾或连在一起的分隔线。
pub fn without_hidden(hidden: &[String]) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for &entry in MENU {
        match entry {
            Entry::Command(c) if hidden.iter().any(|h| h == c.id()) => {}
            Entry::Separator(_) if !matches!(out.last(), Some(Entry::Command(_))) => {}
            _ => out.push(entry),
        }
    }
    if matches!(out.last(), Some(Entry::Separator(_))) {
        out.pop();
    }
    out
}

pub fn visible(entry: Entry, paths: &[String]) -> bool {
    !paths.is_empty()
        && match entry.show() {
            Show::Always => true,
            Show::Archives => paths.iter().all(|p| is_archive(p)),
            Show::InPlace => paths.iter().all(|p| is_archive(p) && in_place(p)),
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn not_dir(_: &str) -> bool {
        false
    }

    #[test]
    fn titles_name_the_target() {
        let one = vec![r"D:\下载\资料.tar.gz".to_string()];
        assert_eq!(Command::ExtractToFolder.title(&one, not_dir), "解压到 \"资料\\\"");
        assert_eq!(Command::Add7z.title(&one, not_dir), "添加到 \"资料.tar.7z\"");
        let iso = vec![r"D:\下载\系统.iso".to_string()];
        assert_eq!(Command::ExtractToFolder.title(&iso, not_dir), "解压到 \"系统\\\"");
        let file = vec![r"D:\下载\报告.docx".to_string()];
        assert_eq!(Command::AddZip.title(&file, not_dir), "添加到 \"报告.zip\"");
        let dir = vec![r"D:\下载\照片 2024".to_string()];
        assert_eq!(Command::Add7z.title(&dir, |_| true), "添加到 \"照片 2024.7z\"");
        let many = vec![r"D:\下载\a.txt".to_string(), r"D:\下载\b.txt".to_string()];
        assert_eq!(Command::Add7z.title(&many, not_dir), "添加到 \"下载.7z\"");
        assert_eq!(Command::ExtractToFolder.title(&many, not_dir), "解压到各自的同名文件夹");
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
        let delete = Entry::Command(Command::ExtractAndDelete);
        assert!(visible(delete, &[r"C:\a.iso".to_string()]));
        assert!(visible(delete, &[r"C:\a.cab".to_string()]));
        assert!(!visible(delete, &[r"C:\a.dmg".to_string()]));
        assert!(visible(extract, &[r"C:\a.dmg".to_string()]));
        // 解压后再删：任何能解压的都行
        let after = Entry::Command(Command::ExtractThenDelete);
        assert!(visible(after, &[r"C:\a.dmg".to_string()]));
        assert!(!visible(after, &mixed));
    }

    #[test]
    fn ids_are_unique() {
        let ids: Vec<&str> = Command::all().map(Command::id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn hidden_items_and_their_separators_go_away() {
        assert_eq!(without_hidden(&[]), MENU.to_vec());
        let hidden: Vec<String> = ["inplace", "test", "crc32", "md5", "sha256", "hashall"].map(String::from).to_vec();
        let menu = without_hidden(&hidden);
        assert!(menu.iter().all(|e| match e {
            Entry::Command(c) => !hidden.iter().any(|h| h == c.id()),
            Entry::Separator(_) => true,
        }));
        assert!(matches!(menu.last(), Some(Entry::Command(Command::AddZip))));
        assert!(menu.windows(2).all(|w| !matches!(w, [Entry::Separator(_), Entry::Separator(_)])));
    }
}
