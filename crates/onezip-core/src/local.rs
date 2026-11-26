//! 本地压缩包：交给 hatchzip-core。
//!
//! 流程照搬 HatchZip 自己的命令行（`crates/cli/src/main.rs` 的 `extract`），
//! 只是去掉了交互：密码由请求带入，确认交给前端的回调。
//! 在它之上加了智能放置（[`Placement`]）和嵌套压缩包自动拆开。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, bail};
use hatchzip_core::archive::Kind;
use hatchzip_core::archive::storage::Reads;
use hatchzip_core::engine::{self, Job, Progress, Run};
use hatchzip_core::journal::{Journal, Paths};
use hatchzip_core::replace::Choices;
use hatchzip_core::session::{self, State};

use crate::route::Local;
use crate::{Confirm, Conflict, Event, ExtractRequest, Observer, Outcome, Placement, Source, human};

/// 嵌套压缩包最多连续拆几层，防止"压缩包套压缩包"的炸弹。
const MAX_NESTED: usize = 4;

/// 压缩包里的一项。
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

/// 列出压缩包内容，不做任何改动。
pub fn list(archive: &Path, password: Option<String>) -> Result<Vec<Entry>> {
    #[cfg(windows)]
    if onezip_formats::engine(&archive.to_string_lossy()) == onezip_formats::Engine::SevenZip {
        return crate::sevenzip::list(archive, password);
    }
    let job = match open(archive, None, false, password)?.0 {
        State::Fresh(job, _) | State::Resume(job, _) => job,
        State::Orphan(_) => bail!("这个压缩包里已经没有剩余内容"),
    };
    let mut entries: Vec<Entry> = job
        .units
        .iter()
        .flat_map(|u| &u.items)
        .map(|i| Entry {
            path: i.path.clone(),
            size: i.size,
            is_dir: matches!(i.kind, Kind::Dir),
        })
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// 完整校验压缩包（解码一遍、核对校验和），不写出任何文件。
pub fn test(archive: &Path, password: Option<String>, observer: Observer) -> Result<()> {
    #[cfg(windows)]
    if onezip_formats::engine(&archive.to_string_lossy()) == onezip_formats::Engine::SevenZip {
        return crate::sevenzip::test(archive, password, observer);
    }
    let job = match open(archive, None, false, password)?.0 {
        State::Fresh(job, _) => job,
        _ => bail!("这个压缩包正解到一半，不能测试；继续解压即可完成"),
    };
    observer(Event::Started {
        total: job.remaining_size(),
        resuming: false,
    });
    let o = observer.clone();
    job.verify(&move |n| o(Event::Progressed(n)))
        .context("压缩包已损坏")
}

fn open(
    archive: &Path,
    output: Option<PathBuf>,
    writable: bool,
    password: Option<String>,
) -> Result<(State, Paths)> {
    let (primary, paths) = session::locate(archive)?;
    let reads = Arc::new(Reads::default());
    let (state, _) = session::open_encrypted(
        &primary,
        &paths,
        output,
        writable,
        password,
        &reads,
        // 不交互：密码错了或缺了就直接报错，由前端带上密码再来。
        |_wrong| None,
    )
    .with_context(|| format!("打开 {} 失败", archive.display()))?;
    Ok((state, paths))
}

pub(crate) fn extract(
    archive: &Path,
    req: &ExtractRequest,
    observer: Observer,
    stop: &Arc<AtomicBool>,
    confirm: &mut dyn FnMut(&Confirm) -> bool,
) -> Result<Outcome> {
    if crate::route::local(archive, req) == Local::Peel {
        return crate::remote::extract_file(archive, req, observer, stop, confirm);
    }
    let (outcome, nested) = extract_any(archive, req, observer.clone(), stop, confirm)?;
    let Outcome::Finished { output, warning } = outcome else {
        return Ok(outcome);
    };
    if !req.nested {
        return Ok(Outcome::Finished { output, warning });
    }

    // 解出来恰好是一个压缩包（如 .tar 套在 .zip 里）：继续拆，拆完删掉中间包。
    let mut inner = nested;
    let mut depth = 0;
    while let Some(next) = inner.take() {
        depth += 1;
        if depth > MAX_NESTED {
            observer(Event::Note(format!("嵌套超过 {MAX_NESTED} 层，停止继续拆包")));
            break;
        }
        observer(Event::Note(format!(
            "解出的是压缩包 {}，继续解开",
            next.file_name().unwrap_or_default().to_string_lossy()
        )));
        let sub = ExtractRequest {
            source: Source::Local(next.clone()),
            output: None,
            placement: Placement::Smart,
            keep_source: false,
            verify_first: false,
            nested: false,
            ..req.clone()
        };
        match extract_any(&next, &sub, observer.clone(), stop, &mut |_| true) {
            Ok((Outcome::Finished { .. }, more)) => inner = more,
            Ok((Outcome::Paused, _)) => return Ok(Outcome::Paused),
            // 看着像压缩包但打不开：保留它，不算失败。
            Err(e) => {
                observer(Event::Note(format!("没有继续拆开：{e:#}")));
                break;
            }
        }
    }
    Ok(Outcome::Finished { output, warning })
}

/// 解一个包（引擎见 [`crate::route`]）。完成时另外返回"解出来唯一的那个文件，如果它是压缩包"。
fn extract_any(
    archive: &Path,
    req: &ExtractRequest,
    observer: Observer,
    stop: &Arc<AtomicBool>,
    confirm: &mut dyn FnMut(&Confirm) -> bool,
) -> Result<(Outcome, Option<PathBuf>)> {
    #[cfg(windows)]
    if let Local::SevenZip(primary) = crate::route::local(archive, req) {
        let r = crate::sevenzip::extract(&primary, req, observer.clone(), stop, confirm);
        match r {
            // 7z.dll 什么都没解出时不留进度文件，这时换 hatchzip-core 再试是干净的。
            Err(e)
                if crate::needs_password(&e)
                    && crate::route::retry_zip_password_with_hatchzip(&primary, req)
                    && !crate::sevenzip::has_progress(&primary) => {}
            r => return r,
        }
    }
    extract_one(archive, req, observer, stop, confirm)
}

/// 解一个包。完成时另外返回"解出来唯一的那个文件，如果它是压缩包"。
fn extract_one(
    archive: &Path,
    req: &ExtractRequest,
    observer: Observer,
    stop: &Arc<AtomicBool>,
    confirm: &mut dyn FnMut(&Confirm) -> bool,
) -> Result<(Outcome, Option<PathBuf>)> {
    let (primary, paths) = session::locate(archive)?;

    // 上次是边解边删开始的，这次却要求保留：先拒绝，免得可写打开时把上次
    // 没做完的截断补上。
    if req.keep_source && paths.exists() {
        let journal = Journal::load(&paths)?.0;
        session::keeps_archive(&journal, true)?;
    }

    let (state, paths) = open(archive, req.output.clone(), true, req.password.clone())?;

    let (job, journal, resuming) = match state {
        State::Orphan(mut journal) => {
            // 压缩包已经没了，但上次在收尾前中断：只差清理。
            let warning = engine::finish_orphaned(&mut journal, &paths)?;
            let outcome = Outcome::Finished {
                output: journal.output,
                warning,
            };
            return Ok((outcome, None));
        }
        State::Resume(job, journal) => {
            let keep = session::keeps_archive(&journal, req.keep_source)?;
            check_space(&job, &journal.output, true, req.keep_free, keep)?;
            session::clear_uncommitted(&job, &journal.output)?;
            (job, journal, true)
        }
        State::Fresh(job, default_out) => {
            if !req.keep_source
                && let Some(why) = job.in_place_refusal()
            {
                bail!("{why}。可以改用保留原压缩包的方式解压");
            }
            let out = if req.output.is_some() {
                default_out
            } else {
                place(&job, &primary, default_out, req.placement)?
            };
            let found = session::existing(&job, &out)?;
            if !found.is_empty() && req.conflict == Conflict::Refuse {
                bail!(
                    "{} 里已有 {} 个同名文件（例如 {}）。什么都没改动；如要替换请选择替换",
                    out.display(),
                    found.len(),
                    found[0].display()
                );
            }
            check_space(&job, &out, false, req.keep_free, req.keep_source)?;
            if req.verify_first {
                observer(Event::Note("正在预先校验整个压缩包…".into()));
                job.verify(&|_n| {})
                    .context("压缩包已损坏，没有解压任何文件")?;
            }
            if !req.keep_source {
                let name = session::display_name(&job);
                if !confirm(&Confirm {
                    archive: &name,
                    output: &out,
                }) {
                    bail!("已取消，压缩包没有任何改动");
                }
            }
            let choices = match req.conflict {
                Conflict::Skip => Choices { keep: found, both: Vec::new() },
                Conflict::Rename => Choices { keep: Vec::new(), both: found },
                Conflict::Refuse | Conflict::Replace => Choices::default(),
            };
            let journal = session::begin(
                &job,
                &paths,
                out,
                req.keep_source,
                (req.conflict != Conflict::Refuse).then_some(&choices),
            )?;
            (job, journal, false)
        }
    };

    let nested = sole_archive(&job, &journal.output);
    // 直接解到压缩包旁边、且只有一个顶层项时，报告那一项本身，"打开"才会定位到它。
    let beside = (Some(journal.output.as_path()) == primary.parent())
        .then(|| top_item(&job))
        .flatten()
        .map(|top| journal.output.join(top));
    let outcome = run(job, journal, paths, resuming, req.keep_free, observer, stop)?;
    let outcome = match (outcome, beside) {
        (Outcome::Finished { warning, .. }, Some(output)) if output.exists() => {
            Outcome::Finished { output, warning }
        }
        (outcome, _) => outcome,
    };
    let nested = matches!(outcome, Outcome::Finished { .. })
        .then_some(nested)
        .flatten();
    Ok((outcome, nested))
}

/// 按放置策略决定输出目录。
fn place(job: &Job, primary: &Path, folder: PathBuf, placement: Placement) -> Result<PathBuf> {
    let here = primary
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    match placement {
        Placement::NewFolder => Ok(folder),
        Placement::Here => session::retarget(job, &here),
        Placement::Smart if session::single_root(job) => {
            // 顶层只有一项：直接放旁边。放不下（与现有文件冲突等）就退回新建文件夹。
            session::retarget(job, &here).or(Ok(folder))
        }
        Placement::Smart => Ok(folder),
    }
}

/// 所有条目都在同一个顶层名字下时返回这个名字。
fn top_item(job: &Job) -> Option<PathBuf> {
    let mut tops = job
        .units
        .iter()
        .flat_map(|u| &u.items)
        .filter_map(|i| i.path.components().next());
    let first = tops.next()?;
    tops.all(|c| c == first).then(|| PathBuf::from(first.as_os_str()))
}

/// 压缩包里只有一个文件、并且它本身像压缩包时，返回它解出后的路径。
fn sole_archive(job: &Job, out: &Path) -> Option<PathBuf> {
    let mut files = job
        .units
        .iter()
        .flat_map(|u| &u.items)
        .filter(|i| !matches!(i.kind, Kind::Dir));
    let only = files.next()?;
    if files.next().is_some() || !matches!(only.kind, Kind::File) {
        return None;
    }
    onezip_formats::is_nested_candidate(&only.path.to_string_lossy()).then(|| out.join(&only.path))
}

fn check_space(job: &Job, out: &Path, resuming: bool, keep_free: u64, keep: bool) -> Result<()> {
    let plan = session::plan(job, out, resuming, keep_free, keep)?;
    if plan.fits {
        return Ok(());
    }
    let mut msg = format!(
        "空间不够：最忙时需要 {} 空闲（另留 {}），现在只有 {}",
        human(plan.needed),
        human(plan.keep_free),
        human(plan.free)
    );
    if plan.solid_block_bound {
        msg.push_str("。瓶颈是一个固实块，它只能整块解完才能释放");
    }
    if plan.archive_kept_whole {
        msg.push_str("。这个文件系统不支持在文件中间释放空间（如 FAT32/exFAT），压缩包要到最后才能删");
    }
    if !plan.reclaims {
        msg.push_str("。输出和压缩包不在同一个磁盘上，释放压缩包帮不上输出");
    }
    bail!("{msg}。什么都没改动")
}

fn run(
    job: Job,
    journal: Journal,
    paths: Paths,
    resuming: bool,
    keep_free: u64,
    observer: Observer,
    stop: &Arc<AtomicBool>,
) -> Result<Outcome> {
    let total = journal.total_size;
    observer(Event::Started { total, resuming });
    let already = total.saturating_sub(job.remaining_size());
    if already > 0 {
        observer(Event::Progressed(already));
    }
    if !journal.keep {
        observer(Event::ArchiveLeft(journal.committed_len));
    }
    let output = journal.output.clone();
    let (o1, o2) = (observer.clone(), observer.clone());
    let run = Run {
        job,
        journal,
        paths,
        stop,
        progress: Progress {
            on_bytes: Box::new(move |n| o1(Event::Progressed(n))),
            on_commit: Box::new(move |len| o2(Event::ArchiveLeft(len))),
        },
        keep_free,
    };
    match run.execute()? {
        engine::Outcome::Finished(warning) => Ok(Outcome::Finished { output, warning }),
        engine::Outcome::Paused => Ok(Outcome::Paused),
    }
}
