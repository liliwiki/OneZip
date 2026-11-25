//! 主窗口、压缩对话框、密码对话框和任务队列之间的胶水。

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use onezip_core::{CompressRequest, ExtractRequest, Format, Placement, Source};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::request::{Action, Request};
use crate::tasks::{Kind, Manager, Status, file_name};
use crate::{
    CompressDialog, HashDialog, MainWindow, PasswordDialog, ProgressDialog, TaskRow, menu, win,
};

struct App {
    window: MainWindow,
    tasks: Arc<Manager>,
    /// 打开中的对话框，防止被提前释放。
    compress: RefCell<Option<CompressDialog>>,
    password: RefCell<Option<PasswordDialog>>,
    /// 已经自动弹过密码框的任务（输入后又报密码错误时会移出，再弹一次）。
    prompted: RefCell<HashSet<u64>>,
    /// 打开着的校验值结果窗口。
    hashes: RefCell<Vec<HashDialog>>,
    /// 已经自动弹过结果窗口的校验值任务。
    shown: RefCell<HashSet<u64>>,
    /// 右键菜单发起的任务各自的进度窗口。
    progress: RefCell<HashMap<u64, ProgressDialog>>,
    /// 第一个请求已经处理过（在那之前没有窗口是正常的，不能当成可以退出）。
    ready: Cell<bool>,
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
    });
    wire(&app);
    app.window.set_status(status_text().into());

    // 定时：收取其它实例转来的请求，刷新任务列表。
    let timer = Timer::default();
    {
        let app = app.clone();
        timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
            while let Ok(req) = incoming.try_recv() {
                handle(&app, req, true);
            }
            refresh(&app);
            quit_if_idle(&app);
        });
    }

    // 事件循环开始后再处理第一个请求：那时窗口已创建，才能提到最前、给对话框指定归属。
    {
        let app = app.clone();
        Timer::single_shot(Duration::ZERO, move || {
            handle(&app, first, true);
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
fn quit_if_idle(app: &App) {
    if !app.ready.get() {
        return;
    }
    let any_window = app.window.window().is_visible()
        || !app.progress.borrow().is_empty()
        || app.compress.borrow().is_some()
        || app.password.borrow().is_some()
        || app.hashes.borrow().iter().any(|h| h.window().is_visible());
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
fn show_dialog<C: ComponentHandle + 'static>(app: &App, dialog: &C) {
    if app.window.window().is_visible() {
        win::present_owned(dialog, &app.window);
    } else {
        win::present(dialog);
    }
}

/// 归属 `parent` 的提示框：始终在它上面，不会跑到别的窗口后面让人以为程序卡死。
/// 父窗口隐藏时（例如从右键菜单来、主窗口没开）就不指定父窗口，单独显示。
fn message(parent: &slint::Window) -> MessageDialog {
    let d = MessageDialog::new().set_title("OneZip");
    if parent.is_visible() {
        d.set_parent(&parent.window_handle())
    } else {
        d
    }
}

fn file_dialog(parent: &slint::Window) -> FileDialog {
    let d = FileDialog::new();
    if parent.is_visible() {
        d.set_parent(&parent.window_handle())
    } else {
        d
    }
}

fn status_text() -> String {
    if menu::shell_package_installed() {
        "右键菜单已启用（Windows 11 第一层菜单）。".into()
    } else if menu::is_registered() {
        "右键菜单已启用。".into()
    } else {
        "右键菜单尚未启用：点右上角“右键菜单…”启用。".into()
    }
}

fn refresh(app: &Rc<App>) {
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
        ask_password(app, id);
    }
    // 校验值算完就弹出结果窗口（像 7-Zip 一样）。
    let finished: Vec<u64> = snapshot
        .iter()
        .filter(|t| t.report.is_some() && !app.shown.borrow().contains(&t.id))
        .map(|t| t.id)
        .collect();
    for id in finished {
        app.shown.borrow_mut().insert(id);
        show_hash(app, id);
    }
    update_progress(app, &snapshot);
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

fn wire(app: &Rc<App>) {
    let w = &app.window;
    let tasks = app.tasks.clone();

    w.on_pause({
        let t = tasks.clone();
        move |id| t.pause(id as u64)
    });
    w.on_resume({
        let t = tasks.clone();
        move |id| t.resume(id as u64)
    });
    w.on_cancel({
        let t = tasks.clone();
        move |id| t.cancel(id as u64)
    });
    w.on_remove({
        let t = tasks.clone();
        move |id| t.remove(id as u64)
    });
    w.on_clear_finished({
        let t = tasks.clone();
        move || t.clear_finished()
    });
    w.on_open({
        let app = Rc::downgrade(app);
        move |id| {
            let Some(app) = app.upgrade() else { return };
            let Some(task) = app.tasks.get(id as u64) else { return };
            if task.report.is_some() {
                show_hash(&app, id as u64);
            } else if let Some(p) = task.output {
                reveal(&p);
            }
        }
    });
    w.on_password({
        let app = Rc::downgrade(app);
        move |id| {
            if let Some(app) = app.upgrade() {
                ask_password(&app, id as u64);
            }
        }
    });
    w.on_extract_files({
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            let Some(files) = file_dialog(app.window.window())
                .set_title("选择要解压的压缩包")
                .add_filter("压缩包", menu::ARCHIVE_EXTENSIONS)
                .add_filter("所有文件", &["*"])
                .pick_files()
            else {
                return;
            };
            let reclaim = app.window.get_reclaim();
            if reclaim {
                let no: Vec<String> = files
                    .iter()
                    .filter(|f| !onezip_core::can_extract_in_place(f))
                    .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .collect();
                if !no.is_empty() {
                    message(app.window.window())
                        .set_title("边解压边释放空间")
                        .set_level(MessageLevel::Warning)
                        .set_description(format!(
                            "这些文件的格式（磁盘镜像、安装包等）不能边解压边删除，\
                             请取消勾选“边解压边释放空间”后再解压：\n\n{}",
                            no.join("\n")
                        ))
                        .show();
                    return;
                }
            }
            if reclaim && !confirm_reclaim(app.window.window(), files.len()) {
                return;
            }
            handle_from_window(
                &app,
                Request {
                    action: Action::Extract {
                        placement: Placement::Smart,
                        ask_folder: false,
                        reclaim,
                    },
                    paths: files,
                },
            );
        }
    });
    w.on_extract_url({
        let app = Rc::downgrade(app);
        move |url| {
            let Some(app) = app.upgrade() else { return };
            let url = url.trim().to_string();
            if url.is_empty() {
                return;
            }
            handle_from_window(
                &app,
                Request {
                    action: Action::Url(url),
                    paths: Vec::new(),
                },
            );
            app.window.set_url(SharedString::new());
        }
    });
    w.on_compress_files({
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            if let Some(files) = file_dialog(app.window.window())
                .set_title("选择要压缩的文件")
                .pick_files() {
                open_compress_dialog(&app, files, false);
            }
        }
    });
    w.on_compress_folder({
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            if let Some(dir) = file_dialog(app.window.window())
                .set_title("选择要压缩的文件夹")
                .pick_folder() {
                open_compress_dialog(&app, vec![dir], false);
            }
        }
    });
    w.on_register_menu({
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            toggle_menu(app.window.window());
            app.window.set_status(status_text().into());
        }
    });

    // 关窗口时还有任务：解压暂停在安全点（进度已保存），压缩取消。
    w.window().on_close_requested({
        let tasks = tasks.clone();
        let weak = w.as_weak();
        let app_weak = Rc::downgrade(app);
        move || {
            // 右键菜单发起的任务各有进度窗口，关主窗口不影响它们。
            let has_progress = app_weak
                .upgrade()
                .is_some_and(|a| !a.progress.borrow().is_empty());
            if tasks.busy() && !has_progress {
                let Some(w) = weak.upgrade() else {
                    return slint::CloseRequestResponse::HideWindow;
                };
                let r = message(w.window())
                    .set_level(MessageLevel::Warning)
                    .set_description(
                        "还有任务在进行。退出后，解压任务会在安全点暂停并保存进度，下次可以继续；\
                         压缩任务会被取消。确定退出吗？",
                    )
                    .set_buttons(MessageButtons::OkCancel)
                    .show();
                if r != MessageDialogResult::Ok {
                    return slint::CloseRequestResponse::KeepWindowShown;
                }
                tasks.pause_all();
            }
            // 只把窗口藏起来：任务停稳、没有别的窗口后由 quit_if_idle 退出。
            // 以前在这里同步等待，窗口会“未响应”。
            slint::CloseRequestResponse::HideWindow
        }
    });
}

fn toggle_menu(parent: &slint::Window) {
    if menu::shell_package_installed() {
        // 第一层菜单由安装程序注册；再写传统菜单只会在"显示更多选项"里重复一份。
        message(parent)
            .set_title("右键菜单")
            .set_description(
                "Windows 11 第一层右键菜单已由安装程序启用。\n要移除它，请在“设置 → 应用 → 已安装的应用”里卸载 OneZip。",
            )
            .set_buttons(MessageButtons::Ok)
            .show();
        return;
    }
    let registered = menu::is_registered();
    let question = if registered {
        "右键菜单已启用。要移除它吗？"
    } else {
        "在资源管理器的右键菜单中加入 OneZip（只对当前用户生效，不需要管理员权限）？"
    };
    let r = message(parent)
        .set_title("右键菜单")
        .set_description(question)
        .set_buttons(MessageButtons::OkCancel)
        .show();
    if r != MessageDialogResult::Ok {
        return;
    }
    let result = if registered {
        menu::unregister();
        Ok(())
    } else {
        std::env::current_exe()
            .map_err(anyhow::Error::from)
            .and_then(|exe| menu::register(&exe))
    };
    if let Err(e) = result {
        error(parent, &format!("设置右键菜单失败：{e:#}"));
    }
}

fn confirm_reclaim(parent: &slint::Window, n: usize) -> bool {
    message(parent)
        .set_title("边解压边释放空间")
        .set_level(MessageLevel::Warning)
        .set_description(format!(
            "解压过程中会逐步删除这 {n} 个压缩包，完成后压缩包不再存在。\
             中途断电或关闭可以继续，但无法撤销。确定吗？"
        ))
        .set_buttons(MessageButtons::OkCancel)
        .show()
        == MessageDialogResult::Ok
}

fn error(parent: &slint::Window, msg: &str) {
    message(parent)
        .set_level(MessageLevel::Error)
        .set_description(msg)
        .set_buttons(MessageButtons::Ok)
        .show();
}

/// 处理一个请求：来自命令行、右键菜单、或窗口按钮。
fn handle_from_window(app: &Rc<App>, req: Request) {
    handle(app, req, false);
}

/// 处理一个请求。`external`：来自命令行 / 右键菜单 / 另一个实例——这时不弹主窗口，
/// 每个任务弹一个自己的进度窗口；来自主窗口的按钮时任务只显示在主窗口的列表里。
fn handle(app: &Rc<App>, req: Request, external: bool) {
    let w = &app.window;
    if !external || matches!(req.action, Action::Show | Action::RegisterMenu | Action::UnregisterMenu) {
        win::present(w);
    }
    let mut ids = Vec::new();
    match req.action {
        Action::Show => {}
        Action::Extract {
            placement,
            ask_folder,
            reclaim,
        } => {
            let output_root = if ask_folder {
                let start = req.paths.first().and_then(|p| p.parent()).map(Path::to_path_buf);
                let mut d = file_dialog(w.window()).set_title("解压到哪个文件夹");
                if let Some(s) = start {
                    d = d.set_directory(s);
                }
                match d.pick_folder() {
                    Some(f) => Some(f),
                    None => return,
                }
            } else {
                None
            };
            for path in req.paths {
                let output = output_root.as_ref().map(|root| {
                    // 多个压缩包解到同一处时各自一个子文件夹，免得混在一起。
                    if output_root.is_some() && placement == Placement::Smart {
                        root.join(stem(&path))
                    } else {
                        root.clone()
                    }
                });
                let r = ExtractRequest {
                    source: Source::Local(path.clone()),
                    output,
                    placement,
                    keep_source: !reclaim,
                    verify_first: reclaim,
                    keep_free: 512 << 20,
                    nested: w.get_nested(),
                    ..Default::default()
                };
                let verb = if reclaim { "解压并释放空间" } else { "解压" };
                ids.push(
                    app.tasks
                        .add(format!("{verb} {}", file_name(&path)), Kind::Extract(r)),
                );
            }
        }
        Action::Test => {
            for path in req.paths {
                ids.push(app.tasks.add(
                    format!("测试 {}", file_name(&path)),
                    Kind::Test {
                        path,
                        password: None,
                    },
                ));
            }
        }
        Action::Url(url) => {
            let downloads = std::env::var_os("USERPROFILE")
                .map(|h| PathBuf::from(h).join("Downloads"));
            let mut d = file_dialog(w.window()).set_title("下载并解压到哪个文件夹");
            if let Some(dl) = downloads {
                d = d.set_directory(dl);
            }
            let Some(dir) = d.pick_folder() else { return };
            let name = url
                .rsplit('/')
                .next()
                .and_then(|n| n.split(['?', '#']).next())
                .filter(|n| !n.is_empty())
                .unwrap_or("download")
                .to_string();
            let r = ExtractRequest {
                source: Source::Url(url.clone()),
                output: Some(dir.join(stem(Path::new(&name)))),
                ..Default::default()
            };
            ids.push(app.tasks.add(format!("下载并解压 {name}"), Kind::Extract(r)));
        }
        Action::Compress(Some(format)) => {
            if req.paths.is_empty() {
                return;
            }
            let Some(out) = onezip_core::compress::default_output(&req.paths, format) else {
                return;
            };
            let out = unique(out);
            let r = CompressRequest::new(req.paths, out.clone(), format);
            ids.push(
                app.tasks
                    .add(format!("压缩为 {}", file_name(&out)), Kind::Compress(r)),
            );
        }
        Action::Compress(None) => {
            if !req.paths.is_empty() {
                open_compress_dialog(app, req.paths, external);
            }
        }
        Action::Hash(kinds) => {
            if !req.paths.is_empty() {
                let name = match req.paths.as_slice() {
                    [one] => file_name(one),
                    [first, ..] => format!("{} 等 {} 项", file_name(first), req.paths.len()),
                    [] => String::new(),
                };
                let algos = kinds.iter().map(|k| k.name()).collect::<Vec<_>>().join(" ");
                ids.push(app.tasks.add(
                    format!("校验 {name}（{algos}）"),
                    Kind::Hash {
                        paths: req.paths,
                        kinds,
                    },
                ));
            }
        }
        Action::RegisterMenu | Action::UnregisterMenu => toggle_menu(w.window()),
    }
    if external {
        for id in ids {
            open_progress(app, id);
        }
    }
}

/// 去掉压缩扩展名后的名字，用作文件夹名。
fn stem(path: &Path) -> String {
    let name = file_name(path);
    let lower = name.to_ascii_lowercase();
    for ext in [
        ".tar.zst", ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.lz4", ".tgz", ".txz", ".tbz2",
        ".tzst", ".tar", ".zip", ".7z", ".rar", ".zst", ".gz", ".xz", ".bz2", ".lz4", ".001",
    ] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    // 7z.dll 的格式（iso、cab、wim……）
    if let Some((base, ext)) = name.rsplit_once('.')
        && !base.is_empty()
        && menu::ARCHIVE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
    {
        return base.to_string();
    }
    format!("{name}-extracted")
}

/// 已存在时在名字后面加 (2)、(3)…
fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let name = file_name(&path);
    let (base, ext) = Format::ALL
        .iter()
        .map(|f| format!(".{}", f.extension()))
        .find(|e| name.ends_with(e.as_str()))
        .map(|e| (name[..name.len() - e.len()].to_string(), e))
        .unwrap_or((name.clone(), String::new()));
    for i in 2.. {
        let p = path.with_file_name(format!("{base} ({i}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

fn reveal(path: &Path) {
    let mut cmd = std::process::Command::new("explorer.exe");
    if path.is_file() {
        cmd.arg(format!("/select,{}", path.display()));
    } else {
        cmd.arg(path);
    }
    let _ = cmd.spawn();
}

fn open_compress_dialog(app: &Rc<App>, inputs: Vec<PathBuf>, external: bool) {
    let Ok(d) = CompressDialog::new() else { return };
    let formats: Vec<SharedString> = Format::ALL
        .iter()
        .map(|f| SharedString::from(f.extension()))
        .collect();
    d.set_formats(ModelRc::new(VecModel::from(formats)));
    let label = match inputs.len() {
        1 => format!("压缩：{}", inputs[0].display()),
        n => format!("压缩 {n} 项：{} 等", file_name(&inputs[0])),
    };
    d.set_inputs_label(label.into());
    let first = Format::ALL[0];
    if let Some(out) = onezip_core::compress::default_output(&inputs, first) {
        d.set_output(unique(out).display().to_string().into());
    }

    let inputs = Rc::new(inputs);
    d.on_format_changed({
        let d = d.as_weak();
        move |i| {
            let Some(d) = d.upgrade() else { return };
            let f = Format::ALL[i as usize];
            d.set_password_allowed(f.supports_password());
            let out = d.get_output().to_string();
            let replaced = Format::ALL
                .iter()
                .map(|f| format!(".{}", f.extension()))
                .find(|e| out.ends_with(e.as_str()))
                .map(|e| format!("{}.{}", &out[..out.len() - e.len()], f.extension()));
            if let Some(r) = replaced {
                d.set_output(r.into());
            }
        }
    });
    d.on_browse({
        let d = d.as_weak();
        move || {
            let Some(d) = d.upgrade() else { return };
            let current = PathBuf::from(d.get_output().to_string());
            let mut fd = file_dialog(d.window()).set_title("保存压缩包");
            if let Some(dir) = current.parent() {
                fd = fd.set_directory(dir);
            }
            fd = fd.set_file_name(file_name(&current));
            if let Some(p) = fd.save_file() {
                d.set_output(p.display().to_string().into());
            }
        }
    });
    d.on_reject({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade()
                && let Some(d) = app.compress.borrow_mut().take()
            {
                let _ = d.hide();
            }
        }
    });
    d.on_accept({
        let app = Rc::downgrade(app);
        let inputs = inputs.clone();
        move || {
            let Some(app) = app.upgrade() else { return };
            let Some(d) = app.compress.borrow_mut().take() else {
                return;
            };
            let format = Format::ALL[d.get_format_index().clamp(0, 4) as usize];
            let mut output = PathBuf::from(d.get_output().to_string());
            let ext = format!(".{}", format.extension());
            if !file_name(&output).to_ascii_lowercase().ends_with(&ext) {
                output = PathBuf::from(format!("{}{ext}", output.display()));
            }
            let mut r = CompressRequest::new(inputs.to_vec(), output.clone(), format);
            r.level = d.get_level().round().clamp(0.0, 9.0) as u32;
            let pw = d.get_password().to_string();
            r.password = (!pw.is_empty() && format.supports_password()).then_some(pw);
            r.solid = d.get_solid();
            if output.exists() {
                let ok = message(d.window())
                    .set_description(format!("{} 已存在，要替换它吗？", output.display()))
                    .set_buttons(MessageButtons::OkCancel)
                    .show()
                    == MessageDialogResult::Ok;
                if !ok {
                    *app.compress.borrow_mut() = Some(d);
                    return;
                }
                r.overwrite = true;
            }
            let _ = d.hide();
            let id = app
                .tasks
                .add(format!("压缩为 {}", file_name(&output)), Kind::Compress(r));
            if external {
                open_progress(&app, id);
            }
        }
    });
    // 之前打开、还没处理的对话框先关掉，免得藏在后面。
    if let Some(old) = app.compress.borrow_mut().take() {
        let _ = old.hide();
    }
    show_dialog(app, &d);
    *app.compress.borrow_mut() = Some(d);
}

fn ask_password(app: &Rc<App>, id: u64) {
    let Some(task) = app.tasks.get(id) else { return };
    let Ok(d) = PasswordDialog::new() else { return };
    d.set_archive(task.archive_name().into());
    d.set_retry(matches!(task.status, Status::NeedsPassword { retry: true }));
    d.on_reject({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade()
                && let Some(d) = app.password.borrow_mut().take()
            {
                let _ = d.hide();
            }
        }
    });
    d.on_accept({
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            let Some(d) = app.password.borrow_mut().take() else {
                return;
            };
            let pw = d.get_password().to_string();
            let _ = d.hide();
            if !pw.is_empty() {
                app.tasks.set_password(id, pw);
            }
        }
    });
    if let Some(old) = app.password.borrow_mut().take() {
        let _ = old.hide();
    }
    show_dialog(app, &d);
    *app.password.borrow_mut() = Some(d);
}

/// 显示某个校验值任务的结果。
fn show_hash(app: &Rc<App>, id: u64) {
    let Some(text) = app.tasks.get(id).and_then(|t| t.report) else {
        return;
    };
    let Ok(d) = HashDialog::new() else { return };
    d.set_text(text.clone().into());
    d.on_copy({
        let d = d.as_weak();
        move || {
            let Some(d) = d.upgrade() else { return };
            let msg = if win::set_clipboard(&text) {
                "已复制到剪贴板"
            } else {
                "复制失败，可以在上面选中文字后按 Ctrl+C"
            };
            d.set_copied(msg.into());
        }
    });
    d.on_dismiss({
        let app = Rc::downgrade(app);
        let weak = d.as_weak();
        move || {
            let (Some(app), Some(d)) = (app.upgrade(), weak.upgrade()) else {
                return;
            };
            let _ = d.hide();
            app.hashes.borrow_mut().retain(|h| h.window().is_visible());
        }
    });
    show_dialog(app, &d);
    app.hashes.borrow_mut().push(d);
}

/// 给右键菜单发起的任务弹一个进度窗口。
fn open_progress(app: &Rc<App>, id: u64) {
    let Ok(d) = ProgressDialog::new() else { return };
    if let Some(t) = app.tasks.get(id) {
        d.set_heading(t.title.clone().into());
        d.set_caption(t.title.into());
    }
    d.set_progress(-1.0);
    let tasks = app.tasks.clone();
    d.on_pause({
        let t = tasks.clone();
        move || t.pause(id)
    });
    d.on_resume({
        let t = tasks.clone();
        move || t.resume(id)
    });
    d.on_cancel({
        let t = tasks.clone();
        move || t.cancel(id)
    });
    d.on_show_tasks({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                win::present(&app.window);
            }
        }
    });
    d.on_dismiss({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                close_progress(&app, id);
            }
        }
    });
    // 点右上角 ×：解压暂停在安全点（进度已保存，再点一次同样的菜单就能接着来），
    // 其它任务取消。
    d.window().on_close_requested({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                if let Some(t) = app.tasks.get(id) {
                    if t.can_pause() {
                        app.tasks.pause(id);
                    } else if t.can_cancel() {
                        app.tasks.cancel(id);
                    }
                }
                app.progress.borrow_mut().remove(&id);
            }
            slint::CloseRequestResponse::HideWindow
        }
    });
    win::present(&d);
    app.progress.borrow_mut().insert(id, d);
}

fn close_progress(app: &App, id: u64) {
    if let Some(d) = app.progress.borrow_mut().remove(&id) {
        let _ = d.hide();
    }
}

/// 00:01:23
fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// 刷新各进度窗口；做完的自动关掉（失败和测试结果留着给人看）。
fn update_progress(app: &App, snapshot: &[crate::tasks::Task]) {
    let mut done = Vec::new();
    for (&id, d) in app.progress.borrow().iter() {
        let Some(t) = snapshot.iter().find(|t| t.id == id) else {
            done.push(id);
            continue;
        };
        let p = t.progress();
        let pct = if p >= 0.0 {
            format!("{}% ", (p * 100.0).floor() as i32)
        } else {
            String::new()
        };
        let running = matches!(t.status, Status::Running | Status::Queued);
        d.set_caption(if running { format!("{pct}{}", t.title) } else { t.title.clone() }.into());
        d.set_progress(p);
        d.set_elapsed(clock(t.elapsed()).into());
        d.set_remaining(
            running
                .then(|| t.remaining().map(clock))
                .flatten()
                .unwrap_or_default()
                .into(),
        );
        d.set_speed(
            t.speed()
                .map(|s| format!("{}/s", onezip_core::human(s as u64)))
                .unwrap_or_default()
                .into(),
        );
        d.set_processed(if t.total > 0 {
            format!("{} / {}", onezip_core::human(t.processed()), onezip_core::human(t.total))
                .into()
        } else {
            SharedString::new()
        });
        d.set_extra(match (t.left, &t.remote) {
            (Some(left), _) => format!("原压缩包剩余 {}", onezip_core::human(left)).into(),
            (None, Some((_, out))) => format!("已解出 {}", onezip_core::human(*out)).into(),
            _ => SharedString::new(),
        });
        d.set_can_pause(t.can_pause());
        d.set_can_resume(t.status == Status::Paused);
        d.set_can_cancel(t.can_cancel());
        let (message, failed, finished) = match &t.status {
            Status::Done if matches!(t.kind, Kind::Test { .. }) => (t.detail(), false, true),
            // 解压、压缩做完就关；校验值会弹出结果窗口。
            Status::Done | Status::Cancelled => {
                done.push(id);
                continue;
            }
            Status::Failed(e) => (e.clone(), true, true),
            Status::Paused => (
                "已暂停，进度已保存。可以继续，或以后再从右键菜单解压同一个压缩包接着来。".into(),
                false,
                false,
            ),
            // 密码框会自动弹出；这里给个能关掉的出口。
            Status::NeedsPassword { .. } => ("需要密码：请在弹出的密码框里输入。".into(), false, true),
            _ => (String::new(), false, false),
        };
        d.set_message(message.into());
        d.set_failed(failed);
        d.set_finished(finished);
    }
    for id in done {
        close_progress(app, id);
    }
}
