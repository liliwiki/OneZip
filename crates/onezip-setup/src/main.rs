//! OneZip 安装程序，同时也是卸载程序（安装时把自己去掉后面的安装文件，存成安装目录里的
//! `uninstall.exe`）。全部步骤直接调 Windows 接口完成，不运行 PowerShell 脚本。
//!
//! - 无参数：安装界面。默认装到已安装的位置，没装过就是 `%LOCALAPPDATA%\Programs\OneZip`。
//! - `--update`：OneZip 的"立即更新"下载后启动。不问问题，等 OneZip 退出后装到原来的位置，
//!   装完重新打开 OneZip。
//! - 文件名是 uninstall.exe，或 `--uninstall`：卸载界面。
//! - `--grant <文件夹> <SID>`、`--trust <证书>`：安装过程中以管理员身份再启动自己做的那一步。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod install;
mod payload;
mod sys;
mod uninstall;

use onezip_i18n::t;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use anyhow::{Result, bail};
use slint::ComponentHandle;

use install::{Plan, same_dir};

slint::include_modules!();

fn main() -> Result<()> {
    // 装过 OneZip 的按它的语言设置，没装过的跟随 Windows（中文用中文，其他用英文）。
    onezip_settings::apply_language();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let is_uninstaller = std::env::current_exe()
        .ok()
        .and_then(|e| e.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
        .is_some_and(|s| s.starts_with("uninstall"));
    match arg(0).as_str() {
        "--grant" => elevated(sys::grant(Path::new(&arg(1)), &arg(2))),
        "--trust" => elevated(std::fs::read(arg(1)).map_err(Into::into).and_then(|der| sys::trust(&der))),
        "--uninstall" => run_uninstall(),
        _ if is_uninstaller => run_uninstall(),
        "--update" => run_setup(true),
        _ => run_setup(false),
    }
}

/// 管理员身份运行的那一步：用退出码告诉安装程序结果。
fn elevated(r: Result<()>) -> ! {
    std::process::exit(if r.is_ok() { 0 } else { 1 })
}

/// 安装界面上显示进度：从安装线程把当前步骤交给界面。
fn reporter<W: ComponentHandle + 'static>(weak: slint::Weak<W>, set: fn(&W, &str)) -> impl Fn(&str) + Send + 'static {
    move |s: &str| {
        let s = s.to_string();
        let _ = weak.upgrade_in_event_loop(move |w| set(&w, &s));
    }
}

fn run_setup(update: bool) -> Result<()> {
    let w = SetupWindow::new()?;
    let _ = slint::select_bundled_translation(onezip_i18n::lang().slint_name());
    w.set_version(env!("CARGO_PKG_VERSION").into());
    let old = Rc::new(installed_location());
    let start = old.as_ref().clone().unwrap_or_else(default_dir);
    w.set_location(start.display().to_string().into());
    w.set_upgrade(old.is_some());
    w.set_win11(sys::is_win11());
    let refresh = {
        let (weak, old) = (w.as_weak(), old.clone());
        move || {
            let Some(w) = weak.upgrade() else { return };
            match resolve(w.get_location().as_str(), old.as_deref()) {
                Ok(t) => {
                    w.set_target(t.display().to_string().into());
                    w.set_problem("".into());
                }
                Err(e) => {
                    w.set_target("".into());
                    w.set_problem(format!("{e:#}").into());
                }
            }
        }
    };
    refresh();
    w.on_location_edited({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    w.on_browse({
        let (weak, refresh) = (w.as_weak(), refresh.clone());
        move || {
            let Some(w) = weak.upgrade() else { return };
            let current = PathBuf::from(w.get_location().as_str());
            let mut d = rfd::FileDialog::new().set_title(t!("选择安装位置"));
            if let Some(dir) = current.ancestors().find(|p| p.is_dir()) {
                d = d.set_directory(dir);
            }
            if let Some(dir) = d.pick_folder() {
                w.set_location(dir.display().to_string().into());
                refresh();
            }
        }
    });

    // 开始安装（更新时不经过按钮，直接开始）。
    let begin = {
        let (weak, old) = (w.as_weak(), old.clone());
        move |update: bool| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(dest) = resolve(w.get_location().as_str(), old.as_deref()) else { return };
            let mut dirs = vec![dest.clone()];
            dirs.extend(old.as_ref().clone());
            if !update && !sys::onezip_running(&dirs).is_empty() {
                let ok = rfd::MessageDialog::new()
                    .set_title(t!("安装 OneZip"))
                    .set_description(t!("OneZip 正在运行，安装时会把它关掉（没做完的边解压边删除任务以后可以接着做）。继续吗？"))
                    .set_buttons(rfd::MessageButtons::OkCancel)
                    .show();
                if ok != rfd::MessageDialogResult::Ok {
                    return;
                }
            }
            w.set_target(dest.display().to_string().into());
            w.set_stage("installing".into());
            let plan = Plan { dest, old: old.as_ref().clone(), update };
            let say = reporter(weak.clone(), |w: &SetupWindow, s| w.set_status(s.into()));
            let weak = weak.clone();
            std::thread::spawn(move || {
                let r = install::install(&plan, &say);
                let _ = weak.upgrade_in_event_loop(move |w| match r {
                    Ok(()) if update => {
                        launch(&plan.dest);
                        let _ = w.hide();
                    }
                    Ok(()) => w.set_stage("done".into()),
                    Err(e) => {
                        w.set_error(format!("{e:#}").into());
                        w.set_stage("failed".into());
                    }
                });
            });
        }
    };
    w.on_install({
        let begin = begin.clone();
        move || begin(false)
    });
    w.on_finish({
        let weak = w.as_weak();
        move || {
            let Some(w) = weak.upgrade() else { return };
            if w.get_launch() {
                launch(Path::new(w.get_target().as_str()));
            }
            let _ = w.hide();
        }
    });
    w.on_quit({
        let weak = w.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                let _ = w.hide();
            }
        }
    });
    // 安装进行中不允许关窗口，免得只装了一半。
    w.window().on_close_requested({
        let weak = w.as_weak();
        move || match weak.upgrade() {
            Some(w) if w.get_stage() == "installing" => slint::CloseRequestResponse::KeepWindowShown,
            _ => slint::CloseRequestResponse::HideWindow,
        }
    });
    if update && old.is_some() {
        w.set_updating(true);
        begin(true);
    }
    w.run()?;
    Ok(())
}

fn launch(dir: &Path) {
    let _ = Command::new(dir.join("OneZip.exe")).current_dir(dir).spawn();
}

fn run_uninstall() -> Result<()> {
    let Some(dest) = uninstall::target() else {
        rfd::MessageDialog::new()
            .set_title(t!("卸载 OneZip"))
            .set_description(t!("没有找到已安装的 OneZip。"))
            .show();
        return Ok(());
    };
    let w = UninstallWindow::new()?;
    let _ = slint::select_bundled_translation(onezip_i18n::lang().slint_name());
    let version = sys::uninstall_key()
        .and_then(|k| k.get_value::<String, _>("DisplayVersion").ok())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
    w.set_version(version.into());
    w.set_location(dest.display().to_string().into());
    w.on_uninstall({
        let (weak, dest) = (w.as_weak(), dest.clone());
        move || {
            let Some(w) = weak.upgrade() else { return };
            w.set_stage("working".into());
            let delete_settings = w.get_delete_settings();
            let say = reporter(weak.clone(), |w: &UninstallWindow, s| w.set_status(s.into()));
            let (weak, dest) = (weak.clone(), dest.clone());
            std::thread::spawn(move || {
                let r = uninstall::uninstall(&dest, delete_settings, &say);
                let _ = weak.upgrade_in_event_loop(move |w| match r {
                    Ok(()) => w.set_stage("done".into()),
                    Err(e) => {
                        w.set_error(format!("{e:#}").into());
                        w.set_stage("failed".into());
                    }
                });
            });
        }
    });
    w.on_quit({
        let weak = w.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                let _ = w.hide();
            }
        }
    });
    w.window().on_close_requested({
        let weak = w.as_weak();
        move || match weak.upgrade() {
            Some(w) if w.get_stage() == "working" => slint::CloseRequestResponse::KeepWindowShown,
            _ => slint::CloseRequestResponse::HideWindow,
        }
    });
    w.run()?;
    // 卸载完成后删掉正在运行的自己（以及空了的安装目录）。
    let finished = matches!(w.get_stage().as_str(), "done" | "failed");
    if finished
        && let Ok(me) = std::env::current_exe()
        && me.parent().is_some_and(|p| same_dir(p, &dest))
    {
        sys::delete_after_exit(&me, &dest);
    }
    Ok(())
}

fn default_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join("Programs").join("OneZip")
}

/// 已安装的位置（"应用和功能"卸载项里的 InstallLocation）。
fn installed_location() -> Option<PathBuf> {
    let loc: String = sys::uninstall_key()?.get_value("InstallLocation").ok()?;
    let p = PathBuf::from(loc);
    p.join("OneZip.exe").exists().then_some(p)
}

/// 实际装进哪个文件夹：选定的文件夹本来就装着 OneZip，或者叫 OneZip 且是空的/还不存在，
/// 就装进它本身；否则装进它下面的 OneZip 子文件夹，那里也有别的文件就不装。
/// 这样卸载按文件名删除时不会碰到别人的同名文件。
fn resolve(chosen: &str, old: Option<&Path>) -> Result<PathBuf> {
    let chosen = chosen.trim().trim_matches('"');
    if chosen.is_empty() {
        bail!(t!("请选择安装位置"));
    }
    let p = PathBuf::from(chosen);
    if !p.is_absolute() || !p.has_root() {
        bail!(t!("请填写完整的路径，例如 D:\\Programs\\OneZip"));
    }
    if chosen.contains(['*', '?', '<', '>', '|', '"']) {
        bail!(t!("路径里有不能用的字符"));
    }
    let p = PathBuf::from(chosen.trim_end_matches('\\'));
    let p = if p.as_os_str().len() <= 2 { PathBuf::from(format!("{}\\", p.display())) } else { p };
    let is_install = |d: &Path| {
        d.join("installed-files.txt").exists()
            || (d.join("OneZip.exe").exists() && d.join("uninstall.exe").exists())
            || old.is_some_and(|o| same_dir(o, d))
    };
    let is_empty = |d: &Path| !d.exists() || std::fs::read_dir(d).is_ok_and(|mut r| r.next().is_none());
    let named = p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("OneZip"));
    if is_install(&p) || (named && is_empty(&p)) {
        return Ok(p);
    }
    let sub = p.join("OneZip");
    if is_install(&sub) || is_empty(&sub) {
        return Ok(sub);
    }
    bail!(t!("{} 已经存在并且有别的文件，请换一个位置", sub.display()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_safe_install_folder() {
        let root = std::env::temp_dir().join(format!("onezip-setup-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let s = |p: &Path| p.display().to_string();
        // 不叫 OneZip 的文件夹：装进它下面的 OneZip
        assert_eq!(resolve(&s(&root), None).unwrap(), root.join("OneZip"));
        // 叫 OneZip、还不存在或是空的：装进它本身
        let named = root.join("x").join("OneZip");
        assert_eq!(resolve(&s(&named), None).unwrap(), named);
        std::fs::create_dir_all(&named).unwrap();
        assert_eq!(resolve(&s(&named), None).unwrap(), named);
        // 叫 OneZip 但有别的文件（比如源码目录）：装进子文件夹
        std::fs::write(named.join("README.md"), "x").unwrap();
        assert_eq!(resolve(&s(&named), None).unwrap(), named.join("OneZip"));
        // 子文件夹也被占了：拒绝
        std::fs::create_dir_all(named.join("OneZip")).unwrap();
        std::fs::write(named.join("OneZip").join("a.txt"), "x").unwrap();
        assert!(resolve(&s(&named), None).is_err());
        // 已经装在这里（旧位置）：原地升级
        assert_eq!(resolve(&s(&named), Some(&named)).unwrap(), named);
        // 早期版本（有安装清单）和现在的安装（有 uninstall.exe）都算
        let inst = root.join("inst");
        std::fs::create_dir_all(&inst).unwrap();
        std::fs::write(inst.join("installed-files.txt"), "OneZip.exe").unwrap();
        assert_eq!(resolve(&s(&inst), None).unwrap(), inst);
        let now = root.join("now");
        std::fs::create_dir_all(&now).unwrap();
        std::fs::write(now.join("OneZip.exe"), "x").unwrap();
        std::fs::write(now.join("uninstall.exe"), "x").unwrap();
        assert_eq!(resolve(&s(&now), None).unwrap(), now);
        // 相对路径、空路径不行
        assert!(resolve("relative\\dir", None).is_err());
        assert!(resolve("  ", None).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
