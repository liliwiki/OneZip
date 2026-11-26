//! 界面语言：简体中文（源文本）和英文。
//!
//! 代码里直接写中文，用 [`t!`] 包起来；Slint 界面里用 `@tr("…")`。英文译文都在仓库根目录的
//! `i18n/en.po` 里（msgid 是中文原文，msgstr 是英文），Rust 这边在这里查，Slint 那边由
//! [`bundle_for_slint`] 在编译时打包进界面。占位符用 `{}` 或 `{0}`、`{1}`，两边一样。
//!
//! 默认是中文；程序启动时按设置调 [`set`]（设置为"跟随系统"时用 [`system`]）。

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// 界面语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// Slint 的 `select_bundled_translation` 用的名字：中文是源文本，用空字符串。
    pub fn slint_name(self) -> &'static str {
        match self {
            Lang::Zh => "",
            Lang::En => "en",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// 切换语言（之后的 [`t!`] 用新语言）。
pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// 当前语言。
pub fn lang() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En as u8 {
        Lang::En
    } else {
        Lang::Zh
    }
}

/// Windows 的显示语言：中文（简体、繁体）用中文，其他一律英文。
pub fn system() -> Lang {
    #[cfg(windows)]
    {
        // LANGID 的低 10 位是主语言，LANG_CHINESE = 0x04。
        let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        if id & 0x3ff == 0x04 {
            Lang::Zh
        } else {
            Lang::En
        }
    }
    #[cfg(not(windows))]
    {
        let v = std::env::var("LANG").unwrap_or_default();
        if v.starts_with("zh") {
            Lang::Zh
        } else {
            Lang::En
        }
    }
}

/// 英文翻译表的原文，`i18n/en.po`。
pub const EN_PO: &str = include_str!("../../../i18n/en.po");

fn table() -> &'static HashMap<String, String> {
    static T: OnceLock<HashMap<String, String>> = OnceLock::new();
    T.get_or_init(|| parse_po(EN_PO).into_iter().collect())
}

/// 把 `text` 翻译成当前语言。没有译文时原样返回。
pub fn tr(text: &'static str) -> &'static str {
    match lang() {
        Lang::Zh => text,
        Lang::En => table().get(text).map_or(text, |s| s.as_str()),
    }
}

/// 有没有这条的英文（测试用）。
pub fn has_english(text: &str) -> bool {
    table().get(text).is_some_and(|s| !s.is_empty())
}

/// 把参数填进 `{}`、`{0}`、`{1}`…；`{{`、`}}` 是花括号本身。
pub fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut num = String::new();
                for d in chars.by_ref() {
                    if d == '}' {
                        break;
                    }
                    num.push(d);
                }
                let i = if num.is_empty() {
                    next += 1;
                    next - 1
                } else {
                    num.parse().unwrap_or(usize::MAX)
                };
                match args.get(i) {
                    Some(a) => out.push_str(&a.to_string()),
                    None => {
                        out.push('{');
                        out.push_str(&num);
                        out.push('}');
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// 翻译：`t!("解压")` 返回 `&'static str`；带参数时 `t!("打开 {} 失败", path.display())`
/// 返回 `String`。原文必须是字面量，`i18n/en.po` 里要有对应的英文。
#[macro_export]
macro_rules! t {
    ($text:literal) => {
        $crate::tr($text)
    };
    ($text:literal, $($arg:expr),+ $(,)?) => {
        $crate::fill($crate::tr($text), &[$(&$arg as &dyn ::std::fmt::Display),+])
    };
}

/// 解析 .po：只认 `msgid "…"`、`msgstr "…"` 和续行 `"…"`，忽略注释和 msgctxt。
pub fn parse_po(po: &str) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let (mut id, mut s) = (None::<String>, None::<String>);
    for line in po.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("msgid ") {
            if let (Some(i), Some(v)) = (id.take(), s.take()) {
                pairs.push((i, v));
            }
            id = Some(unquote(rest));
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            s = Some(unquote(rest));
        } else if line.starts_with('"') {
            match (s.as_mut(), id.as_mut()) {
                (Some(v), _) => v.push_str(&unquote(line)),
                (None, Some(i)) => i.push_str(&unquote(line)),
                _ => {}
            }
        }
    }
    if let (Some(i), Some(v)) = (id, s) {
        pairs.push((i, v));
    }
    pairs.retain(|(i, v)| !i.is_empty() && !v.is_empty());
    pairs
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 在 build.rs 里调用：把 `i18n/en.po` 放成 Slint 要的
/// `<OUT_DIR>/lang/en/LC_MESSAGES/<包名>.po`，返回给 `with_bundled_translations` 的目录。
pub fn bundle_for_slint() -> std::path::PathBuf {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let pkg = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME");
    let root = out.join("lang");
    let dir = root.join("en").join("LC_MESSAGES");
    std::fs::create_dir_all(&dir).expect("创建翻译目录");
    std::fs::write(dir.join(format!("{pkg}.po")), EN_PO).expect("写翻译文件");
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_placeholders() {
        assert_eq!(fill("打开 {} 失败：{}", &[&"a", &3]), "打开 a 失败：3");
        assert_eq!(fill("{1} 和 {0}", &[&"x", &"y"]), "y 和 x");
        assert_eq!(fill("{{}} {}", &[&1]), "{} 1");
        assert_eq!(fill("缺 {}", &[]), "缺 {}");
    }

    #[test]
    fn po_parsing() {
        let po = "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\n# 注释\nmsgid \"打开\"\nmsgstr \"Open\"\n\nmsgid \"两行\\n第二行\"\nmsgstr \"\"\n\"Two lines\\n\"\n\"second\"\n\nmsgid \"没译\"\nmsgstr \"\"\n";
        let p = parse_po(po);
        assert_eq!(
            p,
            vec![
                ("打开".into(), "Open".into()),
                ("两行\n第二行".into(), "Two lines\nsecond".into())
            ]
        );
    }

    #[test]
    fn switching() {
        set(Lang::Zh);
        assert_eq!(tr("取消"), "取消");
    }
}
