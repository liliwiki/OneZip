//! 代码里每一处 `t!("…")` 和 Slint 的 `@tr("…")` 在 i18n/en.po 里都要有英文。

use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if name != "tests" && name != "target" {
                files(&p, out);
            }
        } else if (name.ends_with(".rs") && name != "build.rs") || name.ends_with(".slint") {
            out.push(p);
        }
    }
}

/// 解出字面量的值（`\n`、`\"`、`\\`、行尾 `\` 续行）。`s` 从开头的 `"` 开始，返回值和结束位置。
fn literal(s: &str) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut it = s.char_indices().skip(1).peekable();
    while let Some((i, c)) = it.next() {
        match c {
            '"' => return Some((out, i + 1)),
            '\\' => match it.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '\r' | '\n' => {
                    while it.peek().is_some_and(|(_, c)| c.is_whitespace()) {
                        it.next();
                    }
                }
                o => out.push(o),
            },
            c => out.push(c),
        }
    }
    None
}

#[test]
fn every_text_has_english() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut list = Vec::new();
    files(root, &mut list);
    let mut missing = Vec::new();
    let mut count = 0;
    for p in list {
        if p.strip_prefix(root).unwrap().starts_with("onezip-i18n") {
            continue;
        }
        let src = std::fs::read_to_string(&p).unwrap();
        // 测试代码不翻译
        let src = match src
            .find("#[cfg(test)]")
            .or_else(|| src.find("#[cfg(all(test"))
        {
            Some(i) => &src[..i],
            None => &src[..],
        };
        for marker in ["t!(\"", "@tr(\""] {
            let mut rest = src;
            while let Some(i) = rest.find(marker) {
                let before = rest[..i].chars().last();
                let from = &rest[i + marker.len() - 1..];
                rest = &rest[i + marker.len()..];
                if marker == "t!(\"" && before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let Some((text, _)) = literal(from) else {
                    continue;
                };
                count += 1;
                if !onezip_i18n::has_english(&text) {
                    missing.push(format!("{}: {text:?}", p.display()));
                }
            }
        }
    }
    assert!(count > 400, "只找到 {count} 处，扫描有问题");
    assert!(
        missing.is_empty(),
        "这些文字在 i18n/en.po 里没有英文：\n{}",
        missing.join("\n")
    );
}
