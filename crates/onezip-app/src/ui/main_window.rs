//! 主窗口：按钮、勾选项、关闭时的处理。

use std::rc::Rc;

use rfd::{MessageButtons, MessageDialogResult, MessageLevel};
use slint::{ComponentHandle, SharedString};

use super::common::{file_dialog, message, reveal};
use super::{App, compress, hash, password, requests, settings};
use crate::menu;
use crate::request::{Action, Delete, Request};

pub(super) fn status_text(s: &onezip_settings::Settings) -> String {
    if !s.menu_enabled {
        "右键菜单已关闭：可以在“设置 → 右键菜单”里打开。".into()
    } else if menu::shell_package_installed() {
        "右键菜单已启用（Windows 11 第一层菜单）。".into()
    } else if menu::is_registered() {
        "右键菜单已启用。".into()
    } else {
        "右键菜单尚未启用：在“设置 → 右键菜单”里打开。".into()
    }
}

pub(super) fn wire(app: &Rc<App>) {
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
                hash::show_hash(&app, id as u64);
            } else if let Some(p) = task.output {
                reveal(&p);
            }
        }
    });
    w.on_password({
        let app = Rc::downgrade(app);
        move |id| {
            if let Some(app) = app.upgrade() {
                password::ask_password(&app, id as u64);
            }
        }
    });
    w.on_extract_files({
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            let Some(files) = file_dialog(app.window.window())
                .set_title("选择要解压的压缩包")
                .add_filter("压缩包", &onezip_formats::archive_extensions().collect::<Vec<_>>())
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
                        .set_title("边解压边删除压缩包")
                        .set_level(MessageLevel::Warning)
                        .set_description(format!(
                            "这些文件的格式（磁盘镜像、安装包等）不能边解压边删除，\
                             请取消勾选“解压时释放原压缩包空间”后再解压：\n\n{}",
                            no.join("\n")
                        ))
                        .show();
                    return;
                }
            }
            requests::handle_from_window(
                &app,
                Request {
                    action: Action::Extract {
                        placement: requests::placement(&app.settings.borrow()),
                        ask_folder: false,
                        delete: if reclaim { Delete::InPlace } else { Delete::Keep },
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
            requests::handle_from_window(
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
                compress::open_compress_dialog(&app, files, false);
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
                compress::open_compress_dialog(&app, vec![dir], false);
            }
        }
    });
    w.on_open_settings({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                settings::open(&app, None);
            }
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
