//! 校验值结果窗口。

use onezip_i18n::t;
use std::rc::Rc;

use slint::ComponentHandle;

use super::{App, show_dialog};
use crate::{HashDialog, win};

/// 显示某个校验值任务的结果。
pub(super) fn show_hash(app: &Rc<App>, id: u64) {
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
                t!("已复制到剪贴板")
            } else {
                t!("复制失败，可以在上面选中文字后按 Ctrl+C")
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
