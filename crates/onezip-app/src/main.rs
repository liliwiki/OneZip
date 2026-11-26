//! OneZip.exe：主窗口，也是右键菜单调用的程序。
//!
//! 同一用户只运行一个 OneZip：再次启动（例如在右键菜单里连续操作）时，
//! 新进程把请求交给已在运行的那个，排进同一个任务队列，然后立即退出。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod log;
mod menu;
mod request;
mod tasks;
mod ui;
mod update;
mod verify;
mod win;

use std::sync::mpsc;

use request::{Action, Request, Role};

slint::include_modules!();

fn main() {
    if let Err(e) = run() {
        rfd::MessageDialog::new()
            .set_title("OneZip")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!("{e:#}"))
            .show();
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    log::install_panic_hook();
    let req = Request::from_args(std::env::args().skip(1))?;

    // 安装/卸载程序调用：不开窗口。
    match req.action {
        Action::RegisterMenu if req.paths.first().is_some_and(|p| p.as_os_str() == "/quiet") => {
            // 按设置来：装了 Windows 11 第一层菜单或关掉了右键菜单时不写传统菜单。
            return menu::apply(&onezip_settings::Settings::load());
        }
        Action::UnregisterMenu => {
            menu::unregister();
            return Ok(());
        }
        _ => {}
    }

    let (tx, rx) = mpsc::channel::<Request>();
    match request::claim(&req, move |r| {
        let _ = tx.send(r);
    }) {
        Role::Forwarded => Ok(()),
        Role::Primary => ui::run(req, rx),
    }
}
