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
    }
}
