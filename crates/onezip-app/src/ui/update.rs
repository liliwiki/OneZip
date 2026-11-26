//! 检查更新的界面部分：自动检查（主窗口打开时、每天最多一次）、手动检查、更新提示窗口、
//! 下载进度，以及"报告问题"。网络都在后台线程，结果经通道交回界面线程（[`poll`]）。

use onezip_i18n::t;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use slint::ComponentHandle;

use super::common::open_url;
use super::settings::{GITHUB, commit};
use super::{App, show_dialog};
use crate::update::{self, Release};
use crate::{SettingsWindow, UpdateDialog};

enum Msg {
    Checked { manual: bool, result: Result<Release, String> },
    Progress(u64, Option<u64>),
    Downloaded(Result<PathBuf, String>),
}

/// 最近一次检查的结果（给设置页显示）。
enum Last {
    Never,
    Checking,
    UpToDate,
    Found(String),
    Failed(String),
}

pub(crate) struct State {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// 这次运行已经自动检查过（或不需要检查）。
    auto_done: bool,
    last: Last,
    dialog: Option<(UpdateDialog, Release)>,
    /// 用户点了"立即更新"，但还有任务在跑：等它们做完。
    waiting: bool,
    downloading: Option<Arc<AtomicBool>>,
}

impl State {
    pub(super) fn dialog(&self) -> Option<&(UpdateDialog, Release)> {
        self.dialog.as_ref()
    }

    pub(super) fn new() -> State {
        let (tx, rx) = channel();
        State {
            tx,
            rx,
            auto_done: false,
            last: Last::Never,
            dialog: None,
            waiting: false,
            downloading: None,
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn start_check(app: &Rc<App>, manual: bool) {
    let mut st = app.update.borrow_mut();
    if matches!(st.last, Last::Checking) {
        return;
    }
    st.last = Last::Checking;
    let tx = st.tx.clone();
    drop(st);
    std::thread::spawn(move || {
        let result = update::latest().map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Checked { manual, result });
    });
    refresh_settings(app);
}

/// 设置页的"检查更新"。
pub(super) fn check_now(app: &Rc<App>) {
    start_check(app, true);
}

/// 每 250 毫秒由主循环调用：需要时自动检查，处理后台结果。
pub(super) fn poll(app: &Rc<App>) {
    let due = {
        let st = app.update.borrow();
        let s = app.settings.borrow();
        !st.auto_done && app.window.window().is_visible() && s.auto_update && now().saturating_sub(s.last_check) >= 86_400
    };
    if due {
        start_check(app, false);
    }
    if app.window.window().is_visible() {
        app.update.borrow_mut().auto_done = true;
    }

    loop {
        let msg = app.update.borrow().rx.try_recv();
        let Ok(msg) = msg else { break };
        match msg {
            Msg::Checked { manual, result } => checked(app, manual, result),
            Msg::Progress(done, total) => {
                if let Some((d, _)) = &app.update.borrow().dialog {
                    d.set_progress(total.map_or(-1.0, |t| done as f32 / t.max(1) as f32));
                    d.set_status(
                        t!("正在下载 {} / {}", onezip_core::human(done), total.map_or("?".into(), onezip_core::human))
                        .into(),
                    );
                }
            }
            Msg::Downloaded(r) => downloaded(app, r),
        }
    }

    let start = {
        let st = app.update.borrow();
        st.waiting && !app.tasks.busy()
    };
    if start {
        app.update.borrow_mut().waiting = false;
        start_download(app);
    }
}

fn checked(app: &Rc<App>, manual: bool, result: Result<Release, String>) {
    // 只有查成功才算"今天查过了"；失败（断网、被限流）下次打开再试。
    if result.is_ok() {
        let mut s = app.settings.borrow().clone();
        s.last_check = now();
        commit(app, s);
    }
    match result {
        Ok(rel) if update::newer(&rel.version, update::current()) => {
            app.update.borrow_mut().last = Last::Found(rel.version.clone());
            let skipped = app.settings.borrow().skipped.contains(&rel.version);
            if manual || !skipped {
                show(app, rel, !manual);
            }
        }
        Ok(_) => app.update.borrow_mut().last = Last::UpToDate,
        Err(e) => {
            crate::log::error(&t!("检查更新失败：{}", e));
            app.update.borrow_mut().last = Last::Failed(e);
        }
    }
    refresh_settings(app);
}

fn refresh_settings(app: &Rc<App>) {
    if let Some(d) = app.settings_window.borrow().as_ref() {
        show_status(app, d);
    }
}

/// 设置页"更新"一栏的文字。
pub(super) fn show_status(app: &Rc<App>, d: &SettingsWindow) {
    let st = app.update.borrow();
    let last_check = app.settings.borrow().last_check;
    let ago = match now().saturating_sub(last_check) {
        _ if last_check == 0 => t!("还没有检查过").to_string(),
        s if s < 120 => t!("上次检查：刚才").into(),
        s if s < 3600 => t!("上次检查：{} 分钟前", s / 60),
        s if s < 86_400 => t!("上次检查：{} 小时前", s / 3600),
        s => t!("上次检查：{} 天前", s / 86_400),
    };
    let text = match &st.last {
        Last::Never => ago,
        Last::Checking => t!("正在检查…").into(),
        Last::UpToDate => t!("已是最新版 · {}", ago),
        Last::Found(v) => t!("有新版本 {} · {}", v, ago),
        Last::Failed(e) => t!("检查失败：{}", e),
    };
    d.set_update_status(text.into());
    d.set_checking(matches!(st.last, Last::Checking));
    d.set_skipped(app.settings.borrow().skipped.join(t!("、")).into());
}

fn show(app: &Rc<App>, rel: Release, automatic: bool) {
    if let Some((old, _)) = app.update.borrow_mut().dialog.take() {
        let _ = old.hide();
    }
    let Ok(d) = UpdateDialog::new() else { return };
    d.set_heading(t!("OneZip {} 已发布（你现在用的是 {}）", rel.version, update::current()).into());
    d.set_notes(if rel.notes.trim().is_empty() {
        t!("更新说明见发布页：\n{}", rel.page).into()
    } else {
        rel.notes.clone().into()
    });
    d.set_automatic(automatic);

    d.on_update_now({
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            remember_never_ask(&app);
            if app.tasks.busy() {
                app.update.borrow_mut().waiting = true;
                if let Some((d, _)) = &app.update.borrow().dialog {
                    d.set_busy(true);
                    d.set_failed(false);
                    d.set_progress(-1.0);
                    d.set_status(t!("还有任务在进行，等它们完成后自动开始更新…").into());
                }
            } else {
                start_download(&app);
            }
        }
    });
    d.on_skip_version({
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            remember_never_ask(&app);
            let Some((d, rel)) = app.update.borrow_mut().dialog.take() else { return };
            let _ = d.hide();
            let mut s = app.settings.borrow().clone();
            if !s.skipped.contains(&rel.version) {
                s.skipped.push(rel.version);
            }
            commit(&app, s);
            refresh_settings(&app);
        }
    });
    d.on_later({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                remember_never_ask(&app);
                close(&app);
            }
        }
    });
    d.window().on_close_requested({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                close(&app);
            }
            slint::CloseRequestResponse::HideWindow
        }
    });
    show_dialog(app, &d);
    app.update.borrow_mut().dialog = Some((d, rel));
}

/// 勾了"以后不再自动提示"：关掉自动检查。
fn remember_never_ask(app: &Rc<App>) {
    let never = app.update.borrow().dialog.as_ref().is_some_and(|(d, _)| d.get_never_ask());
    if never && app.settings.borrow().auto_update {
        let mut s = app.settings.borrow().clone();
        s.auto_update = false;
        commit(app, s);
        if let Some(d) = app.settings_window.borrow().as_ref() {
            d.set_auto_update(false);
        }
    }
}

/// 关掉提示窗口：正在下载的取消，在等任务的不等了。
fn close(app: &Rc<App>) {
    let mut st = app.update.borrow_mut();
    st.waiting = false;
    if let Some(stop) = st.downloading.take() {
        stop.store(true, Ordering::Relaxed);
    }
    if let Some((d, _)) = st.dialog.take() {
        let _ = d.hide();
    }
}

fn start_download(app: &Rc<App>) {
    let mut st = app.update.borrow_mut();
    let Some((d, rel)) = &st.dialog else { return };
    d.set_busy(true);
    d.set_failed(false);
    d.set_progress(-1.0);
    d.set_status(t!("正在下载…").into());
    let rel = rel.clone();
    let stop = Arc::new(AtomicBool::new(false));
    st.downloading = Some(stop.clone());
    let tx = st.tx.clone();
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let last = std::cell::Cell::new(0u64);
        let r = update::download(
            &rel,
            &|done, total| {
                // 每 256 KB 报一次就够了
                if done - last.get() >= 256 << 10 || Some(done) == total {
                    last.set(done);
                    let _ = progress_tx.send(Msg::Progress(done, total));
                }
            },
            &stop,
        );
        let _ = tx.send(Msg::Downloaded(r.map_err(|e| format!("{e:#}"))));
    });
}

fn downloaded(app: &Rc<App>, r: Result<PathBuf, String>) {
    let cancelled = app.update.borrow_mut().downloading.take().is_none();
    if cancelled {
        return;
    }
    let setup = match r {
        Ok(p) => p,
        Err(e) => {
            crate::log::error(&t!("下载更新失败：{}", e));
            if let Some((d, _)) = &app.update.borrow().dialog {
                d.set_busy(false);
                d.set_failed(true);
                d.set_status(t!("更新失败：{}", e).into());
            }
            return;
        }
    };
    crate::log::info(&t!("开始安装更新 {}", setup.display()));
    if let Err(e) = update::run_installer(&setup) {
        crate::log::error(&format!("{e:#}"));
        if let Some((d, _)) = &app.update.borrow().dialog {
            d.set_busy(false);
            d.set_failed(true);
            d.set_status(t!("更新失败：{}", format!("{:#}", e)).into());
        }
        return;
    }
    // 安装程序接手：它会等 OneZip 退出后替换文件，再重新打开。
    let _ = slint::quit_event_loop();
}

/// 设置 → 关于 →"报告问题"：在浏览器里打开 GitHub 新问题页，填好版本、系统和最近的出错记录。
/// 用户在网页上能看到全部内容，提交前可以删改。
pub(super) fn report_issue() {
    let mut log = crate::log::recent(20).join("\n");
    // 网址不能太长（GitHub 约 8 KB）：只留最后一段。
    if log.chars().count() > 1500 {
        log = log.chars().rev().take(1500).collect::<Vec<_>>().into_iter().rev().collect();
    }
    if log.is_empty() {
        log = t!("（没有出错记录）").into();
    }
    let body = t!("**OneZip 版本**：{}\n**Windows**：{}\n\n**发生了什么、怎么操作的**：\n\n\n**最近的出错记录**（可删改）：\n```\n{}\n```\n", update::current(), crate::win::windows_version(), log);
    open_url(&format!("{GITHUB}/issues/new?body={}", percent_encode(&body)));
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 有新版本还没处理时，主窗口关不关都不影响；这里只给退出判断用。
pub(super) fn has_window(app: &App) -> bool {
    app.update.borrow().dialog.is_some()
}
