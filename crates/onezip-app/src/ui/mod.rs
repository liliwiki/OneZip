//! 界面：主窗口、各个对话框、进度窗口，和任务队列之间的胶水。
//!
//! - [`main_window`]：主窗口的按钮和关闭；
//! - [`requests`]：把一个请求（右键菜单、命令行、按钮）变成任务；
//! - [`compress`]、[`password`]、[`hash`]、[`progress`]、[`settings`]、[`update`]：各自的窗口；
//! - [`common`]：提示框、文件对话框等小工具。

mod browser;
mod common;
mod compress;
mod confirm;
mod hash;
mod main_window;
mod password;
mod same_names;
mod progress;
mod requests;
mod settings;
mod update;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::request::Request;
use crate::tasks::{Manager, Status};
use onezip_settings::Settings;

use crate::{
    BrowserWindow, CompressDialog, ConfirmDialog, HashDialog, MainWindow, PasswordDialog, ProgressDialog, SameNamesDialog, SettingsWindow, TaskRow, UpdateDialog, win,
};

/// 跟着设置里的主题切换浅色 / 深色的窗口（都继承 theme.slint 的 ThemedWindow）。
pub(super) trait Themed {
    fn apply(&self, theme: i32);
}

macro_rules! themed {
    ($($t:ty),*) => {
        $(impl Themed for $t {
            fn apply(&self, theme: i32) {
                use slint::language::ColorScheme;
                self.global::<crate::Palette>().set_color_scheme(match theme {
                    1 => ColorScheme::Light,
                    2 => ColorScheme::Dark,
                    _ => ColorScheme::Unknown,
                });
            }
        })*
    };
}
themed!(BrowserWindow, MainWindow, CompressDialog, ConfirmDialog, PasswordDialog, HashDialog, ProgressDialog, SameNamesDialog, SettingsWindow, UpdateDialog);

pub(super) struct App {
    pub(super) window: MainWindow,
    pub(super) tasks: Arc<Manager>,
    /// 打开中的对话框，防止被提前释放。
    pub(super) compress: RefCell<Option<CompressDialog>>,
    pub(super) password: RefCell<Option<PasswordDialog>>,
    /// 已经自动弹过密码框的任务（输入后又报密码错误时会移出，再弹一次）。
    pub(super) prompted: RefCell<HashSet<u64>>,
    /// 打开着的校验值结果窗口。
    pub(super) hashes: RefCell<Vec<HashDialog>>,
    /// 已经自动弹过结果窗口的校验值任务。
    pub(super) shown: RefCell<HashSet<u64>>,
    /// 右键菜单发起的任务各自的进度窗口。
    pub(super) progress: RefCell<HashMap<u64, ProgressDialog>>,
    /// 第一个请求已经处理过（在那之前没有窗口是正常的，不能当成可以退出）。
    pub(super) ready: Cell<bool>,
    /// 当前设置（改动由设置窗口保存，见 [`settings::commit`]）。
    pub(super) settings: RefCell<Settings>,
    pub(super) settings_window: RefCell<Option<SettingsWindow>>,
    pub(super) update: RefCell<update::State>,
    /// 开着的"边解压边删除"说明对话框。
    pub(super) confirms: RefCell<Vec<ConfirmDialog>>,
    /// 开着的"有同名文件"询问窗口和它对应的任务。
    pub(super) same_names: RefCell<Option<(u64, SameNamesDialog)>>,
    /// 开着的压缩包浏览窗口。
    pub(super) browsers: RefCell<Vec<Rc<browser::Browser>>>,
}

impl App {
    pub(super) fn theme(&self) -> i32 {
        settings::theme_index(self.settings.borrow().theme)
    }

    /// 主题改了：所有开着的窗口一起换。
    pub(super) fn apply_theme(&self) {
        let t = self.theme();
        self.window.apply(t);
        for d in self.compress.borrow().iter() {
            d.apply(t);
        }
        for d in self.password.borrow().iter() {
            d.apply(t);
        }
        for d in self.hashes.borrow().iter() {
            d.apply(t);
        }
        for d in self.progress.borrow().values() {
            d.apply(t);
        }
        for d in self.settings_window.borrow().iter() {
            d.apply(t);
        }
        if let Some((d, _)) = self.update.borrow().dialog() {
            d.apply(t);
        }
        for d in self.confirms.borrow().iter() {
            d.apply(t);
        }
        if let Some((_, d)) = self.same_names.borrow().as_ref() {
            d.apply(t);
        }
        for b in self.browsers.borrow().iter() {
            b.window.apply(t);
        }
    }
}

pub fn run(first: Request, incoming: Receiver<Request>) -> anyhow::Result<()> {
    let window = MainWindow::new()?;
    let app = Rc::new(App {
        window,
        tasks: Manager::new(),
        compress: RefCell::new(None),
        password: RefCell::new(None),
        prompted: RefCell::new(HashSet::new()),
        hashes: RefCell::new(Vec::new()),
        shown: RefCell::new(HashSet::new()),
        progress: RefCell::new(HashMap::new()),
        ready: Cell::new(false),
        settings: RefCell::new(Settings::load()),
        settings_window: RefCell::new(None),
        update: RefCell::new(update::State::new()),
        confirms: RefCell::new(Vec::new()),
        same_names: RefCell::new(None),
        browsers: RefCell::new(Vec::new()),
    });
    main_window::wire(&app);
    app.window.apply(app.theme());
    app.window.set_status(main_window::status_text(&app.settings.borrow()).into());

    // 定时：收取其它实例转来的请求，刷新任务列表。
    let timer = Timer::default();
    {
        let app = app.clone();
        timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
            while let Ok(req) = incoming.try_recv() {
                requests::handle(&app, req, true);
            }
            refresh(&app);
            update::poll(&app);
            quit_if_idle(&app);
        });
    }

    // 事件循环开始后再处理第一个请求：那时窗口已创建，才能提到最前、给对话框指定归属。
    {
        let app = app.clone();
        Timer::single_shot(Duration::ZERO, move || {
            requests::handle(&app, first, true);
            refresh(&app);
            app.ready.set(true);
        });
    }
    // 主窗口不一定显示：从右键菜单来的操作只弹各自的进度窗口（像 7-Zip）。
    // 所有窗口都关了、也没有在跑的任务时由 [`quit_if_idle`] 退出。
    slint::run_event_loop_until_quit()?;
    Ok(())
}

/// 没有任何窗口、也没有在跑或排队的任务时退出。
/// （暂停的解压已把进度存在磁盘上，退出不丢。）
pub(super) fn quit_if_idle(app: &App) {
    if !app.ready.get() {
        return;
    }
    let any_window = app.window.window().is_visible()
        || !app.progress.borrow().is_empty()
        || app.compress.borrow().is_some()
        || app.password.borrow().is_some()
        || app.hashes.borrow().iter().any(|h| h.window().is_visible())
        || app.settings_window.borrow().is_some()
        || update::has_window(app)
        || !app.confirms.borrow().is_empty()
        || app.same_names.borrow().is_some()
        || !app.browsers.borrow().is_empty();
    let busy = app
        .tasks
        .snapshot()
        .iter()
        .any(|t| matches!(t.status, Status::Running | Status::Queued));
    if !any_window && !busy {
        let _ = slint::quit_event_loop();
    }
}

/// 显示一个对话框：主窗口开着时归属主窗口（始终在它上面），否则单独显示。
pub(super) fn show_dialog<C: ComponentHandle + Themed + 'static>(app: &App, dialog: &C) {
    dialog.apply(app.theme());
    if app.window.window().is_visible() {
        win::present_owned(dialog, &app.window);
    } else {
        win::present(dialog);
    }
}

pub(super) fn refresh(app: &Rc<App>) {
    let snapshot = app.tasks.snapshot();
    // 像 7-Zip 一样：发现需要密码就直接弹出密码框，不用再去任务列表里找按钮。
    let waiting = snapshot
        .iter()
        .find(|t| matches!(t.status, Status::NeedsPassword { .. }) && !app.prompted.borrow().contains(&t.id))
        .map(|t| t.id);
    app.prompted.borrow_mut().retain(|id| {
        snapshot
            .iter()
            .any(|t| t.id == *id && matches!(t.status, Status::NeedsPassword { .. }))
    });
    if let Some(id) = waiting
        && app.password.borrow().is_none()
    {
        app.prompted.borrow_mut().insert(id);
        password::ask_password(app, id);
    }
    // 设置为"每次询问"时遇到同名文件：弹窗问怎么办。
    same_names::prompt_next(app);
    // 校验值算完就弹出结果窗口（像 7-Zip 一样）。
    let finished: Vec<u64> = snapshot
        .iter()
        .filter(|t| t.report.is_some() && !app.shown.borrow().contains(&t.id))
        .map(|t| t.id)
        .collect();
    for id in finished {
        app.shown.borrow_mut().insert(id);
        hash::show_hash(app, id);
    }
    progress::update_progress(app, &snapshot);
    let rows: Vec<TaskRow> = snapshot
        .iter()
        .map(|t| TaskRow {
            id: t.id as i32,
            title: t.title.clone().into(),
            detail: t.detail().into(),
            progress: t.progress(),
            state: t.state_text().into(),
            can_pause: t.can_pause(),
            can_resume: t.can_resume(),
            can_cancel: t.can_cancel(),
            can_remove: t.can_remove(),
            can_open: t.status == Status::Done && (t.output.is_some() || t.report.is_some()),
            open_label: if t.report.is_some() { "查看" } else { "打开" }.into(),
            needs_password: matches!(t.status, Status::NeedsPassword { .. }),
        })
        .collect();
    app.window
        .set_tasks(ModelRc::new(VecModel::from(rows)));
}
