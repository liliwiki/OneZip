//! 命令行参数 ↔ 请求，以及单实例之间传递请求的格式。
//!
//! 右键菜单、拖到 exe 上、第二次启动，都会变成一个 [`Request`]：
//! 已有 OneZip 窗口在运行时，通过命名管道交给它排进同一个任务队列，自己立即退出。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
use onezip_core::{Format, Placement};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 只是打开（或唤起）窗口。
    Show,
    Extract {
        placement: Placement,
        /// 先让用户选输出文件夹。
        ask_folder: bool,
        /// 边解边删原压缩包。右键菜单里选这一项本身就是确认。
        reclaim: bool,
    },
    Test,
    /// `None`：弹出压缩对话框；`Some`：直接压成该格式。
    Compress(Option<Format>),
    Url(String),
    RegisterMenu,
    UnregisterMenu,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub action: Action,
    pub paths: Vec<PathBuf>,
}

impl Request {
    /// 解析命令行（不含程序名）。
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut action = None;
        let mut reclaim = false;
        let mut paths = Vec::new();
        let mut args = args.into_iter();
        while let Some(a) = args.next() {
            match a.as_str() {
                "--extract-smart" => action = Some(extract(Placement::Smart, false)),
                "--extract-here" => action = Some(extract(Placement::Here, false)),
                "--extract-folder" => action = Some(extract(Placement::NewFolder, false)),
                "--extract-to" => action = Some(extract(Placement::Smart, true)),
                "--reclaim" => reclaim = true,
                "--test" => action = Some(Action::Test),
                "--compress" => action = Some(Action::Compress(None)),
                "--compress-7z" => action = Some(Action::Compress(Some(Format::SevenZ))),
                "--compress-zip" => action = Some(Action::Compress(Some(Format::Zip))),
                "--url" => match args.next() {
                    Some(u) => action = Some(Action::Url(u)),
                    None => bail!("--url 后面要跟一个链接"),
                },
                "--register-menu" => action = Some(Action::RegisterMenu),
                "--unregister-menu" => action = Some(Action::UnregisterMenu),
                "/quiet" => paths.push(PathBuf::from(a)),
                s if s.starts_with("--") => bail!("不认识的参数 {s}"),
                _ => paths.push(PathBuf::from(a)),
            }
        }
        let action = match action {
            Some(Action::Extract {
                placement,
                ask_folder,
                ..
            }) => Action::Extract {
                placement,
                ask_folder,
                reclaim,
            },
            Some(a) => a,
            // 直接把压缩包拖到 OneZip.exe 上 / 双击关联文件：智能解压并保留原包。
            None if !paths.is_empty() => extract(Placement::Smart, false),
            None => Action::Show,
        };
        Ok(Self { action, paths })
    }

    fn encode(&self) -> String {
        let head = match &self.action {
            Action::Show => "show".to_string(),
            Action::Extract {
                placement,
                ask_folder,
                reclaim,
            } => format!(
                "extract {} {} {}",
                match placement {
                    Placement::Smart => "smart",
                    Placement::Here => "here",
                    Placement::NewFolder => "folder",
                },
                u8::from(*ask_folder),
                u8::from(*reclaim)
            ),
            Action::Test => "test".into(),
            Action::Compress(f) => format!("compress {}", f.map_or("ask", |f| f.extension())),
            Action::Url(u) => format!("url {u}"),
            Action::RegisterMenu => "register".into(),
            Action::UnregisterMenu => "unregister".into(),
        };
        let mut s = head;
        s.push('\n');
        for p in &self.paths {
            s.push_str(&p.to_string_lossy());
            s.push('\n');
        }
        s.push('\n');
        s
    }

    fn decode(lines: &[String]) -> Option<Self> {
        let (head, rest) = lines.split_first()?;
        let mut words = head.splitn(2, ' ');
        let action = match (words.next()?, words.next()) {
            ("show", _) => Action::Show,
            ("extract", Some(r)) => {
                let w: Vec<&str> = r.split(' ').collect();
                Action::Extract {
                    placement: match *w.first()? {
                        "here" => Placement::Here,
                        "folder" => Placement::NewFolder,
                        _ => Placement::Smart,
                    },
                    ask_folder: w.get(1) == Some(&"1"),
                    reclaim: w.get(2) == Some(&"1"),
                }
            }
            ("test", _) => Action::Test,
            ("compress", Some("ask")) => Action::Compress(None),
            ("compress", Some(f)) => Action::Compress(Some(Format::parse(f)?)),
            ("url", Some(u)) => Action::Url(u.to_string()),
            ("register", _) => Action::RegisterMenu,
            ("unregister", _) => Action::UnregisterMenu,
            _ => return None,
        };
        Some(Self {
            action,
            paths: rest.iter().map(PathBuf::from).collect(),
        })
    }
}

fn extract(placement: Placement, ask_folder: bool) -> Action {
    Action::Extract {
        placement,
        ask_folder,
        reclaim: false,
    }
}

/// 每个用户一个管道名，不同用户的 OneZip 互不干扰。
fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let user: String = user
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("OneZip-{user}.sock")
}

/// 交给已经在运行的 OneZip。没有在运行的则返回 `false`。
pub fn forward(req: &Request) -> bool {
    let Ok(name) = pipe_name().to_ns_name::<GenericNamespaced>() else {
        return false;
    };
    let Ok(conn) = Stream::connect(name) else {
        return false;
    };
    // 我们是刚被资源管理器启动的前台进程；把前台权限让给已在运行的实例，
    // 它的窗口才能出现在最前面。
    crate::win::allow_any_foreground();
    let mut conn = BufReader::new(conn);
    if conn.get_mut().write_all(req.encode().as_bytes()).is_err() {
        return false;
    }
    let mut ack = String::new();
    conn.read_line(&mut ack).is_ok() && ack.trim() == "ok"
}

/// 成为"主实例"：开始监听，收到的每个请求交给 `handle`。
/// 管道已被别的实例占用时返回错误。
pub fn listen(handle: impl Fn(Request) + Send + 'static) -> Result<()> {
    let name = pipe_name().to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let mut conn = BufReader::new(conn);
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                match conn.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = line.trim_end_matches(['\r', '\n']).to_string();
                        if line.is_empty() {
                            break;
                        }
                        lines.push(line);
                    }
                }
            }
            let _ = conn.get_mut().write_all(b"ok\n");
            if let Some(req) = Request::decode(&lines) {
                handle(req);
            }
        }
    });
    Ok(())
}

/// 交给已有实例，或者自己成为主实例。处理多个右键菜单进程同时启动的竞争。
pub enum Role {
    Forwarded,
    Primary,
}

pub fn claim(req: &Request, handle: impl Fn(Request) + Send + Clone + 'static) -> Role {
    for _ in 0..40 {
        if forward(req) {
            return Role::Forwarded;
        }
        if listen(handle.clone()).is_ok() {
            return Role::Primary;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // 管道出了问题：至少自己把事情做完。
    Role::Primary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Request {
        Request::from_args(s.iter().map(|s| s.to_string())).unwrap()
    }

    #[test]
    fn parse_and_round_trip() {
        let r = args(&["--reclaim", "--extract-smart", r"D:\a b\x.zip", r"D:\y.7z"]);
        assert_eq!(
            r.action,
            Action::Extract {
                placement: Placement::Smart,
                ask_folder: false,
                reclaim: true
            }
        );
        assert_eq!(r.paths.len(), 2);
        let lines: Vec<String> = r.encode().lines().map(str::to_string).collect();
        assert_eq!(Request::decode(&lines[..lines.len() - 1]), Some(r));

        assert_eq!(args(&[]).action, Action::Show);
        assert_eq!(args(&["x.zip"]).action, extract(Placement::Smart, false));
        let c = args(&["--compress-7z", "a"]);
        assert_eq!(c.action, Action::Compress(Some(Format::SevenZ)));
        let lines: Vec<String> = c.encode().lines().map(str::to_string).collect();
        assert_eq!(Request::decode(&lines[..lines.len() - 1]), Some(c));
    }
}
