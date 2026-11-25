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

#[cfg(not(windows))]
mod imp {
    pub fn bring_to_front(_window: &slint::Window) {}
    pub fn set_owner(_child: &slint::Window, _owner: &slint::Window) {}
    pub fn center_on(_child: &slint::Window, _owner: &slint::Window) {}
    pub fn allow_any_foreground() {}
    pub fn set_clipboard(_text: &str) -> bool {
        false
    }
}

pub use imp::{allow_any_foreground, bring_to_front, center_on, set_clipboard, set_owner};

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
