//! 本地压缩包：交给 hatchzip-core。
//!
//! 流程照搬 HatchZip 自己的命令行（`crates/cli/src/main.rs` 的 `extract`），
//! 只是去掉了交互：密码由请求带入，确认交给前端的回调。

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, bail};
use hatchzip_core::archive::storage::Reads;
use hatchzip_core::engine::{self, Job, Progress, Run};
use hatchzip_core::journal::{Journal, Paths};
use hatchzip_core::replace::Choices;
use hatchzip_core::session::{self, State};

use crate::{Confirm, Event, ExtractRequest, Observer, Outcome, human};

pub(crate) fn extract(
    archive: &Path,
    req: &ExtractRequest,
    observer: Observer,
    stop: &Arc<AtomicBool>,
    confirm: &mut dyn FnMut(&Confirm) -> bool,
) -> Result<Outcome> {
    let (primary, paths) = session::locate(archive)?;

    // 上次是边解边删开始的，这次却要求保留：先拒绝，免得可写打开时把上次
    // 没做完的截断补上。
    if req.keep_source && paths.exists() {
        let journal = Journal::load(&paths)?.0;
        session::keeps_archive(&journal, true)?;
    }

    let reads = Arc::new(Reads::default());
    let (state, _) = session::open_encrypted(
        &primary,
        &paths,
        req.output.clone(),
        true,
        req.password.clone(),
        &reads,
        // 不交互：密码错了或缺了就直接报错，由前端重新带密码再来。
        |_wrong| None,
    )
    .with_context(|| format!("打开 {} 失败", archive.display()))?;

    let (job, journal, resuming) = match state {
        State::Orphan(mut journal) => {
            // 压缩包已经没了，但上次在收尾前中断：只差清理。
            let warning = engine::finish_orphaned(&mut journal, &paths)?;
            return Ok(Outcome::Finished {
                output: journal.output,
                warning,
            });
        }
        State::Resume(job, journal) => {
            let keep = session::keeps_archive(&journal, req.keep_source)?;
            check_space(&job, &journal.output, true, req.keep_free, keep)?;
            session::clear_uncommitted(&job, &journal.output)?;
            (job, journal, true)
        }
        State::Fresh(job, out) => {
            if !req.keep_source
                && let Some(why) = job.in_place_refusal()
            {
                bail!("{why}。可以改用保留原压缩包的方式解压");
            }
            let found = session::existing(&job, &out)?;
            if !found.is_empty() && !req.replace {
                bail!(
                    "{} 里已有 {} 个同名文件（例如 {}）。什么都没改动；如要替换请加上 replace",
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
            let choices = Choices {
                keep: Vec::new(),
                both: Vec::new(),
            };
            let journal = session::begin(
                &job,
                &paths,
                out,
                req.keep_source,
                req.replace.then_some(&choices),
            )?;
            (job, journal, false)
        }
    };

    run(job, journal, paths, resuming, req.keep_free, observer, stop)
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
        observer(Event::Extracted(already));
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
            on_bytes: Box::new(move |n| o1(Event::Extracted(n))),
            on_commit: Box::new(move |len| o2(Event::ArchiveLeft(len))),
        },
        keep_free,
    };
    match run.execute()? {
        engine::Outcome::Finished(warning) => Ok(Outcome::Finished { output, warning }),
        engine::Outcome::Paused => Ok(Outcome::Paused),
    }
}
