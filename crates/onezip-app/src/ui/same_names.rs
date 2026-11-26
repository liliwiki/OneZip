//! 设置为"每次询问"时，解压前发现目标位置已有同名文件：任务停下（什么都没动），
//! 弹窗列出这些文件，选"全部替换 / 全部跳过 / 都保留 / 取消"后按选择重新开始。

use std::rc::Rc;

use onezip_core::Conflict;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{App, show_dialog};
use crate::SameNamesDialog;
use crate::tasks::Status;

/// 列表里最多列这么多个，再多只说个数。
const SHOW_AT_MOST: usize = 500;

/// 有任务在等用户选、而且现在没有开着的询问窗口时，弹出一个。
pub(super) fn prompt_next(app: &Rc<App>) {
    if app.same_names.borrow().is_some() {
        return;
    }
    let Some(task) = app.tasks.snapshot().into_iter().find(|t| matches!(t.status, Status::SameNames { .. })) else {
        return;
    };
    let Status::SameNames { output, files } = &task.status else { return };
    let Ok(d) = SameNamesDialog::new() else { return };
    d.set_heading(format!("{} 里已有 {} 个同名文件", output.display(), files.len()).into());
    d.set_detail(
        format!(
            "解压 {} 会用到下面这些名字。什么都还没改动，请选择怎么办：",
            task.archive_name()
        )
        .into(),
    );
    let mut rows: Vec<SharedString> = files.iter().take(SHOW_AT_MOST).map(|f| f.display().to_string().into()).collect();
    if files.len() > SHOW_AT_MOST {
        rows.push(format!("……还有 {} 个", files.len() - SHOW_AT_MOST).into());
    }
    d.set_files(ModelRc::new(VecModel::from(rows)));

    let id = task.id;
    let choose = |choice: Option<Conflict>| {
        let app = Rc::downgrade(app);
        move || {
            let Some(app) = app.upgrade() else { return };
            if let Some((_, d)) = app.same_names.borrow_mut().take() {
                let _ = d.hide();
            }
            match choice {
                Some(c) => app.tasks.set_conflict(id, c),
                None => app.tasks.cancel(id),
            }
            super::refresh(&app);
            super::quit_if_idle(&app);
        }
    };
    d.on_replace(choose(Some(Conflict::Replace)));
    d.on_skip(choose(Some(Conflict::Skip)));
    d.on_rename(choose(Some(Conflict::Rename)));
    d.on_cancel(choose(None));
    let close = choose(None);
    d.window().on_close_requested(move || {
        close();
        slint::CloseRequestResponse::HideWindow
    });
    show_dialog(app, &d);
    *app.same_names.borrow_mut() = Some((id, d));
}
