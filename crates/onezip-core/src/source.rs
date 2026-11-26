use std::path::PathBuf;

/// 压缩包从哪来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Local(PathBuf),
    Url(String),
}

impl Default for Source {
    fn default() -> Self {
        Source::Local(PathBuf::new())
    }
}

impl Source {
    /// `http://` / `https://` 开头的当作 URL，其余当作本地路径。
    pub fn parse(s: &str) -> Self {
        let lower = s.trim_start().to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            Source::Url(s.trim().to_string())
        } else {
            Source::Local(PathBuf::from(s))
        }
    }
}

/// 链接里的文件名（去掉查询参数），如 `https://h/a/data.tar.zst?x=1` → `data.tar.zst`。
pub(crate) fn url_file_name(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.split(['?', '#']).next()?;
    let (_, path) = path.split_once('/')?;
    let last = path.rsplit('/').next()?;
    (!last.is_empty()).then(|| last.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(
            Source::parse("HTTPS://x/a.zip"),
            Source::Url("HTTPS://x/a.zip".into())
        );
        assert_eq!(
            Source::parse(r"D:\a.zip"),
            Source::Local(PathBuf::from(r"D:\a.zip"))
        );
        assert_eq!(url_file_name("https://h.example/a/b/data.tar.zst?x=1").as_deref(), Some("data.tar.zst"));
        assert_eq!(url_file_name("https://h.example/"), None);
    }
}
