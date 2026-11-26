//! 把一个请求（右键菜单、命令行、另一个实例、主窗口按钮）变成任务。

use std::path::{Path, PathBuf};
use std::rc::Rc;

use onezip_core::{CompressRequest, Conflict, ExtractRequest, Placement, Source};
use onezip_settings::Settings;
use slint::ComponentHandle;

use super::common::{file_dialog, stem, unique};
use super::{App, compress, confirm, progress, settings};
use crate::request::{Action, Delete, Request};
use crate::tasks::{Kind, file_name};
use crate::win;

/// 处理一个请求：来自命令行、右键菜单、或窗口按钮。
pub(super) fn handle_from_window(app: &Rc<App>, req: Request) {
    handle(app, req, false);
}

/// 处理一个请求。`external`：来自命令行 / 右键菜单 / 另一个实例——这时不弹主窗口，
/// 每个任务弹一个自己的进度窗口；来自主窗口的按钮时任务只显示在主窗口的列表里。
pub(super) fn handle(app: &Rc<App>, req: Request, external: bool) {
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
            delete,
        } => {
            let paths = req.paths;
            if delete == Delete::InPlace {
                let shown = paths.clone();
                confirm::in_place(app, &shown, move |app| extract(app, paths, placement, ask_folder, delete, external));
            } else {
                extract(app, paths, placement, ask_folder, delete, external);
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
            // 放到设置里的"默认保存位置"（没设就是"下载"文件夹），不再每次问。
            let s = app.settings.borrow().clone();
            let Some(dir) = s.download_dir.clone().or_else(|| {
                std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Downloads"))
            }) else {
                return;
            };
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
                nested: s.nested,
                conflict: conflict(&s),
                connections: s.connections,
                ..Default::default()
            };
            ids.push(app.tasks.add(format!("下载并解压 {name}"), Kind::Extract(r, None)));
        }
        Action::Compress(Some(format)) => {
            if req.paths.is_empty() {
                return;
            }
            let Some(out) = onezip_core::compress::default_output(&req.paths, format) else {
                return;
            };
            let out = unique(out);
            let mut r = CompressRequest::new(req.paths, out.clone(), format);
            compress_defaults(&mut r, &app.settings.borrow());
            ids.push(
                app.tasks
                    .add(format!("压缩为 {}", file_name(&out)), Kind::Compress(r)),
            );
        }
        Action::Compress(None) => {
            if !req.paths.is_empty() {
                compress::open_compress_dialog(app, req.paths, external);
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
        // 不带 /quiet 的 --register-menu：打开设置的"右键菜单"页。
        Action::RegisterMenu | Action::UnregisterMenu => settings::open(app, Some(3)),
    }
    if external {
        for id in ids {
            progress::open_progress(app, id);
        }
    }
}

/// 把要解压的压缩包排进任务（边解边删时已经确认过）。
fn extract(app: &Rc<App>, paths: Vec<PathBuf>, placement: Placement, ask_folder: bool, delete: Delete, external: bool) {
    let w = &app.window;
    let output_root = if ask_folder {
        let start = paths.first().and_then(|p| p.parent()).map(Path::to_path_buf);
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
    let s = app.settings.borrow().clone();
    let in_place = delete == Delete::InPlace;
    let mut ids = Vec::new();
    for path in paths {
        let output = output_root.as_ref().map(|root| {
            // 多个压缩包解到同一处时各自一个子文件夹，免得混在一起。
            if placement == Placement::Smart { root.join(stem(&path)) } else { root.clone() }
        });
        let r = ExtractRequest {
            source: Source::Local(path.clone()),
            output,
            placement,
            keep_source: !in_place,
            verify_first: in_place,
            keep_free: 512 << 20,
            nested: s.nested,
            conflict: conflict(&s),
            ..Default::default()
        };
        let (title, after) = match delete {
            Delete::Keep => (format!("解压 {}", file_name(&path)), None),
            Delete::InPlace => (format!("解压并释放空间 {}", file_name(&path)), None),
            Delete::After => (format!("解压后删除 {}", file_name(&path)), Some(s.delete_mode)),
        };
        ids.push(app.tasks.add(title, Kind::Extract(r, after)));
    }
    if external {
        for id in ids {
            progress::open_progress(app, id);
        }
    }
}

/// 设置里的"遇到同名文件"。
pub(super) fn conflict(s: &Settings) -> Conflict {
    match s.conflict {
        onezip_settings::Conflict::Replace => Conflict::Replace,
        onezip_settings::Conflict::Skip => Conflict::Skip,
        onezip_settings::Conflict::Rename => Conflict::Rename,
    }
}

/// 设置里的压缩默认值（级别、线程、固实、加密文件名）。
pub(super) fn compress_defaults(r: &mut CompressRequest, s: &Settings) {
    r.level = s.level;
    r.threads = s.threads;
    r.solid = s.solid;
    r.encrypt_names = s.encrypt_names;
}

/// 设置里的"默认解压方式"。
pub(super) fn placement(s: &Settings) -> Placement {
    match s.extract_mode {
        onezip_settings::ExtractMode::Smart => Placement::Smart,
        onezip_settings::ExtractMode::NewFolder => Placement::NewFolder,
        onezip_settings::ExtractMode::Here => Placement::Here,
    }
}
