//! 右键菜单发起的任务各自的进度窗口（像 7-Zip 那样），做完自动关闭。

use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, SharedString};

use super::{App, Themed};
use crate::tasks::{Kind, Status};
use crate::{ProgressDialog, win};

/// 给右键菜单发起的任务弹一个进度窗口。
pub(super) fn open_progress(app: &Rc<App>, id: u64) {
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
    d.apply(app.theme());
    win::present(&d);
    app.progress.borrow_mut().insert(id, d);
}

pub(super) fn close_progress(app: &App, id: u64) {
    if let Some(d) = app.progress.borrow_mut().remove(&id) {
        let _ = d.hide();
    }
}

/// 00:01:23
pub(super) fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// 刷新各进度窗口；做完的自动关掉（失败和测试结果留着给人看）。
pub(super) fn update_progress(app: &App, snapshot: &[crate::tasks::Task]) {
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
            Status::SameNames { .. } => ("目标位置有同名文件：请在弹出的窗口里选择怎么办。".into(), false, true),
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
