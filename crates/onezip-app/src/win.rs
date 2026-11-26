//! 和 Windows 窗口管理打交道的小工具。
//!
//! 右键菜单启动的 OneZip 不是前台进程，Windows 默认不让它把窗口放到最前面，
//! 结果窗口（例如"添加到压缩包"对话框）被压在资源管理器后面，看起来像"什么都没发生"。
//! 这里负责：把窗口提到最前、让对话框归属主窗口（始终在主窗口之上），
//! 以及把前台权限转交给已在运行的实例。

use std::time::Duration;

use slint::ComponentHandle;

#[cfg(windows)]
mod imp {
    use onezip_i18n::t;
    use std::ffi::c_void;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        ASFW_ANY, AllowSetForegroundWindow, BringWindowToTop, GWLP_HWNDPARENT,
        GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic,
        SW_RESTORE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow,
        SetWindowLongPtrW, SetWindowPos, ShowWindow,
    };

    pub fn hwnd(window: &slint::Window) -> Option<HWND> {
        let handle = window.window_handle();
        let raw = handle.window_handle().ok()?.as_raw();
        match raw {
            RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut c_void)),
            _ => None,
        }
    }

    pub fn bring_to_front(window: &slint::Window) {
        let Some(h) = hwnd(window) else { return };
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            // 先暂时置顶再取消：即使拿不到键盘焦点，窗口也一定看得见。
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
            let _ = SetWindowPos(h, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);

            // 借用当前前台线程的输入状态，SetForegroundWindow 才会生效。
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let me = GetCurrentThreadId();
            let attached = fg_thread != 0
                && fg_thread != me
                && AttachThreadInput(me, fg_thread, true).as_bool();
            let _ = BringWindowToTop(h);
            let _ = SetForegroundWindow(h);
            if attached {
                let _ = AttachThreadInput(me, fg_thread, false);
            }
        }
    }

    /// 让 `child` 归属 `owner`：始终显示在 owner 之上，不单独占任务栏按钮。
    pub fn set_owner(child: &slint::Window, owner: &slint::Window) {
        let (Some(c), Some(o)) = (hwnd(child), hwnd(owner)) else {
            return;
        };
        unsafe {
            SetWindowLongPtrW(c, GWLP_HWNDPARENT, o.0 as isize);
        }
    }

    /// 把 `child` 放到 `owner` 正中（不改大小）。
    pub fn center_on(child: &slint::Window, owner: &slint::Window) {
        let (Some(c), Some(o)) = (hwnd(child), hwnd(owner)) else {
            return;
        };
        let mut cr = RECT::default();
        let mut or = RECT::default();
        unsafe {
            if GetWindowRect(c, &mut cr).is_err() || GetWindowRect(o, &mut or).is_err() {
                return;
            }
            let (cw, ch) = (cr.right - cr.left, cr.bottom - cr.top);
            let x = or.left + ((or.right - or.left) - cw) / 2;
            let y = or.top + ((or.bottom - or.top) - ch) / 2;
            let _ = SetWindowPos(c, None, x, y.max(0), 0, 0, SWP_NOSIZE | SWP_NOZORDER);
        }
    }

    pub fn allow_any_foreground() {
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }
    }

    /// 用默认程序打开链接、文件夹或文件（浏览器、资源管理器）。
    pub fn shell_open(target: &str) {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{HSTRING, w};
        unsafe {
            ShellExecuteW(None, w!("open"), &HSTRING::from(target), None, None, SW_SHOWNORMAL);
        }
    }

    /// 把文件移到回收站，不弹任何确认或进度框。
    pub fn recycle(paths: &[std::path::PathBuf]) -> std::io::Result<()> {
        use windows::Win32::UI::Shell::{
            FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW,
        };
        use windows::core::PCWSTR;
        if paths.is_empty() {
            return Ok(());
        }
        // 多个路径用 \0 隔开，最后再多一个 \0。
        let mut from: Vec<u16> = Vec::new();
        for p in paths {
            let abs = std::path::absolute(p)?;
            from.extend(abs.as_os_str().to_string_lossy().encode_utf16());
            from.push(0);
        }
        from.push(0);
        let mut op = SHFILEOPSTRUCTW {
            wFunc: FO_DELETE,
            pFrom: PCWSTR(from.as_ptr()),
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI).0 as u16,
            ..Default::default()
        };
        let code = unsafe { SHFileOperationW(&mut op) };
        if code != 0 {
            return Err(std::io::Error::other(t!("移到回收站失败（错误 {}）", format!("{:#x}", code))));
        }
        if op.fAnyOperationsAborted.as_bool() {
            return Err(std::io::Error::other(t!("移到回收站被中止")));
        }
        Ok(())
    }

    /// 如 `Windows 11 24H2（26100）`，报告问题时用。
    pub fn windows_version() -> String {
        use winreg::RegKey;
        use winreg::enums::HKEY_LOCAL_MACHINE;
        let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") else {
            return t!("未知").into();
        };
        let build: String = k.get_value("CurrentBuild").unwrap_or_default();
        let display: String = k.get_value("DisplayVersion").unwrap_or_default();
        let major = if build.parse::<u32>().unwrap_or(0) >= 22000 { "Windows 11" } else { "Windows 10" };
        t!("{} {}（{}）", major, display, build)
    }

    /// 把文本放进剪贴板。
    pub fn set_clipboard(text: &str) -> bool {
        use windows::Win32::Foundation::{HANDLE, HGLOBAL};
        use windows::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
        };
        use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
        const CF_UNICODETEXT: u32 = 13;

        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            if OpenClipboard(None).is_err() {
                return false;
            }
            let ok = (|| -> Option<()> {
                EmptyClipboard().ok()?;
                let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
                let ptr = GlobalLock(mem) as *mut u16;
                if ptr.is_null() {
                    return None;
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                let _ = GlobalUnlock(mem);
                // 成功后内存归剪贴板所有，不能再释放。
                SetClipboardData(CF_UNICODETEXT, Some(HANDLE(mem.0))).ok()?;
                Some(())
            })()
            .is_some();
            let _ = CloseClipboard();
            ok
        }
    }
}

#[cfg(windows)]
mod drag {
    //! 把文件拖到资源管理器（或任何接受文件拖放的程序）。

    use std::path::PathBuf;

    use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};
    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize};
    use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{BHID_DataObject, ILCreateFromPathW, ILFree, IShellItemArray, SHCreateShellItemArrayFromIDLists};
    use windows::core::{BOOL, HRESULT, HSTRING};
    use windows_core::implement;

    #[implement(IDropSource)]
    struct Source;

    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
            if escape.as_bool() {
                DRAGDROP_S_CANCEL
            } else if keys.0 & MK_LBUTTON.0 == 0 {
                DRAGDROP_S_DROP
            } else {
                S_OK
            }
        }
        fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// 开始拖放这些文件（都在同一个文件夹里），直到松开鼠标才返回。
    pub fn drag_files(paths: &[PathBuf]) -> windows::core::Result<()> {
        unsafe {
            let _ = OleInitialize(None);
            let pidls: Vec<*mut ITEMIDLIST> =
                paths.iter().map(|p| ILCreateFromPathW(&HSTRING::from(p.as_path()))).filter(|p| !p.is_null()).collect();
            let r = (|| {
                let list: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
                let items: IShellItemArray = SHCreateShellItemArrayFromIDLists(&list)?;
                let data: IDataObject = items.BindToHandler(None, &BHID_DataObject)?;
                let source: IDropSource = Source.into();
                let mut effect = DROPEFFECT(0);
                DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect).ok()
            })();
            for p in pidls {
                ILFree(Some(p));
            }
            r
        }
    }
}

#[cfg(windows)]
pub use drag::drag_files;

/// 本地时间，如 `2026-11-26 21:10`。
#[cfg(windows)]
pub fn local_time(t: std::time::SystemTime) -> String {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) else { return String::new() };
    let ft = d.as_nanos() as u64 / 100 + 116_444_736_000_000_000;
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            return String::new();
        }
    }
    format!("{}-{:02}-{:02} {:02}:{:02}", local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
}

#[cfg(not(windows))]
pub fn local_time(_t: std::time::SystemTime) -> String {
    String::new()
}

#[cfg(not(windows))]
pub fn drag_files(_paths: &[std::path::PathBuf]) -> anyhow::Result<()> {
    anyhow::bail!(onezip_i18n::t!("只支持 Windows"))
}

#[cfg(not(windows))]
mod imp {
    use onezip_i18n::t;
    pub fn bring_to_front(_window: &slint::Window) {}
    pub fn set_owner(_child: &slint::Window, _owner: &slint::Window) {}
    pub fn center_on(_child: &slint::Window, _owner: &slint::Window) {}
    pub fn allow_any_foreground() {}
    pub fn set_clipboard(_text: &str) -> bool {
        false
    }
    pub fn shell_open(_target: &str) {}
    pub fn recycle(paths: &[std::path::PathBuf]) -> std::io::Result<()> {
        paths.iter().try_for_each(std::fs::remove_file)
    }
    pub fn windows_version() -> String {
        t!("未知").into()
    }
}

#[cfg(windows)]
pub use imp::hwnd;
pub use imp::{
    allow_any_foreground, bring_to_front, center_on, recycle, set_clipboard, set_owner, shell_open, windows_version,
};

/// 显示并提到最前。窗口可能要等事件循环跑一轮才真正创建，所以稍后再提一次。
pub fn present<C: ComponentHandle + 'static>(component: &C) {
    let _ = component.show();
    bring_to_front(component.window());
    let weak = component.as_weak();
    slint::Timer::single_shot(Duration::from_millis(60), move || {
        if let Some(c) = weak.upgrade() {
            bring_to_front(c.window());
        }
    });
}

/// 显示一个归属 `owner` 的对话框并提到最前。
pub fn present_owned<C: ComponentHandle + 'static, O: ComponentHandle + 'static>(
    dialog: &C,
    owner: &O,
) {
    let _ = dialog.show();
    set_owner(dialog.window(), owner.window());
    center_on(dialog.window(), owner.window());
    bring_to_front(dialog.window());
    let d = dialog.as_weak();
    let o = owner.as_weak();
    slint::Timer::single_shot(Duration::from_millis(60), move || {
        if let (Some(d), Some(o)) = (d.upgrade(), o.upgrade()) {
            set_owner(d.window(), o.window());
            center_on(d.window(), o.window());
            bring_to_front(d.window());
        }
    });
}
