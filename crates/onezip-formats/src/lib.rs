//! OneZip 对压缩包文件名的全部认识，引擎、主程序、右键菜单 DLL 共用这一份：
//!
//! - 哪些扩展名算压缩包（右键菜单、打开对话框的过滤器）；
//! - 由哪个引擎打开（[`Engine`]）；
//! - 能不能边解压边删除（[`in_place`]）；
//! - 解出的文件夹叫什么（[`archive_stem`]）、压缩包默认叫什么（[`compress_stem`]）。
//!
//! 只看名字，不读文件，也不依赖任何别的模块，所以 DLL 里用也很轻。

/// 由 hatchzip-core / Peel 处理的扩展名（小写，不带点）。`001` 是分卷的第一卷。
pub const NATIVE_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "r00", "tar", "tgz", "gz", "tbz2", "tbz", "bz2", "txz", "xz", "zst", "tzst",
    "lz4", "001",
];

/// 由 7z.dll 处理的扩展名。
pub const SEVENZIP_EXTENSIONS: &[&str] = &[
    "iso", "img", "udf", "wim", "swm", "esd", "cab", "dmg", "vhd", "vhdx", "vmdk", "vdi", "qcow",
    "qcow2", "msi", "msp", "msu", "arj", "lzh", "lha", "chm", "chi", "chw", "cpio", "rpm", "deb",
    "xar", "pkg", "squashfs", "cramfs", "hfs", "hfsx", "apfs", "z", "taz", "lzma",
];

/// 7z.dll 的格式里，结构上允许边解边删的。
pub const SEVENZIP_IN_PLACE: &[&str] = &["iso", "udf", "wim", "esd", "cab", "arj", "lzh", "lha"];

/// 本地文件也交给 Peel 的扩展名（单流 zstd / lz4）。
pub const PEEL_LOCAL: &[&str] = &["zst", "tzst", "lz4"];

/// 解出来恰好是一个这种文件时，"解开套着的压缩包"会继续解它。
const NESTED: &[&str] = &[
    ".zip", ".7z", ".tar", ".tgz", ".tar.gz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar.zst", ".tzst",
];

/// 所有算作压缩包的扩展名。
pub fn archive_extensions() -> impl Iterator<Item = &'static str> {
    NATIVE_EXTENSIONS.iter().chain(SEVENZIP_EXTENSIONS).copied()
}

/// 由哪个引擎打开一个本地压缩包（只看名字）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// hatchzip-core：zip、7z、rar、tar 系列。
    HatchZip,
    /// Peel：单流的 zst / lz4。
    Peel,
    /// 7z.dll：iso、cab、wim 等。
    SevenZip,
}

/// 名字的最后一个扩展名，小写、不带点。
pub fn ext(name: &str) -> String {
    let name = file_name(name);
    match name.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    }
}

pub fn engine(name: &str) -> Engine {
    let e = ext(name);
    if SEVENZIP_EXTENSIONS.contains(&e.as_str()) {
        Engine::SevenZip
    } else if PEEL_LOCAL.contains(&e.as_str()) {
        Engine::Peel
    } else {
        Engine::HatchZip
    }
}

/// 是否像压缩包。`.002` 这类纯数字扩展名也算（分卷）。
pub fn is_archive(name: &str) -> bool {
    let e = ext(name);
    archive_extensions().any(|a| a == e) || e.len() == 3 && e.bytes().all(|b| b.is_ascii_digit())
}

/// 能不能边解压边删除。7z.dll 的格式只有一部分可以；其余格式都可以。
pub fn in_place(name: &str) -> bool {
    match engine(name) {
        Engine::SevenZip => SEVENZIP_IN_PLACE.contains(&ext(name).as_str()),
        _ => true,
    }
}

/// 解出来只有这一个文件时，要不要接着解开它。
pub fn is_nested_candidate(name: &str) -> bool {
    let lower = file_name(name).to_ascii_lowercase();
    NESTED.iter().any(|e| lower.ends_with(e))
}

/// 路径的最后一段。`\` 和 `/` 都算分隔符。
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed)
}

/// 压缩包名去掉压缩扩展名，用作解压出的文件夹名：
/// `资料.tar.gz` → `资料`，`game.part1.rar` → `game`，`a.7z.001` → `a`，`x.iso` → `x`。
/// 认不出扩展名时加 `-extracted`，免得文件夹和压缩包同名。
pub fn archive_stem(name: &str) -> String {
    let name = file_name(name);
    let lower = name.to_ascii_lowercase();
    // game.part1.rar / game.part01.rar
    if let Some(stem) = lower.strip_suffix(".rar")
        && let Some((base, digits)) = stem.rsplit_once(".part")
        && !base.is_empty()
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
    {
        return name[..base.len()].to_string();
    }
    for e in [
        ".tar.zst", ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.lz4", ".7z.001", ".zip.001", ".tgz", ".txz",
        ".tbz2", ".tbz", ".tzst", ".tar",
    ] {
        if lower.ends_with(e) && name.len() > e.len() {
            return name[..name.len() - e.len()].to_string();
        }
    }
    if let Some((base, _)) = name.rsplit_once('.')
        && !base.is_empty()
        && is_archive(name)
    {
        return base.to_string();
    }
    format!("{name}-extracted")
}

/// 压缩包默认名（不含扩展名）：一个文件取主名，一个文件夹取文件夹名，多项取所在文件夹名。
pub fn compress_stem(paths: &[&str], is_dir: impl Fn(&str) -> bool) -> String {
    let name = match paths {
        [one] if is_dir(one) => file_name(one).to_string(),
        [one] => {
            let n = file_name(one);
            match n.rsplit_once('.') {
                Some((stem, _)) if !stem.is_empty() => stem.to_string(),
                _ => n.to_string(),
            }
        }
        [first, ..] => {
            let trimmed = first.trim_end_matches(['\\', '/']);
            match trimmed.rsplit_once(['\\', '/']) {
                Some((parent, _)) => file_name(parent).to_string(),
                None => String::new(),
            }
        }
        [] => String::new(),
    };
    // 盘符根目录（如 D:）没有文件夹名。
    if name.is_empty() || name.ends_with(':') {
        "压缩包".into()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_and_engines() {
        assert!(is_archive(r"C:\a\B.ZIP"));
        assert!(is_archive(r"C:\a\b.tar.zst"));
        assert!(is_archive(r"C:\a\b.7z.001"));
        assert!(is_archive("x.iso"));
        assert!(!is_archive(r"C:\a\b.txt"));
        assert!(!is_archive(r"C:\a\dir"));
        assert_eq!(engine("a.ISO"), Engine::SevenZip);
        assert_eq!(engine("a.tar.zst"), Engine::Peel);
        assert_eq!(engine("a.part1.rar"), Engine::HatchZip);
        assert!(in_place("a.zip") && in_place("a.cab") && !in_place("a.dmg"));
    }

    #[test]
    fn stems() {
        assert_eq!(archive_stem(r"D:\下载\资料.tar.gz"), "资料");
        assert_eq!(archive_stem("game.part1.rar"), "game");
        assert_eq!(archive_stem("game.part02.RAR"), "game");
        assert_eq!(archive_stem("a.7z.001"), "a");
        assert_eq!(archive_stem("disk.iso"), "disk");
        assert_eq!(archive_stem("x.zip"), "x");
        assert_eq!(archive_stem("notes.txt"), "notes.txt-extracted");
        assert_eq!(compress_stem(&[r"D:\下载\报告.docx"], |_| false), "报告");
        assert_eq!(compress_stem(&[r"D:\下载\照片 2024"], |_| true), "照片 2024");
        assert_eq!(compress_stem(&[r"D:\下载\a.txt", r"D:\下载\b.txt"], |_| false), "下载");
        assert_eq!(compress_stem(&[r"D:\a.txt", r"D:\b.txt"], |_| false), "压缩包");
    }

    #[test]
    fn nested() {
        assert!(is_nested_candidate("inner.tar"));
        assert!(is_nested_candidate("a/b/inner.TAR.GZ"));
        assert!(!is_nested_candidate("disk.iso"));
    }
}
