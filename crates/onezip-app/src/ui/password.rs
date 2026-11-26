//! 需要密码时弹出的密码框。

use std::rc::Rc;

use slint::ComponentHandle;

use super::{App, show_dialog};
use crate::PasswordDialog;
use crate::tasks::Status;

pub(super) fn ask_password(app: &Rc<App>, id: u64) {
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
