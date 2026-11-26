//! 设置窗口：主窗口点"设置"弹出。每改一项就整体保存并立即生效（主题、右键菜单）。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use onezip_core::Format;
use onezip_menu::{Command, Show};
use onezip_settings::assoc;
use onezip_settings::passwords::{self, Saved};
use onezip_settings::{Conflict, DeleteMode, ExtractMode, Settings, Theme};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::common::{error, file_dialog, open_url};
use super::{App, show_dialog};
use crate::{AssocRow, MenuRow, MenuToggle, SavedPassword, SettingsWindow, menu};

pub(super) const GITHUB: &str = "https://github.com/liliwiki/OneZip";

/// 压缩级别下拉框的各项对应的级别。
const LEVELS: [u32; 6] = [0, 1, 3, 5, 7, 9];

pub(super) fn theme_index(t: Theme) -> i32 {
    match t {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    }
}

/// 线程数下拉框：自动，然后 1、2、4…直到 CPU 核数。
fn thread_choices() -> (Vec<String>, Vec<u32>) {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
    let mut values = vec![0];
    let mut n = 1;
    while n < cores {
        values.push(n);
        n *= 2;
    }
    values.push(cores);
    let labels = values
        .iter()
        .map(|&v| if v == 0 { format!("自动（{cores}）") } else { v.to_string() })
        .collect();
    (labels, values)
}

/// 右键菜单页的两组勾选项：右键压缩包时 / 右键任意文件时。
fn menu_rows(s: &Settings, archive: bool) -> ModelRc<MenuRow> {
    let toggles: Vec<MenuToggle> = Command::all()
        .filter(|c| (c.show() != Show::Always) == archive)
        .map(|c| MenuToggle {
            id: c.id().into(),
            label: c.label().into(),
            checked: !s.menu_hidden.iter().any(|h| h == c.id()),
        })
        .collect();
    let rows: Vec<MenuRow> = toggles
        .chunks(2)
        .map(|pair| MenuRow {
            a: pair[0].clone(),
            b: pair.get(1).cloned().unwrap_or_default(),
            has_b: pair.len() > 1,
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

fn download_placeholder() -> String {
    std::env::var_os("USERPROFILE")
        .map(|h| PathBuf::from(h).join("Downloads").display().to_string())
        .unwrap_or_default()
}

/// 密码页：重新读出保存的密码放进列表。`shown` 为假时不把密码本身交给界面。
fn fill_passwords(d: &SettingsWindow, list: &RefCell<Vec<Saved>>, shown: bool) {
    let saved = passwords::load();
    let rows: Vec<SavedPassword> = saved
        .iter()
        .map(|s| SavedPassword {
            text: if shown { s.password.as_str().into() } else { SharedString::new() },
            note: if s.note.is_empty() { "手动添加".into() } else { format!("上次用于 {}", s.note).into() },
        })
        .collect();
    d.set_saved_passwords(ModelRc::new(VecModel::from(rows)));
    d.set_passwords_shown(shown && !saved.is_empty());
    *list.borrow_mut() = saved;
}

/// 密码页的按钮。`list` 是界面上显示的那份（按序号删除时用它，免得期间有新保存的密码时删错）。
fn wire_passwords(d: &SettingsWindow) {
    let list = Rc::new(RefCell::new(Vec::new()));
    fill_passwords(d, &list, false);
    let failed = |d: &SettingsWindow, e: std::io::Error| error(d.window(), &format!("保存密码失败：{e}"));
    d.on_add_password({
        let (w, list) = (d.as_weak(), list.clone());
        move || {
            let Some(d) = w.upgrade() else { return };
            let pw = d.get_new_password().to_string();
            if pw.is_empty() {
                return;
            }
            if let Err(e) = passwords::remember(&pw, "") {
                failed(&d, e);
            }
            d.set_new_password(SharedString::new());
            fill_passwords(&d, &list, d.get_passwords_shown());
        }
    });
    d.on_remove_password({
        let (w, list) = (d.as_weak(), list.clone());
        move |i| {
            let Some(d) = w.upgrade() else { return };
            let pw = list.borrow().get(i as usize).map(|s| s.password.clone());
            if let Some(pw) = pw
                && let Err(e) = passwords::forget(&pw)
            {
                failed(&d, e);
            }
            fill_passwords(&d, &list, d.get_passwords_shown());
        }
    });
    d.on_remove_all_passwords({
        let (w, list) = (d.as_weak(), list.clone());
        move || {
            let Some(d) = w.upgrade() else { return };
            let r = super::common::message(d.window())
                .set_description("删除全部保存的密码？删除后不能恢复。")
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show();
            if r != rfd::MessageDialogResult::Ok {
                return;
            }
            if let Err(e) = passwords::save(&[]) {
                failed(&d, e);
            }
            fill_passwords(&d, &list, false);
        }
    });
    d.on_toggle_passwords({
        let (w, list) = (d.as_weak(), list.clone());
        move || {
            let Some(d) = w.upgrade() else { return };
            if d.get_passwords_shown() {
                fill_passwords(&d, &list, false);
                return;
            }
            d.set_verifying(true);
            let w = w.clone();
            crate::verify::verify_user(d.window(), "验证你的身份以显示 OneZip 保存的密码", move |ok| {
                let Some(d) = w.upgrade() else { return };
                d.set_verifying(false);
                if ok {
                    d.invoke_passwords_verified();
                }
            });
        }
    });
    d.on_passwords_verified({
        let (w, list) = (d.as_weak(), list.clone());
        move || {
            if let Some(d) = w.upgrade() {
                fill_passwords(&d, &list, true);
            }
        }
    });
}

/// 文件关联页：按注册表里的实际情况勾选。
fn fill_assoc(d: &SettingsWindow) {
    let rows = |exts: &[&str]| -> ModelRc<AssocRow> {
        let rows: Vec<AssocRow> = exts
            .chunks(4)
            .map(|chunk| {
                let items: Vec<MenuToggle> = chunk
                    .iter()
                    .map(|e| MenuToggle { id: (*e).into(), label: format!(".{e}").into(), checked: assoc::associated(e) })
                    .collect();
                AssocRow { items: ModelRc::new(VecModel::from(items)) }
            })
            .collect();
        ModelRc::new(VecModel::from(rows))
    };
    d.set_assoc_archives(rows(assoc::ARCHIVES));
    d.set_assoc_images(rows(assoc::IMAGES));
    let overridden: Vec<String> = assoc::ARCHIVES
        .iter()
        .chain(assoc::IMAGES)
        .filter(|e| assoc::overridden(e))
        .map(|e| format!(".{e}"))
        .collect();
    d.set_assoc_overridden(overridden.join("、").into());
}

/// 关联或取消一批扩展名，然后刷新页面。
fn set_assoc(d: &SettingsWindow, exts: &[&str], on: bool) {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return error(d.window(), &format!("找不到 OneZip.exe：{e}")),
    };
    let failed: Vec<String> = exts
        .iter()
        .filter_map(|e| assoc::set(&exe, e, on).err().map(|err| format!(".{e}：{err}")))
        .collect();
    assoc::notify();
    if !failed.is_empty() {
        error(d.window(), &format!("设置文件关联失败：\n{}", failed.join("\n")));
    }
    fill_assoc(d);
}

fn wire_assoc(d: &SettingsWindow) {
    fill_assoc(d);
    let w = d.as_weak();
    d.on_assoc_toggled(move |id, on| {
        if let Some(d) = w.upgrade() {
            set_assoc(&d, &[id.as_str()], on);
        }
    });
    let w = d.as_weak();
    d.on_assoc_common(move || {
        if let Some(d) = w.upgrade() {
            set_assoc(&d, assoc::COMMON, true);
        }
    });
    let w = d.as_weak();
    d.on_assoc_none(move || {
        if let Some(d) = w.upgrade() {
            let all: Vec<&str> = assoc::ARCHIVES.iter().chain(assoc::IMAGES).copied().collect();
            set_assoc(&d, &all, false);
        }
    });
    let w = d.as_weak();
    d.on_assoc_refresh(move || {
        if let Some(d) = w.upgrade() {
            fill_assoc(&d);
        }
    });
    d.on_open_default_apps(|| open_url(&format!("ms-settings:defaultapps?registeredAppUser={}", assoc::APP)));
}

/// 设置 → 窗口。
fn fill(d: &SettingsWindow, s: &Settings) {
    d.set_theme_index(theme_index(s.theme));
    super::Themed::apply(d, theme_index(s.theme));

    d.set_extract_mode(match s.extract_mode {
        ExtractMode::Smart => 0,
        ExtractMode::NewFolder => 1,
        ExtractMode::Here => 2,
    });
    d.set_conflict(match s.conflict {
        Conflict::Ask => 0,
        Conflict::Replace => 1,
        Conflict::Skip => 2,
        Conflict::Rename => 3,
    });
    d.set_nested(s.nested);
    d.set_delete_mode(match s.delete_mode {
        DeleteMode::RecycleBin => 0,
        DeleteMode::Permanent => 1,
    });
    d.set_confirm_in_place(s.confirm_in_place);
    d.set_download_dir(s.download_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into());
    d.set_download_placeholder(download_placeholder().into());
    d.set_connections(s.connections as i32);

    let formats: Vec<SharedString> = Format::ALL.iter().map(|f| SharedString::from(f.extension())).collect();
    d.set_formats(ModelRc::new(VecModel::from(formats)));
    d.set_format_index(Format::ALL.iter().position(|f| f.extension() == s.format).unwrap_or(0) as i32);
    let level = LEVELS.iter().enumerate().min_by_key(|(_, l)| l.abs_diff(s.level)).map_or(3, |(i, _)| i);
    d.set_level_index(level as i32);
    let (labels, values) = thread_choices();
    let labels: Vec<SharedString> = labels.into_iter().map(Into::into).collect();
    d.set_thread_choices(ModelRc::new(VecModel::from(labels)));
    d.set_threads_index(values.iter().position(|&v| v == s.threads).unwrap_or(0) as i32);
    d.set_solid(s.solid);
    d.set_encrypt_names(s.encrypt_names);

    d.set_menu_enabled(s.menu_enabled);
    d.set_archive_items(menu_rows(s, true));
    d.set_file_items(menu_rows(s, false));

    d.set_version(env!("CARGO_PKG_VERSION").into());
    d.set_auto_update(s.auto_update);
    d.set_skipped(s.skipped.join("、").into());
}

/// 窗口 → 设置（只覆盖窗口里有的项）。
fn read(d: &SettingsWindow, s: &mut Settings) {
    s.theme = match d.get_theme_index() {
        1 => Theme::Light,
        2 => Theme::Dark,
        _ => Theme::System,
    };
    s.extract_mode = match d.get_extract_mode() {
        1 => ExtractMode::NewFolder,
        2 => ExtractMode::Here,
        _ => ExtractMode::Smart,
    };
    s.conflict = match d.get_conflict() {
        0 => Conflict::Ask,
        1 => Conflict::Replace,
        2 => Conflict::Skip,
        _ => Conflict::Rename,
    };
    s.nested = d.get_nested();
    s.delete_mode = if d.get_delete_mode() == 1 { DeleteMode::Permanent } else { DeleteMode::RecycleBin };
    s.confirm_in_place = d.get_confirm_in_place();
    let dir = d.get_download_dir().trim().to_string();
    s.download_dir = (!dir.is_empty()).then(|| PathBuf::from(dir));
    s.connections = d.get_connections().clamp(1, 32) as u32;
    s.format = Format::ALL
        .get(d.get_format_index().max(0) as usize)
        .map_or("7z", |f| f.extension())
        .to_string();
    s.level = LEVELS[d.get_level_index().clamp(0, 5) as usize];
    s.threads = thread_choices().1.get(d.get_threads_index().max(0) as usize).copied().unwrap_or(0);
    s.solid = d.get_solid();
    s.encrypt_names = d.get_encrypt_names();
    s.menu_enabled = d.get_menu_enabled();
    s.auto_update = d.get_auto_update();
}

/// 保存，并让主题、右键菜单这类马上看得到的设置生效。
pub(super) fn commit(app: &Rc<App>, new: Settings) {
    let old = app.settings.replace(new.clone());
    if let Err(e) = new.save() {
        crate::log::error(&format!("保存设置失败：{e}"));
    }
    if old.theme != new.theme {
        app.apply_theme();
    }
    if old.menu_enabled != new.menu_enabled || old.menu_hidden != new.menu_hidden {
        if let Err(e) = menu::apply(&new) {
            error(app.window.window(), &format!("设置右键菜单失败：{e:#}"));
        }
        app.window.set_status(super::main_window::status_text(&new).into());
    }
}

/// 打开设置窗口（已经开着就提到最前），`page` 是要显示的页（0 常规 … 3 右键菜单 …）。
pub(super) fn open(app: &Rc<App>, page: Option<i32>) {
    if let Some(d) = app.settings_window.borrow().as_ref() {
        if let Some(p) = page {
            d.set_page(p);
        }
        crate::win::present(d);
        return;
    }
    let Ok(d) = SettingsWindow::new() else { return };
    if let Some(p) = page {
        d.set_page(p);
    }
    fill(&d, &app.settings.borrow());
    wire_passwords(&d);
    wire_assoc(&d);

    d.on_changed({
        let app = Rc::downgrade(app);
        let w = d.as_weak();
        move || {
            let (Some(app), Some(d)) = (app.upgrade(), w.upgrade()) else { return };
            let mut s = app.settings.borrow().clone();
            read(&d, &mut s);
            commit(&app, s);
        }
    });
    d.on_menu_toggled({
        let app = Rc::downgrade(app);
        move |id, on| {
            let Some(app) = app.upgrade() else { return };
            let mut s = app.settings.borrow().clone();
            s.menu_hidden.retain(|h| h != id.as_str());
            if !on {
                s.menu_hidden.push(id.to_string());
            }
            commit(&app, s);
        }
    });
    d.on_reset_all({
        let app = Rc::downgrade(app);
        let w = d.as_weak();
        move || {
            let (Some(app), Some(d)) = (app.upgrade(), w.upgrade()) else { return };
            let r = super::common::message(d.window())
                .set_description("所有设置都恢复成刚安装时的样子？")
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show();
            if r != rfd::MessageDialogResult::Ok {
                return;
            }
            // 更新检查的时间不算"设置"，保留，免得一恢复就马上去查。
            let s = Settings {
                last_check: app.settings.borrow().last_check,
                ..Settings::default()
            };
            fill(&d, &s);
            commit(&app, s);
        }
    });
    d.on_reset_menu({
        let app = Rc::downgrade(app);
        let w = d.as_weak();
        move || {
            let (Some(app), Some(d)) = (app.upgrade(), w.upgrade()) else { return };
            let def = Settings::default();
            let s = Settings {
                menu_enabled: def.menu_enabled,
                menu_hidden: def.menu_hidden,
                ..app.settings.borrow().clone()
            };
            fill(&d, &s);
            commit(&app, s);
        }
    });
    d.on_browse_download({
        let w = d.as_weak();
        move || {
            let Some(d) = w.upgrade() else { return };
            let mut fd = file_dialog(d.window()).set_title("边下载边解压：默认保存位置");
            let current = d.get_download_dir().to_string();
            fd = fd.set_directory(if current.is_empty() { download_placeholder() } else { current });
            if let Some(dir) = fd.pick_folder() {
                d.set_download_dir(dir.display().to_string().into());
                d.invoke_changed();
            }
        }
    });
    d.on_check_update({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                super::update::check_now(&app);
            }
        }
    });
    d.on_clear_skipped({
        let app = Rc::downgrade(app);
        let w = d.as_weak();
        move || {
            let (Some(app), Some(d)) = (app.upgrade(), w.upgrade()) else { return };
            let mut s = app.settings.borrow().clone();
            s.skipped.clear();
            d.set_skipped(SharedString::new());
            commit(&app, s);
        }
    });
    d.on_open_releases(|| open_url(&format!("{GITHUB}/releases")));
    d.on_open_github(|| open_url(GITHUB));
    d.on_open_licenses(|| {
        // 安装目录里的 licenses 文件夹；从源码运行时没有，就打开网页。
        let local = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("licenses")));
        match local {
            Some(dir) if dir.is_dir() => open_url(&dir.display().to_string()),
            _ => open_url(&format!("{GITHUB}/blob/main/THIRD_PARTY_NOTICES.md")),
        }
    });
    d.on_report_issue(super::update::report_issue);
    d.window().on_close_requested({
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                app.settings_window.borrow_mut().take();
            }
            slint::CloseRequestResponse::HideWindow
        }
    });
    super::update::show_status(app, &d);
    show_dialog(app, &d);
    *app.settings_window.borrow_mut() = Some(d);
}
