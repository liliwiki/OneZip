//! 任务队列：右键菜单和窗口里发起的所有压缩/解压都排在这里，最多同时跑两个。
//!
//! 引擎在工作线程里运行，进度通过 [`onezip_core::Observer`] 写回这里；
//! 界面定时取快照显示。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use onezip_settings::DeleteMode;
use onezip_core::{
    CompressRequest, Event, ExtractRequest, HashKind, Observer, Outcome, Source, human,
};

const MAX_RUNNING: usize = 2;

#[derive(Debug, Clone)]
pub enum Kind {
    /// 第二项：全部解压成功后要删除压缩包时，怎么删。
    Extract(ExtractRequest, Option<DeleteMode>),
    Compress(CompressRequest),
    Test {
        path: PathBuf,
        password: Option<String>,
    },
    /// 计算校验值（右键菜单 "CRC SHA"）。
    Hash {
        paths: Vec<PathBuf>,
        kinds: Vec<HashKind>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Queued,
    Running,
    Paused,
    Done,
    Cancelled,
    NeedsPassword { retry: bool },
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub kind: Kind,
    pub status: Status,
    pub total: u64,
    pub done: u64,
    /// 本地边解边删：原压缩包还剩多长。
    pub left: Option<u64>,
    /// 远程：已下载、已解出。
    pub remote: Option<(u64, u64)>,
    pub note: String,
    pub output: Option<PathBuf>,
    /// 校验值任务的结果文本。
    pub report: Option<String>,
    /// 这一轮开始运行的时间（算已用时间、速度、剩余时间）。
    pub started: Option<Instant>,
    /// 这一轮结束的时间；结束后已用时间不再增加。
    pub ended: Option<Instant>,
    stop: Arc<AtomicBool>,
}

impl Task {
    pub fn progress(&self) -> f32 {
        match self.status {
            Status::Done => 1.0,
            _ if self.total == 0 => -1.0,
            _ => {
                let done = self.remote.map_or(self.done, |(d, _)| d);
                (done as f64 / self.total as f64).clamp(0.0, 1.0) as f32
            }
        }
    }

    pub fn state_text(&self) -> String {
        match &self.status {
            Status::Queued => "排队中".into(),
            Status::Running => match self.progress() {
                p if p >= 0.0 => format!("{:.0}%", p * 100.0),
                _ => "进行中".into(),
            },
            Status::Paused => "已暂停".into(),
            Status::Done => "完成".into(),
            Status::Cancelled => "已取消".into(),
            Status::NeedsPassword { .. } => "需要密码".into(),
            Status::Failed(_) => "失败".into(),
        }
    }

    pub fn detail(&self) -> String {
        match &self.status {
            Status::Failed(e) => e.clone(),
            Status::Done => match &self.output {
                Some(p) => format!("完成：{}", p.display()),
                None if !self.note.is_empty() => self.note.clone(),
                None => "完成".into(),
            },
            Status::Paused => "进度已保存，可以随时继续（关掉 OneZip 也不会丢）".into(),
            _ if !self.note.is_empty() && self.total == 0 => self.note.clone(),
            _ => {
                let mut s = match self.remote {
                    Some((down, out)) => format!(
                        "已下载 {} / {} · 已解出 {}",
                        human(down),
                        human(self.total),
                        human(out)
                    ),
                    None if self.total > 0 => {
                        format!("{} / {}", human(self.done), human(self.total))
                    }
                    None => String::new(),
                };
                if let Some(left) = self.left {
                    s.push_str(&format!(" · 原压缩包剩余 {}", human(left)));
                }
                if !self.note.is_empty() {
                    s.push_str(" · ");
                    s.push_str(&self.note);
                }
                s
            }
        }
    }

    /// 已处理的字节数（下载任务按已下载算）。
    pub fn processed(&self) -> u64 {
        self.remote.map_or(self.done, |(d, _)| d)
    }

    /// 这一轮已经运行了多久。
    pub fn elapsed(&self) -> Duration {
        match (self.started, self.ended) {
            (Some(s), Some(e)) => e.saturating_duration_since(s),
            (Some(s), None) => s.elapsed(),
            _ => Duration::ZERO,
        }
    }

    /// 平均速度，字节/秒。
    pub fn speed(&self) -> Option<f64> {
        let secs = self.elapsed().as_secs_f64();
        (secs >= 0.5 && self.processed() > 0).then(|| self.processed() as f64 / secs)
    }

    /// 按平均速度估算的剩余时间。
    pub fn remaining(&self) -> Option<Duration> {
        let speed = self.speed()?;
        let left = self.total.checked_sub(self.processed())?;
        (self.total > 0).then(|| Duration::from_secs_f64(left as f64 / speed))
    }

    pub fn is_running(&self) -> bool {
        self.status == Status::Running
    }

    pub fn can_pause(&self) -> bool {
        self.is_running() && matches!(self.kind, Kind::Extract(..))
    }

    pub fn can_resume(&self) -> bool {
        matches!(self.status, Status::Paused | Status::Failed(_))
    }

    pub fn can_cancel(&self) -> bool {
        match self.status {
            Status::Queued => true,
            Status::Running => !matches!(self.kind, Kind::Extract(..)),
            _ => false,
        }
    }

    pub fn can_remove(&self) -> bool {
        !matches!(self.status, Status::Running | Status::Queued)
    }

    pub fn archive_name(&self) -> String {
        match &self.kind {
            Kind::Extract(r, _) => match &r.source {
                Source::Local(p) => file_name(p),
                Source::Url(u) => u.clone(),
            },
            Kind::Test { path, .. } => file_name(path),
            Kind::Compress(r) => file_name(&r.output),
            Kind::Hash { paths, .. } => match paths.as_slice() {
                [one] => file_name(one),
                [first, ..] => format!("{} 等 {} 项", file_name(first), paths.len()),
                [] => String::new(),
            },
        }
    }
}

pub fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

#[derive(Default)]
pub struct Manager {
    tasks: Mutex<Vec<Task>>,
    next: Mutex<u64>,
    /// 正在退出：不再启动排队中的任务。
    closing: AtomicBool,
}

impl Manager {
    pub fn new() -> Arc<Self> {
        let m = Arc::new(Self::default());
        let weak = Arc::downgrade(&m);
        thread::spawn(move || {
            while let Some(m) = weak.upgrade() {
                m.schedule();
                drop(m);
                thread::sleep(Duration::from_millis(100));
            }
        });
        m
    }

    pub fn add(&self, title: String, kind: Kind) -> u64 {
        let mut next = self.next.lock().unwrap();
        *next += 1;
        let id = *next;
        self.tasks.lock().unwrap().push(Task {
            id,
            title,
            kind,
            status: Status::Queued,
            total: 0,
            done: 0,
            left: None,
            remote: None,
            note: String::new(),
            output: None,
            report: None,
            started: None,
            ended: None,
            stop: Arc::new(AtomicBool::new(false)),
        });
        id
    }

    pub fn snapshot(&self) -> Vec<Task> {
        self.tasks.lock().unwrap().clone()
    }

    pub fn get(&self, id: u64) -> Option<Task> {
        self.tasks.lock().unwrap().iter().find(|t| t.id == id).cloned()
    }

    fn with(&self, id: u64, f: impl FnOnce(&mut Task)) {
        if let Some(t) = self.tasks.lock().unwrap().iter_mut().find(|t| t.id == id) {
            f(t);
        }
    }

    pub fn pause(&self, id: u64) {
        self.with(id, |t| {
            if t.can_pause() {
                t.stop.store(true, Ordering::SeqCst);
                t.note = "正在暂停，到下一个安全点就停…".into();
            }
        });
    }

    pub fn resume(&self, id: u64) {
        self.with(id, |t| {
            if t.can_resume() {
                t.stop = Arc::new(AtomicBool::new(false));
                t.status = Status::Queued;
                t.note.clear();
            }
        });
    }

    pub fn cancel(&self, id: u64) {
        self.with(id, |t| match t.status {
            Status::Queued => t.status = Status::Cancelled,
            Status::Running if t.can_cancel() => t.stop.store(true, Ordering::SeqCst),
            _ => {}
        });
    }

    pub fn remove(&self, id: u64) {
        self.tasks
            .lock()
            .unwrap()
            .retain(|t| t.id != id || !t.can_remove());
    }

    pub fn clear_finished(&self) {
        self.tasks
            .lock()
            .unwrap()
            .retain(|t| !matches!(t.status, Status::Done | Status::Cancelled));
    }

    /// 带上密码重新排队。
    pub fn set_password(&self, id: u64, password: String) {
        self.with(id, |t| {
            match &mut t.kind {
                Kind::Extract(r, _) => r.password = Some(password),
                Kind::Test { password: p, .. } => *p = Some(password),
                Kind::Compress(_) | Kind::Hash { .. } => return,
            }
            t.stop = Arc::new(AtomicBool::new(false));
            t.status = Status::Queued;
        });
    }

    /// 还有任务在跑或排队：关窗口前要提醒。
    pub fn busy(&self) -> bool {
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .any(|t| matches!(t.status, Status::Running | Status::Queued))
    }

    /// 退出前调用：在跑的解压停在安全点、压缩取消，排队中的不再启动。
    pub fn pause_all(&self) {
        self.closing.store(true, Ordering::SeqCst);
        for t in self.tasks.lock().unwrap().iter_mut() {
            if t.is_running() {
                t.stop.store(true, Ordering::SeqCst);
            }
        }
    }

    fn schedule(self: &Arc<Self>) {
        if self.closing.load(Ordering::SeqCst) {
            return;
        }
        let mut tasks = self.tasks.lock().unwrap();
        let running = tasks.iter().filter(|t| t.is_running()).count();
        for t in tasks
            .iter_mut()
            .filter(|t| t.status == Status::Queued)
            .take(MAX_RUNNING.saturating_sub(running))
        {
            t.status = Status::Running;
            t.started = Some(Instant::now());
            t.ended = None;
            t.total = 0;
            t.done = 0;
            t.remote = None;
            let (id, kind, stop) = (t.id, t.kind.clone(), t.stop.clone());
            let me = self.clone();
            thread::spawn(move || me.work(id, kind, stop));
        }
    }

    fn observer(self: &Arc<Self>, id: u64) -> Observer {
        let me = Arc::downgrade(self);
        Arc::new(move |ev| {
            let Some(me) = me.upgrade() else { return };
            me.with(id, |t| match ev {
                Event::Started { total, .. } => {
                    t.total = total;
                    t.done = 0;
                }
                Event::Progressed(n) => t.done += n,
                Event::ArchiveLeft(n) => t.left = Some(n),
                Event::Remote {
                    downloaded,
                    extracted,
                    total,
                } => {
                    t.remote = Some((downloaded, extracted));
                    if total > 0 {
                        t.total = total;
                    }
                }
                Event::Note(s) => t.note = s,
            });
        })
    }

    fn work(self: Arc<Self>, id: u64, kind: Kind, stop: Arc<AtomicBool>) {
        let observer = self.observer(id);
        let result: anyhow::Result<(Status, Option<PathBuf>)> = match &kind {
            // 右键菜单里选"释放空间"、或窗口里勾选后确认过，到这里已经是确认过的。
            Kind::Extract(req, after) => {
                onezip_core::extract(req, observer, &stop, &mut |_| true).map(|o| match o {
                    Outcome::Finished { output, warning } => {
                        let note = match (warning, after, &req.source) {
                            // 有需要转告的问题时不删，让用户先看看。
                            (Some(w), Some(_), _) => Some(format!("{w}；压缩包没有删除")),
                            (Some(w), None, _) => Some(w),
                            (None, Some(mode), Source::Local(path)) => delete_archive(path, *mode).err(),
                            (None, _, _) => None,
                        };
                        if let Some(n) = note {
                            self.with(id, |t| t.note = n);
                        }
                        (Status::Done, Some(output))
                    }
                    Outcome::Paused => (Status::Paused, None),
                })
            }
            Kind::Compress(req) => onezip_core::compress(req, observer, &stop)
                .map(|()| (Status::Done, Some(req.output.clone()))),
            Kind::Test { path, password } => {
                onezip_core::test(path, password.clone(), observer).map(|()| {
                    (Status::Done, None)
                })
            }
            Kind::Hash { paths, kinds } => {
                onezip_core::hash(paths, kinds, observer, &stop).map(|report| {
                    let text = report.to_text();
                    self.with(id, |t| {
                        t.note = format!("{} 个文件，点“查看”看结果", report.files.len());
                        t.report = Some(text);
                    });
                    (Status::Done, None)
                })
            }
        };
        self.with(id, |t| t.ended = Some(Instant::now()));
        self.with(id, |t| match result {
            Ok((status, output)) => {
                if status == Status::Done && matches!(kind, Kind::Test { .. }) {
                    t.note = "测试通过，没有发现错误".into();
                    t.output = None;
                } else {
                    t.output = output;
                }
                t.status = status;
            }
            Err(e) if e.chain().any(|c| c.is::<onezip_core::Cancelled>()) => {
                t.status = Status::Cancelled;
            }
            Err(e) if onezip_core::needs_password(&e) => {
                let had = match &t.kind {
                    Kind::Extract(r, _) => r.password.is_some(),
                    Kind::Test { password, .. } => password.is_some(),
                    Kind::Compress(_) | Kind::Hash { .. } => false,
                };
                t.status = Status::NeedsPassword { retry: had };
            }
            Err(e) => {
                crate::log::error(&format!("{}：{e:#}", t.title));
                t.status = Status::Failed(format!("{e:#}"));
            }
        });
    }
}

/// 解压成功后删除压缩包（分卷的每一卷都删）。失败时返回给用户看的说明。
fn delete_archive(path: &std::path::Path, mode: DeleteMode) -> Result<(), String> {
    let volumes = onezip_core::archive_volumes(path);
    let r = match mode {
        DeleteMode::RecycleBin => crate::win::recycle(&volumes),
        DeleteMode::Permanent => volumes.iter().try_for_each(std::fs::remove_file),
    };
    r.map_err(|e| {
        crate::log::error(&format!("删除 {} 失败：{e}", path.display()));
        format!("解压完成，但删除压缩包失败：{e}")
    })
}
