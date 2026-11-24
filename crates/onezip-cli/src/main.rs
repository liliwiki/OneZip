//! onezipc.exe：OneZip 命令行（与 OneZip.exe 区分，Windows 文件名不分大小写）。
//!
//! ```text
//! onezipc x <压缩包或URL> [-o 输出] [--here|--folder] [--keep] [--yes] [-p 密码] [--nested]
//! onezipc a <输出压缩包> <文件或文件夹>... [-t 7z|zip|tar.zst|tar.xz|tar.gz] [-l 0-9] [-p 密码]
//! onezipc l <压缩包>
//! onezipc t <压缩包>
//! ```
//! 解压时 Ctrl+C 会在下一个安全点暂停并保存进度；用同样的命令再运行一次即可继续。

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use onezip_core::{
    CompressRequest, Confirm, Event, ExtractRequest, Format, Observer, Outcome, Placement, Source,
    human,
};

const EXIT_FAILURE: u8 = 1;
const EXIT_DAMAGED: u8 = 2;
const EXIT_PAUSED: u8 = 3;

#[derive(Parser)]
#[command(name = "onezipc", version, about = "OneZip：压缩、解压、边解压边释放空间、边下载边解压")]
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
        /// 输出目录（默认：智能决定）
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 直接解到压缩包所在的文件夹
        #[arg(long, conflicts_with = "folder")]
        here: bool,
        /// 总是新建一个以压缩包命名的文件夹
        #[arg(long)]
        folder: bool,
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
        /// 解出来只有一个压缩包时继续解开它
        #[arg(long)]
        nested: bool,
        /// 磁盘至少保留的空闲空间，单位 MB
        #[arg(long, default_value_t = 512)]
        keep_free_mb: u64,
    },
    /// 压缩文件和文件夹
    #[command(alias = "add")]
    A {
        /// 输出的压缩包；扩展名决定格式（也可用 -t 指定）
        archive: PathBuf,
        /// 要压缩的文件和文件夹
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// 格式：7z、zip、tar.zst、tar.xz、tar.gz
        #[arg(short = 't', long)]
        format: Option<String>,
        /// 压缩级别 0（只存储）到 9（最强）
        #[arg(short, long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(0..=9))]
        level: u32,
        /// 密码（7z 与 zip）
        #[arg(short, long)]
        password: Option<String>,
        /// 7z 不使用固实压缩
        #[arg(long)]
        no_solid: bool,
        /// 覆盖已存在的压缩包
        #[arg(long)]
        overwrite: bool,
    },
    /// 列出压缩包内容
    #[command(alias = "list")]
    L {
        archive: PathBuf,
        #[arg(short, long)]
        password: Option<String>,
    },
    /// 测试压缩包完整性
    #[command(alias = "test")]
    T {
        archive: PathBuf,
        #[arg(short, long)]
        password: Option<String>,
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
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || {
            if stop.swap(true, Ordering::SeqCst) {
                // 第二次 Ctrl+C：不等安全点了。进度以最后一次落盘为准。
                std::process::exit(130);
            }
            eprintln!("\n正在停止，到下一个安全点后保存进度…（再按一次立即退出）");
        })?;
    }

    match command {
        Command::X {
            source,
            output,
            here,
            folder,
            keep,
            yes,
            password,
            verify,
            replace,
            nested,
            keep_free_mb,
        } => {
            let req = ExtractRequest {
                source: Source::parse(&source),
                output,
                placement: if here {
                    Placement::Here
                } else if folder {
                    Placement::NewFolder
                } else {
                    Placement::Smart
                },
                keep_source: keep,
                password,
                verify_first: verify,
                keep_free: keep_free_mb << 20,
                replace,
                nested,
            };
            let (observer, bar) = progress();
            let mut confirm = |c: &Confirm| {
                ask(
                    &format!(
                        "解压到 {}，并在解压过程中删除 {}？此操作不可撤销 [y/N] ",
                        c.output.display(),
                        c.archive
                    ),
                    yes,
                )
            };
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
        Command::A {
            archive,
            inputs,
            format,
            level,
            password,
            no_solid,
            overwrite,
        } => {
            let format = match format {
                Some(f) => Format::parse(&f).ok_or_else(|| anyhow!("不认识的格式 {f}"))?,
                None => format_from_name(&archive)?,
            };
            let mut req = CompressRequest::new(inputs, archive, format);
            req.level = level;
            req.password = password;
            req.solid = !no_solid;
            req.overwrite = overwrite;
            let (observer, bar) = progress();
            let result = onezip_core::compress(&req, observer, &stop);
            bar.lock().unwrap().finish_and_clear();
            match result {
                Ok(()) => {
                    let size = std::fs::metadata(&req.output).map(|m| m.len()).unwrap_or(0);
                    println!("完成：{}（{}）", req.output.display(), human(size));
                    Ok(0)
                }
                Err(e) if e.chain().any(|c| c.is::<onezip_core::Cancelled>()) => {
                    println!("已取消，没有留下文件。");
                    Ok(EXIT_PAUSED)
                }
                Err(e) => Err(e),
            }
        }
        Command::L { archive, password } => {
            let entries = onezip_core::list(&archive, password)?;
            let mut total = 0;
            for e in &entries {
                if e.is_dir {
                    println!("{:>10}  {}/", "", e.path.display());
                } else {
                    total += e.size;
                    println!("{:>10}  {}", human(e.size), e.path.display());
                }
            }
            let files = entries.iter().filter(|e| !e.is_dir).count();
            println!("共 {files} 个文件，解压后 {}", human(total));
            Ok(0)
        }
        Command::T { archive, password } => {
            let (observer, bar) = progress();
            let result = onezip_core::test(&archive, password, observer);
            bar.lock().unwrap().finish_and_clear();
            match result {
                Ok(()) => {
                    println!("测试通过，没有发现错误。");
                    Ok(0)
                }
                Err(e) => {
                    eprintln!("测试失败：{e:#}");
                    Ok(EXIT_DAMAGED)
                }
            }
        }
    }
}

fn format_from_name(path: &std::path::Path) -> Result<Format> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    Format::ALL
        .into_iter()
        .find(|f| name.ends_with(&format!(".{}", f.extension())))
        .ok_or_else(|| anyhow!("从文件名看不出格式，请用 -t 指定"))
        .or_else(|e| if name.ends_with(".tgz") { Ok(Format::TarGz) } else { bail!(e) })
}

fn progress() -> (Observer, Arc<Mutex<ProgressBar>>) {
    let bar = Arc::new(Mutex::new(ProgressBar::hidden()));
    let b2 = bar.clone();
    let observer: Observer = Arc::new(move |ev: Event| {
        let mut b = b2.lock().unwrap();
        match ev {
            Event::Started { total, resuming } => {
                let nb = ProgressBar::new(total).with_style(
                    ProgressStyle::with_template(
                        "{prefix} [{bar:30}] {bytes}/{total_bytes} {bytes_per_sec} {msg}",
                    )
                    .unwrap()
                    .progress_chars("=> "),
                );
                nb.set_prefix(if resuming { "继续" } else { "处理" });
                nb.enable_steady_tick(Duration::from_millis(200));
                *b = nb;
            }
            Event::Progressed(n) => b.inc(n),
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
    });
    (observer, bar)
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
