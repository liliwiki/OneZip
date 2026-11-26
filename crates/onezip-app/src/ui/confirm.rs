//! "边解压边删除压缩包"开始前的说明。设置里关掉"边解压边删除前确认"、或在对话框里
//! 勾了"以后不再提醒"后就不再出现，直接开始。

use onezip_i18n::t;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;

use super::settings::commit;
use super::{App, show_dialog};
use crate::ConfirmDialog;
use crate::tasks::file_name;

type Go = Box<dyn FnOnce(&Rc<App>)>;

/// 需要时先说明，用户点"开始"后才调用 `go`；点"取消"或关掉就什么也不做。
pub(super) fn in_place(app: &Rc<App>, paths: &[PathBuf], go: impl FnOnce(&Rc<App>) + 'static) {
    if !app.settings.borrow().confirm_in_place {
        go(app);
        return;
    }
    let Ok(d) = ConfirmDialog::new() else { return };
    d.set_heading(
        match paths {
            [one] => t!("{} 会在解压过程中逐步删除", file_name(one)),
            _ => t!("这 {} 个压缩包会在解压过程中逐步删除", paths.len()),
        }
        .into(),
    );
    d.set_detail(t!("解压完成后压缩包不再存在，也不会进回收站。").into());

    let go: Rc<RefCell<Option<Go>>> = Rc::new(RefCell::new(Some(Box::new(go))));
    d.on_start({
        let app = Rc::downgrade(app);
        let w = d.as_weak();
        let go = go.clone();
        move || {
            let (Some(app), Some(d)) = (app.upgrade(), w.upgrade()) else { return };
            if d.get_never_again() {
                let mut s = app.settings.borrow().clone();
                s.confirm_in_place = false;
                commit(&app, s);
                if let Some(sw) = app.settings_window.borrow().as_ref() {
                    sw.set_confirm_in_place(false);
                }
            }
            close(&app, &d);
            if let Some(go) = go.borrow_mut().take() {
                go(&app);
            }
            super::quit_if_idle(&app);
        }
    });
    d.on_cancel({
        let app = Rc::downgrade(app);
        let w = d.as_weak();
        move || {
            if let (Some(app), Some(d)) = (app.upgrade(), w.upgrade()) {
                close(&app, &d);
                super::quit_if_idle(&app);
            }
        }
    });
    d.window().on_close_requested({
        let app = Rc::downgrade(app);
        let w = d.as_weak();
        move || {
            if let (Some(app), Some(d)) = (app.upgrade(), w.upgrade()) {
                forget(&app, &d);
            }
            slint::CloseRequestResponse::HideWindow
        }
    });
    show_dialog(app, &d);
    app.confirms.borrow_mut().push(d);
}

fn forget(app: &App, d: &ConfirmDialog) {
    app.confirms.borrow_mut().retain(|c| !std::ptr::eq(c.window(), d.window()));
}

fn close(app: &App, d: &ConfirmDialog) {
    let _ = d.hide();
    forget(app, d);
}
