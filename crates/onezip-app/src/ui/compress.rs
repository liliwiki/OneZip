//! "添加到压缩包"对话框。

use onezip_i18n::t;
use std::path::PathBuf;
use std::rc::Rc;

use onezip_core::{CompressRequest, Format};
use rfd::{MessageButtons, MessageDialogResult};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::common::{file_dialog, message, unique};
use super::{App, progress, show_dialog};
use crate::CompressDialog;
use crate::tasks::{Kind, file_name};

pub(super) fn open_compress_dialog(app: &Rc<App>, inputs: Vec<PathBuf>, external: bool) {
    let Ok(d) = CompressDialog::new() else { return };
    let formats: Vec<SharedString> = Format::ALL
        .iter()
        .map(|f| SharedString::from(f.extension()))
        .collect();
    d.set_formats(ModelRc::new(VecModel::from(formats)));
    let label = match inputs.len() {
        1 => t!("压缩：{}", inputs[0].display()),
        n => t!("压缩 {} 项：{} 等", n, file_name(&inputs[0])),
    };
    d.set_inputs_label(label.into());
    // 设置里的默认格式、级别、固实
    let defaults = app.settings.borrow().clone();
    let index = Format::ALL.iter().position(|f| f.extension() == defaults.format).unwrap_or(0);
    let first = Format::ALL[index];
    d.set_format_index(index as i32);
    d.set_password_allowed(first.supports_password());
    d.set_level(defaults.level as f32);
    d.set_solid(defaults.solid);
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
            let mut fd = file_dialog(d.window()).set_title(t!("保存压缩包"));
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
            super::requests::compress_defaults(&mut r, &app.settings.borrow());
            r.level = d.get_level().round().clamp(0.0, 9.0) as u32;
            let pw = d.get_password().to_string();
            r.password = (!pw.is_empty() && format.supports_password()).then_some(pw);
            r.solid = d.get_solid();
            if output.exists() {
                let ok = message(d.window())
                    .set_description(t!("{} 已存在，要替换它吗？", output.display()))
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
                .add(t!("压缩为 {}", file_name(&output)), Kind::Compress(r));
            if external {
                progress::open_progress(&app, id);
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
