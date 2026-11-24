//! onezip.exe：OneZip 命令行。
//!
//! ```text
//! onezip x <压缩包或URL> [-o 输出] [--keep] [--yes] [-p 密码] [--verify] [--replace]
//! ```
//! Ctrl+C 会在下一个安全点暂停并保存进度；用同样的命令再运行一次即可继续。

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use onezip_core::{Confirm, Event, ExtractRequest, Outcome, Source, human};

const EXIT_FAILURE: u8 = 1;
const EXIT_PAUSED: u8 = 3;

#[derive(Parser)]
#[command(name = "onezip", version, about = "OneZip：边解压边释放空间、边下载边解压")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 解压本地压缩包或 http(s) URL
    #[command(alias = "extract")]
    X {
        /// 本地路径或 http(s):// 地址
        source: String,
        /// 输出目录（默认：智能决定，放在压缩包旁边或当前目录）
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 保留原压缩包，不边解边删
        #[arg(short, long)]
        keep: bool,
        /// 不询问，直接确认边解边删
        #[arg(short, long)]
        yes: bool,
        /// 密码
        #[arg(short, long)]
        password: Option<String>,
        /// 动手前先完整校验一遍压缩包
        #[arg(long)]
        verify: bool,
        /// 替换输出目录里已存在的同名文件
        #[arg(long)]
        replace: bool,
        /// 磁盘至少保留的空闲空间，单位 MB
        #[arg(long, default_value_t = 512)]
        keep_free_mb: u64,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("错误：{e:#}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

fn run(command: Command) -> Result<u8> {
    let Command::X {
        source,
        output,
        keep,
        yes,
        password,
        verify,
        replace,
        keep_free_mb,
    } = command;

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || {
            if stop.swap(true, Ordering::SeqCst) {
                // 第二次 Ctrl+C：不等安全点了。进度以最后一次落盘为准。
                std::process::exit(130);
            }
            eprintln!("\n正在暂停，完成当前文件后保存进度…（再按一次立即退出）");
        })?;
    }

    let req = ExtractRequest {
        source: Source::parse(&source),
        output,
        keep_source: keep,
        password,
        verify_first: verify,
        keep_free: keep_free_mb << 20,
        replace,
    };

    let bar = ProgressBar::hidden();
    let bar = Arc::new(Mutex::new(bar));
    let observer = {
        let bar = bar.clone();
        Arc::new(move |ev: Event| {
            let b = bar.lock().unwrap();
            match ev {
                Event::Started { total, resuming } => {
                    let nb = ProgressBar::new(total).with_style(
                        ProgressStyle::with_template(
                            "{prefix} [{bar:30}] {bytes}/{total_bytes} {bytes_per_sec} {msg}",
                        )
                        .unwrap()
                        .progress_chars("=> "),
                    );
                    nb.set_prefix(if resuming { "继续解压" } else { "解压" });
                    nb.enable_steady_tick(Duration::from_millis(200));
                    drop(b);
                    *bar.lock().unwrap() = nb;
                }
                Event::Extracted(n) => b.inc(n),
                Event::ArchiveLeft(len) => b.set_message(format!("压缩包剩余 {}", human(len))),
                Event::Remote {
                    downloaded,
                    extracted,
                    total,
                } => {
                    if total > 0 {
                        b.set_length(total);
                    }
                    b.set_position(downloaded);
                    b.set_message(format!("已解出 {}", human(extracted)));
                }
                Event::Note(s) => b.println(s),
            }
        })
    };

    let mut confirm = |c: &Confirm| ask(&format!(
        "解压到 {}，并在解压过程中删除 {}？此操作不可撤销 [y/N] ",
        c.output.display(),
        c.archive
    ), yes);

    let outcome = onezip_core::extract(&req, observer, &stop, &mut confirm);
    bar.lock().unwrap().finish_and_clear();

    match outcome? {
        Outcome::Finished { output, warning } => {
            if let Some(w) = warning {
                eprintln!("警告：{w}");
            }
            println!("完成，文件在 {}", output.display());
            Ok(0)
        }
        Outcome::Paused => {
            println!("已暂停，进度已保存。运行同样的命令即可继续。");
            Ok(EXIT_PAUSED)
        }
    }
}

fn ask(question: &str, yes: bool) -> bool {
    if yes {
        return true;
    }
    if !io::stdin().is_terminal() {
        eprintln!("会删除原压缩包，非交互运行时请加 --yes 确认，或用 --keep 保留压缩包");
        return false;
    }
    print!("{question}");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    let _ = io::stdin().lock().read_line(&mut answer);
    matches!(answer.trim(), "y" | "Y" | "yes")
}
