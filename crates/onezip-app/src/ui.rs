//! 主窗口、压缩对话框、密码对话框和任务队列之间的胶水。

use std::cell::RefCell;
use std::collections::HashSet;
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
use crate::{CompressDialog, MainWindow, PasswordDialog, TaskRow, menu, win};

struct App {
    window: MainWindow,
    tasks: Arc<Manager>,
    /// 打开中的对话框，防止被提前释放。
    compress: RefCell<Option<CompressDialog>>,
    password: RefCell<Option<PasswordDialog>>,
    /// 已经自动弹过密码框的任务（输入后又报密码错误时会移出，再弹一次）。
    prompted: RefCell<HashSet<u64>>,
}

pub fn run(first: Request, incoming: Receiver<Request>) -> anyhow::Result<()> {
    let window = MainWindow::new()?;
    let app = Rc::new(App {
        window,
        tasks: Manager::new(),
        compress: RefCell::new(None),
        password: RefCell::new(None),
        prompted: RefCell::new(HashSet::new()),
    });
    wire(&app);
    app.window.set_status(status_text().into());

    // 定时：收取其它实例转来的请求，刷新任务列表。
    let timer = Timer::default();
    {
        let app = app.clone();
        timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
            while let Ok(req) = incoming.try_recv() {
                handle(&app, req);
            }
            refresh(&app);
        });
    }

    // 事件循环开始后再处理第一个请求：那时窗口已创建，才能提到最前、给对话框指定归属。
    {
        let app = app.clone();
        Timer::single_shot(Duration::ZERO, move || {
            handle(&app, first);
            refresh(&app);
        });
    }
    app.window.run()?;
    Ok(())
}

/// 归属 `parent` 的提示框：始终在它上面，不会跑到别的窗口后面让人以为程序卡死。
fn message(parent: &slint::Window) -> MessageDialog {
    MessageDialog::new()
        .set_title("OneZip")
        .set_parent(&parent.window_handle())
}

fn file_dialog(parent: &slint::Window) -> FileDialog {
    FileDialog::new().set_parent(&parent.window_handle())
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
            can_open: t.status == Status::Done && t.output.is_some(),
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
        let t = tasks.clone();
        move |id| {
            if let Some(p) = t.get(id as u64).and_then(|t| t.output) {
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
            if reclaim && !confirm_reclaim(app.window.window(), files.len()) {
                return;
            }
            handle(
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
            handle(
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
                open_compress_dialog(&app, files);
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
                open_compress_dialog(&app, vec![dir]);
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
        move || {
            if tasks.busy() {
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
                for _ in 0..100 {
                    if !tasks.snapshot().iter().any(|t| t.is_running()) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
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
fn handle(app: &Rc<App>, req: Request) {
    let w = &app.window;
    // 先把主窗口亮出来、提到最前，后面的对话框都归属它。
    win::present(w);
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
                app.tasks
                    .add(format!("{verb} {}", file_name(&path)), Kind::Extract(r));
            }
        }
        Action::Test => {
            for path in req.paths {
                app.tasks.add(
                    format!("测试 {}", file_name(&path)),
                    Kind::Test {
                        path,
                        password: None,
                    },
                );
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
            app.tasks.add(format!("下载并解压 {name}"), Kind::Extract(r));
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
            app.tasks
                .add(format!("压缩为 {}", file_name(&out)), Kind::Compress(r));
        }
        Action::Compress(None) => {
            if !req.paths.is_empty() {
                open_compress_dialog(app, req.paths);
            }
        }
        Action::RegisterMenu | Action::UnregisterMenu => toggle_menu(w.window()),
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

fn open_compress_dialog(app: &Rc<App>, inputs: Vec<PathBuf>) {
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
            app.tasks
                .add(format!("压缩为 {}", file_name(&output)), Kind::Compress(r));
        }
    });
    // 之前打开、还没处理的对话框先关掉，免得藏在后面。
    if let Some(old) = app.compress.borrow_mut().take() {
        let _ = old.hide();
    }
    win::present_owned(&d, &app.window);
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
    win::present_owned(&d, &app.window);
    *app.password.borrow_mut() = Some(d);
}
