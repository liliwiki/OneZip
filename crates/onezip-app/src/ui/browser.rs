//! 浏览压缩包的窗口（像 7-Zip 的文件管理器）。
//!
//! 打开时在后台用 7z.dll 列出全部项，按文件夹一层层显示。解压选中的项、全部解压都排进任务队列
//! （有进度窗口、同名文件和密码的处理与别处一样）；双击文件先解到临时文件夹再用默认程序打开；
//! 拖到资源管理器时先把选中的项解到临时文件夹，再把这些文件交给 Windows 拖放。
//!
//! 加密的压缩包先试保存的密码，都不对再弹密码框。

use onezip_i18n::t;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use anyhow::Result;
use onezip_core::browse::{self, Item, PickRequest};
use onezip_core::{Conflict, ExtractRequest, Placement, Source, human};
use onezip_settings::passwords;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::common::{background, error, file_dialog, stem};
use super::{App, Themed, progress, requests};
use crate::tasks::{Kind, file_name};
use crate::{BrowseRow, BrowserWindow, PasswordDialog, win};

/// 当前文件夹里的一项。
#[derive(Clone)]
struct Node {
    name: String,
    /// 压缩包里的完整路径（`/` 分隔）。
    path: String,
    is_dir: bool,
    size: u64,
    modified: Option<SystemTime>,
}

pub(crate) struct Browser {
    pub(super) window: BrowserWindow,
    archive: PathBuf,
    app: Weak<App>,
    password: RefCell<Option<String>>,
    /// 用户在密码框里勾了"记住这个密码"：下一次用它成功后保存。
    remember: Cell<bool>,
    /// 保存的密码已经都试过（不对），之后直接问。
    tried_saved: Cell<bool>,
    /// 文件夹（`/` 分隔，根是空串）→ 里面的项。
    tree: RefCell<BTreeMap<String, Vec<Node>>>,
    cwd: RefCell<String>,
    selected: RefCell<BTreeSet<usize>>,
    anchor: Cell<Option<usize>>,
    /// 按下时已经选中、没按 ctrl/shift 的那一行：松开时（没拖动）才改成只选它。
    pending: Cell<Option<usize>>,
    password_dialog: RefCell<Option<PasswordDialog>>,
    /// 列表的数据。只换选中状态时原地改，不整个换掉（整个换掉会重建每一行，双击就丢了）。
    rows: Rc<VecModel<BrowseRow>>,
    /// 列表里现在显示的是哪个文件夹。
    shown: RefCell<Option<String>>,
}

/// 打开浏览窗口；这个压缩包已经开着就提到最前。
pub(super) fn open(app: &Rc<App>, archive: PathBuf) {
    let archive = std::path::absolute(&archive).unwrap_or(archive);
    if let Some(b) = app.browsers.borrow().iter().find(|b| b.archive == archive) {
        win::present(&b.window);
        return;
    }
    let Ok(window) = BrowserWindow::new() else { return };
    let b = Rc::new(Browser {
        window,
        archive,
        app: Rc::downgrade(app),
        password: RefCell::new(None),
        remember: Cell::new(false),
        tried_saved: Cell::new(false),
        tree: RefCell::new(BTreeMap::new()),
        cwd: RefCell::new(String::new()),
        selected: RefCell::new(BTreeSet::new()),
        anchor: Cell::new(None),
        pending: Cell::new(None),
        password_dialog: RefCell::new(None),
        rows: Rc::new(VecModel::default()),
        shown: RefCell::new(None),
    });
    b.window.set_rows(ModelRc::from(b.rows.clone()));
    wire(&b);
    b.window.set_archive_name(file_name(&b.archive).into());
    b.window.apply(app.theme());
    win::present(&b.window);
    app.browsers.borrow_mut().push(b.clone());
    cleanup_temp();
    load(&b);
}

fn wire(b: &Rc<Browser>) {
    let w = &b.window;
    let me = Rc::downgrade(b);
    let with = move |f: fn(&Rc<Browser>)| {
        let me = me.clone();
        move || {
            if let Some(b) = me.upgrade() {
                f(&b);
            }
        }
    };
    w.on_up(with(|b| {
        let parent = parent(&b.cwd.borrow()).to_string();
        enter(b, parent);
    }));
    w.on_select_all(with(|b| {
        let n = b.tree.borrow().get(&*b.cwd.borrow()).map_or(0, Vec::len);
        *b.selected.borrow_mut() = (0..n).collect();
        show(b);
    }));
    w.on_extract_selected(with(extract_selected));
    w.on_extract_all(with(extract_all));
    w.on_enter_password(with(|b| {
        let retry = b.password.borrow().is_some();
        ask_password(b, retry, Rc::new(load));
    }));

    let me = Rc::downgrade(b);
    w.on_row_pressed(move |i, ctrl, shift| {
        let Some(b) = me.upgrade() else { return };
        let i = i as usize;
        b.pending.set(None);
        {
            let mut sel = b.selected.borrow_mut();
            if shift && let Some(a) = b.anchor.get() {
                let (lo, hi) = if a <= i { (a, i) } else { (i, a) };
                if !ctrl {
                    sel.clear();
                }
                sel.extend(lo..=hi);
            } else if ctrl {
                if !sel.remove(&i) {
                    sel.insert(i);
                }
                b.anchor.set(Some(i));
            } else if sel.contains(&i) {
                // 可能是要拖动已选中的几项：等松开再说。
                b.pending.set(Some(i));
                b.anchor.set(Some(i));
            } else {
                sel.clear();
                sel.insert(i);
                b.anchor.set(Some(i));
            }
        }
        show(&b);
    });
    let me = Rc::downgrade(b);
    w.on_row_clicked(move |i| {
        let Some(b) = me.upgrade() else { return };
        if b.pending.take() == Some(i as usize) {
            *b.selected.borrow_mut() = BTreeSet::from([i as usize]);
            show(&b);
        }
    });
    let me = Rc::downgrade(b);
    w.on_row_opened(move |i| {
        let Some(b) = me.upgrade() else { return };
        let Some(node) = node(&b, i as usize) else { return };
        if node.is_dir {
            enter(&b, node.path);
        } else {
            open_file(&b, node);
        }
    });
    let me = Rc::downgrade(b);
    w.on_drag_out(move |i| {
        let Some(b) = me.upgrade() else { return };
        b.pending.set(None);
        if !b.selected.borrow().contains(&(i as usize)) {
            *b.selected.borrow_mut() = BTreeSet::from([i as usize]);
            b.anchor.set(Some(i as usize));
            show(&b);
        }
        drag_out(&b);
    });

    let me = Rc::downgrade(b);
    w.window().on_close_requested(move || {
        if let Some(b) = me.upgrade() {
            if let Some(d) = b.password_dialog.borrow_mut().take() {
                let _ = d.hide();
            }
            if let Some(app) = b.app.upgrade() {
                app.browsers.borrow_mut().retain(|x| !Rc::ptr_eq(x, &b));
                super::quit_if_idle(&app);
            }
        }
        slint::CloseRequestResponse::HideWindow
    });
}

// ---- 列出、显示 ----

/// 在后台列出压缩包内容。
fn load(b: &Rc<Browser>) {
    b.window.set_message(t!("正在打开…").into());
    b.window.set_needs_password(false);
    b.window.set_busy(true);
    let archive = b.archive.clone();
    attempt(
        b,
        Arc::new(move |pw| browse::items(&archive, pw)),
        Rc::new(|b: &Rc<Browser>, r: Result<Vec<Item>>| {
            b.window.set_busy(false);
            match r {
                Ok(items) => {
                    *b.tree.borrow_mut() = build_tree(items);
                    *b.shown.borrow_mut() = None;
                    b.window.set_message(SharedString::new());
                    enter(b, String::new());
                }
                Err(e) if onezip_core::needs_password(&e) => {
                    b.window.set_message(t!("这个压缩包加密了，要输入密码才能看到里面的内容。").into());
                    b.window.set_needs_password(true);
                    b.window.set_status(SharedString::new());
                }
                Err(e) => {
                    crate::log::error(&t!("打开 {} 失败：{}", b.archive.display(), format!("{:#}", e)));
                    b.window.set_message(t!("打不开这个压缩包：{}", format!("{:#}", e)).into());
                    b.window.set_status(SharedString::new());
                }
            }
        }),
    );
}

/// 全部项 → 每个文件夹里有哪些。压缩包里没单独记文件夹的，按路径补上。
fn build_tree(items: Vec<Item>) -> BTreeMap<String, Vec<Node>> {
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    for it in items {
        // 补上各级上层文件夹
        let mut p = it.path.as_str();
        while let Some((up, _)) = p.rsplit_once('/') {
            nodes.entry(up.to_string()).or_insert_with(|| dir_node(up));
            p = up;
        }
        let node = Node {
            name: name_of(&it.path).to_string(),
            path: it.path.clone(),
            is_dir: it.is_dir,
            size: it.size,
            modified: it.modified,
        };
        match nodes.get_mut(&it.path) {
            // 文件夹先被补上了，现在才遇到它自己的那一项：用它的时间
            Some(n) if it.is_dir => n.modified = it.modified,
            _ => {
                nodes.insert(it.path, node);
            }
        }
    }
    let mut tree: BTreeMap<String, Vec<Node>> = BTreeMap::new();
    tree.insert(String::new(), Vec::new());
    for (path, node) in nodes {
        if node.is_dir {
            tree.entry(path.clone()).or_default();
        }
        tree.entry(parent(&path).to_string()).or_default().push(node);
    }
    for list in tree.values_mut() {
        list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| natural(&a.name).cmp(&natural(&b.name))));
    }
    tree
}

fn dir_node(path: &str) -> Node {
    Node { name: name_of(path).to_string(), path: path.to_string(), is_dir: true, size: 0, modified: None }
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

/// 排序用：不分大小写，数字按大小（file2 在 file10 前面）。
fn natural(s: &str) -> Vec<(u8, u128, String)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut n = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                n.push(d);
                chars.next();
            }
            out.push((0, n.parse().unwrap_or(u128::MAX), String::new()));
        } else {
            let mut t = String::new();
            while let Some(&d) = chars.peek().filter(|d| !d.is_ascii_digit()) {
                t.extend(d.to_lowercase());
                chars.next();
            }
            out.push((1, 0, t));
        }
    }
    out
}

fn node(b: &Browser, i: usize) -> Option<Node> {
    b.tree.borrow().get(&*b.cwd.borrow())?.get(i).cloned()
}

/// 进入一个文件夹。
fn enter(b: &Rc<Browser>, dir: String) {
    if !b.tree.borrow().contains_key(&dir) {
        return;
    }
    *b.cwd.borrow_mut() = dir;
    b.selected.borrow_mut().clear();
    b.anchor.set(None);
    b.pending.set(None);
    show(b);
}

/// 把当前文件夹的内容和选中状态交给界面。
fn show(b: &Browser) {
    let cwd = b.cwd.borrow().clone();
    let tree = b.tree.borrow();
    let list = tree.get(&cwd).map_or(&[][..], Vec::as_slice);
    let sel = b.selected.borrow();
    if b.shown.borrow().as_deref() != Some(cwd.as_str()) {
        let rows: Vec<BrowseRow> = list
            .iter()
            .enumerate()
            .map(|(i, n)| BrowseRow {
                name: n.name.as_str().into(),
                size: if n.is_dir { SharedString::new() } else { human(n.size).into() },
                modified: n.modified.map(win::local_time).unwrap_or_default().into(),
                is_dir: n.is_dir,
                selected: sel.contains(&i),
            })
            .collect();
        b.rows.set_vec(rows);
        *b.shown.borrow_mut() = Some(cwd.clone());
    } else {
        for i in 0..b.rows.row_count() {
            if let Some(mut r) = b.rows.row_data(i)
                && r.selected != sel.contains(&i)
            {
                r.selected = !r.selected;
                b.rows.set_row_data(i, r);
            }
        }
    }
    let w = &b.window;
    w.set_can_up(!cwd.is_empty());
    let mut location = file_name(&b.archive);
    for part in cwd.split('/').filter(|p| !p.is_empty()) {
        location.push_str(" › ");
        location.push_str(part);
    }
    w.set_location(location.into());
    w.set_selected_count(sel.len() as i32);
    let (dirs, files) = (list.iter().filter(|n| n.is_dir).count(), list.iter().filter(|n| !n.is_dir).count());
    let status = if sel.is_empty() {
        let size: u64 = list.iter().map(|n| total(&tree, n)).sum();
        t!("{} 个文件夹，{} 个文件（共 {}）", dirs, files, human(size))
    } else {
        let size: u64 = sel.iter().filter_map(|&i| list.get(i)).map(|n| total(&tree, n)).sum();
        t!("已选中 {} 项（{}）· 可以直接拖到资源管理器", sel.len(), human(size))
    };
    w.set_status(status.into());
}

/// 一项的大小；文件夹是里面全部文件加起来。
fn total(tree: &BTreeMap<String, Vec<Node>>, n: &Node) -> u64 {
    if !n.is_dir {
        return n.size;
    }
    tree.get(&n.path).map_or(0, |list| list.iter().map(|c| total(tree, c)).sum())
}

/// 选中的项在压缩包里的路径。
fn picks(b: &Browser) -> Vec<String> {
    let tree = b.tree.borrow();
    let Some(list) = tree.get(&*b.cwd.borrow()) else { return Vec::new() };
    b.selected.borrow().iter().filter_map(|&i| list.get(i)).map(|n| n.path.clone()).collect()
}

// ---- 解压 ----

/// "解压选中的项…"：选文件夹，排进任务队列。
fn extract_selected(b: &Rc<Browser>) {
    let Some(app) = b.app.upgrade() else { return };
    let picks = picks(b);
    if picks.is_empty() {
        return;
    }
    let start = b.archive.parent().map(Path::to_path_buf).unwrap_or_default();
    let Some(output) = file_dialog(b.window.window()).set_title(t!("解压到哪个文件夹")).set_directory(start).pick_folder()
    else {
        return;
    };
    let req = PickRequest {
        archive: b.archive.clone(),
        password: b.password.borrow().clone(),
        base: b.cwd.borrow().clone(),
        output,
        conflict: requests::conflict(&app.settings.borrow()),
        picks,
    };
    let title = t!("解压 {} 里的 {} 项", file_name(&b.archive), req.picks.len());
    let id = app.tasks.add(title, Kind::Pick(req));
    progress::open_progress(&app, id);
}

/// "全部解压…"：选文件夹，在里面新建一个以压缩包命名的文件夹（和右键"解压到…"一样）。
fn extract_all(b: &Rc<Browser>) {
    let Some(app) = b.app.upgrade() else { return };
    let start = b.archive.parent().map(Path::to_path_buf).unwrap_or_default();
    let Some(root) = file_dialog(b.window.window()).set_title(t!("解压到哪个文件夹")).set_directory(start).pick_folder()
    else {
        return;
    };
    let s = app.settings.borrow().clone();
    let req = ExtractRequest {
        source: Source::Local(b.archive.clone()),
        output: Some(root.join(stem(&b.archive))),
        placement: Placement::Smart,
        keep_source: true,
        password: b.password.borrow().clone(),
        keep_free: 512 << 20,
        nested: s.nested,
        conflict: requests::conflict(&s),
        ..Default::default()
    };
    let id = app.tasks.add(t!("解压 {}", file_name(&b.archive)), Kind::Extract(req, None));
    progress::open_progress(&app, id);
}

/// 解到临时文件夹（打开、拖出用），返回这个文件夹。
fn temp_dir(kind: &str) -> PathBuf {
    let n = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    temp_root().join(kind).join(format!("{}-{n}", std::process::id()))
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join("OneZip")
}

/// 删掉一天前留下的临时文件（拖出、打开的文件可能还被别的程序用着，不马上删）。
fn cleanup_temp() {
    std::thread::spawn(|| {
        for kind in ["open", "drag"] {
            let Ok(dirs) = std::fs::read_dir(temp_root().join(kind)) else { continue };
            for d in dirs.flatten() {
                let old = d
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age.as_secs() > 24 * 3600);
                if old {
                    let _ = std::fs::remove_dir_all(d.path());
                }
            }
        }
    });
}

fn pick_to(b: &Browser, picks: Vec<String>, output: PathBuf) -> PickRequest {
    PickRequest {
        archive: b.archive.clone(),
        password: None,
        picks,
        base: b.cwd.borrow().clone(),
        output,
        conflict: Conflict::Replace,
    }
}

/// 双击文件：解到临时文件夹，用默认程序打开。
fn open_file(b: &Rc<Browser>, node: Node) {
    let req = pick_to(b, vec![node.path.clone()], temp_dir("open"));
    b.window.set_status(t!("正在打开 {}…", node.name).into());
    b.window.set_busy(true);
    attempt(
        b,
        Arc::new(move |pw| {
            let mut r = req.clone();
            r.password = pw;
            browse::extract(&r, Arc::new(|_| {}), &Arc::new(AtomicBool::new(false)))
        }),
        Rc::new(move |b: &Rc<Browser>, r: Result<PathBuf>| {
            b.window.set_busy(false);
            show(b);
            match r {
                Ok(dir) => win::shell_open(&dir.join(&node.name).display().to_string()),
                Err(e) if onezip_core::needs_password(&e) => {}
                Err(e) => error(b.window.window(), &t!("打开 {} 失败：{}", node.name, format!("{:#}", e))),
            }
        }),
    );
}

/// 拖到资源管理器：先把选中的项解到临时文件夹（要等一会儿，界面这时不动），再开始拖放。
/// 需要密码时先问密码，输完再拖一次。
fn drag_out(b: &Rc<Browser>) {
    let picks = picks(b);
    if picks.is_empty() {
        return;
    }
    let names: Vec<String> = picks.iter().map(|p| name_of(p).to_string()).collect();
    let mut req = pick_to(b, picks, temp_dir("drag"));
    req.password = b.password.borrow().clone();
    let r = browse::extract(&req, Arc::new(|_| {}), &Arc::new(AtomicBool::new(false)));
    match r {
        Ok(dir) => {
            if b.remember.take()
                && let Some(pw) = b.password.borrow().as_deref()
            {
                let _ = passwords::remember(pw, &file_name(&b.archive));
            }
            let paths: Vec<PathBuf> = names.iter().map(|n| dir.join(n)).collect();
            if let Err(e) = win::drag_files(&paths) {
                error(b.window.window(), &t!("拖放失败：{}", format!("{:#}", e)));
            }
        }
        Err(e) if onezip_core::needs_password(&e) => {
            // 后台试保存的密码、问密码；有了密码后请用户再拖一次。
            let first = names.first().cloned().unwrap_or_default();
            let req = pick_to(b, vec![join(&b.cwd.borrow(), &first)], temp_dir("drag"));
            attempt(
                b,
                Arc::new(move |pw| {
                    let mut r = req.clone();
                    r.password = pw;
                    browse::extract(&r, Arc::new(|_| {}), &Arc::new(AtomicBool::new(false)))
                }),
                Rc::new(|b: &Rc<Browser>, r: Result<PathBuf>| match r {
                    Ok(_) => b.window.set_status(t!("密码正确，请再拖一次").into()),
                    Err(e) if onezip_core::needs_password(&e) => {}
                    Err(e) => error(b.window.window(), &t!("解压失败：{}", format!("{:#}", e))),
                }),
            );
        }
        Err(e) => error(b.window.window(), &t!("解压失败：{}", format!("{:#}", e))),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

// ---- 密码 ----

type Op<T> = Arc<dyn Fn(Option<String>) -> Result<T> + Send + Sync>;
type Done<T> = Rc<dyn Fn(&Rc<Browser>, Result<T>)>;
/// 有了密码之后接着做的事。
type Then = Rc<dyn Fn(&Rc<Browser>)>;

/// 在后台做 `op`（带着已知的密码）。要密码时：先试保存的密码（只试一次），
/// 再弹密码框，输入后重做。最后把结果交给 `done`（还要密码而用户取消了，也会交给它）。
fn attempt<T: Send + 'static>(b: &Rc<Browser>, op: Op<T>, done: Done<T>) {
    let pw = b.password.borrow().clone();
    let try_saved = !b.tried_saved.get();
    let work = op.clone();
    let me = Rc::downgrade(b);
    background(
        move || {
            let r = work(pw.clone());
            match r {
                Err(e) if try_saved && onezip_core::needs_password(&e) => {
                    for s in passwords::load() {
                        if Some(&s.password) == pw.as_ref() {
                            continue;
                        }
                        match work(Some(s.password.clone())) {
                            Err(e) if onezip_core::needs_password(&e) => continue,
                            r => return (r, Some(s.password)),
                        }
                    }
                    (Err(e), None)
                }
                r => (r, None),
            }
        },
        move |(r, found): (Result<T>, Option<String>)| {
            let Some(b) = me.upgrade() else { return };
            b.tried_saved.set(true);
            if let Some(pw) = found {
                let _ = passwords::remember(&pw, &file_name(&b.archive));
                *b.password.borrow_mut() = Some(pw);
            }
            match &r {
                Err(e) if onezip_core::needs_password(e) => {
                    let retry = b.password.borrow().is_some();
                    let (op, done2) = (op.clone(), done.clone());
                    ask_password(&b, retry, Rc::new(move |b: &Rc<Browser>| attempt(b, op.clone(), done2.clone())));
                }
                Ok(_) => {
                    if b.remember.take()
                        && let Some(pw) = b.password.borrow().as_deref()
                    {
                        let _ = passwords::remember(pw, &file_name(&b.archive));
                    }
                }
                Err(_) => {}
            }
            done(&b, r);
        },
    );
}

/// 弹出密码框；输入后记下密码并调用 `then`。
fn ask_password(b: &Rc<Browser>, retry: bool, then: Then) {
    if let Some(old) = b.password_dialog.borrow_mut().take() {
        let _ = old.hide();
    }
    let Ok(d) = PasswordDialog::new() else { return };
    d.set_archive(file_name(&b.archive).into());
    d.set_retry(retry);
    let me = Rc::downgrade(b);
    d.on_reject(move || {
        if let Some(b) = me.upgrade()
            && let Some(d) = b.password_dialog.borrow_mut().take()
        {
            let _ = d.hide();
        }
    });
    let me = Rc::downgrade(b);
    d.on_accept(move || {
        let Some(b) = me.upgrade() else { return };
        let Some(d) = b.password_dialog.borrow_mut().take() else { return };
        let pw = d.get_password().to_string();
        let remember = d.get_remember();
        let _ = d.hide();
        if pw.is_empty() {
            return;
        }
        *b.password.borrow_mut() = Some(pw);
        b.remember.set(remember);
        then(&b);
    });
    if let Some(app) = b.app.upgrade() {
        d.apply(app.theme());
    }
    win::present_owned(&d, &b.window);
    *b.password_dialog.borrow_mut() = Some(d);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, is_dir: bool) -> Item {
        Item { path: path.into(), is_dir, size: 1, modified: None }
    }

    #[test]
    fn tree_fills_in_missing_folders_and_sorts() {
        let t = build_tree(vec![
            item("a/file10.txt", false),
            item("a/file2.txt", false),
            item("a/b/c.txt", false),
            item("z.txt", false),
            item("a", true),
        ]);
        let names = |dir: &str| t[dir].iter().map(|n| n.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(""), ["a", "z.txt"]);
        assert_eq!(names("a"), ["b", "file2.txt", "file10.txt"]);
        assert_eq!(names("a/b"), ["c.txt"]);
        assert!(t[""][0].is_dir);
    }
}
