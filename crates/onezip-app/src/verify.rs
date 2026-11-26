//! 看保存的密码之前，先确认是电脑的主人：设置了 Windows Hello（PIN、指纹、人脸）就用它，
//! 否则输入 Windows 登录密码。和浏览器"显示已保存的密码"一样。

/// 在后台线程里验证，结果回到界面线程交给 `done`（`true` 是验证通过）。
pub fn verify_user(window: &slint::Window, message: &str, done: impl FnOnce(bool) + Send + 'static) {
    let owner = imp::owner(window);
    let message = message.to_string();
    std::thread::spawn(move || {
        let ok = imp::verify(owner, &message);
        let _ = slint::invoke_from_event_loop(move || done(ok));
    });
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use windows_future::IAsyncOperation;
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Win32::Foundation::{CloseHandle, ERROR_ACCOUNT_RESTRICTION, ERROR_LOGON_FAILURE, HANDLE, HWND};
    use windows::Win32::Security::Credentials::{
        CRED_PACK_PROTECTED_CREDENTIALS, CREDUI_INFOW, CREDUIWIN_ENUMERATE_CURRENT_USER,
        CredUIPromptForWindowsCredentialsW, CredUnPackAuthenticationBufferW,
    };
    use windows::Win32::Security::{LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, LogonUserW};
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::System::WinRT::{IUserConsentVerifierInterop, RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::{HSTRING, PCWSTR, PWSTR};

    /// 窗口句柄（跨线程传数字）。
    pub fn owner(window: &slint::Window) -> isize {
        crate::win::hwnd(window).map_or(0, |h| h.0 as isize)
    }

    pub fn verify(owner: isize, message: &str) -> bool {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let hwnd = HWND(owner as *mut c_void);
        match hello(hwnd, message) {
            Some(ok) => ok,
            None => windows_password(hwnd, message),
        }
    }

    /// Windows Hello；没设置或用不了时返回 `None`。
    fn hello(hwnd: HWND, message: &str) -> Option<bool> {
        let available = UserConsentVerifier::CheckAvailabilityAsync().ok()?.join().ok()?;
        if available != UserConsentVerifierAvailability::Available {
            return None;
        }
        let interop = windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>().ok()?;
        let op: IAsyncOperation<UserConsentVerificationResult> =
            unsafe { interop.RequestVerificationForWindowAsync(hwnd, &HSTRING::from(message)) }.ok()?;
        Some(op.join().ok()? == UserConsentVerificationResult::Verified)
    }

    /// Windows 的凭据框，只列当前用户；密码不对就提示后再问，取消返回 `false`。
    fn windows_password(hwnd: HWND, message: &str) -> bool {
        let caption = HSTRING::from("OneZip");
        let text = HSTRING::from(message);
        let info = CREDUI_INFOW {
            cbSize: size_of::<CREDUI_INFOW>() as u32,
            hwndParent: hwnd,
            pszMessageText: PCWSTR(text.as_ptr()),
            pszCaptionText: PCWSTR(caption.as_ptr()),
            ..Default::default()
        };
        let mut error = 0;
        loop {
            let mut package = 0u32;
            let mut buf: *mut c_void = std::ptr::null_mut();
            let mut len = 0u32;
            let r = unsafe {
                CredUIPromptForWindowsCredentialsW(
                    Some(&info),
                    error,
                    &mut package,
                    None,
                    0,
                    &mut buf,
                    &mut len,
                    None,
                    CREDUIWIN_ENUMERATE_CURRENT_USER,
                )
            };
            if r != 0 {
                return false;
            }
            let ok = logon(buf, len);
            unsafe {
                std::ptr::write_bytes(buf.cast::<u8>(), 0, len as usize);
                CoTaskMemFree(Some(buf));
            }
            if ok {
                return true;
            }
            error = ERROR_LOGON_FAILURE.0;
        }
    }

    /// 凭据框给回的用户名和密码能不能登录。
    fn logon(buf: *const c_void, len: u32) -> bool {
        let mut user = [0u16; 514];
        let mut domain = [0u16; 338];
        let mut password = [0u16; 514];
        let (mut ul, mut dl, mut pl) = (user.len() as u32, domain.len() as u32, password.len() as u32);
        let unpacked = unsafe {
            CredUnPackAuthenticationBufferW(
                CRED_PACK_PROTECTED_CREDENTIALS,
                buf,
                len,
                Some(PWSTR(user.as_mut_ptr())),
                &mut ul,
                Some(PWSTR(domain.as_mut_ptr())),
                Some(&mut dl),
                Some(PWSTR(password.as_mut_ptr())),
                &mut pl,
            )
        };
        let ok = unpacked.is_ok() && {
            let mut token = HANDLE::default();
            let domain = if domain[0] == 0 { PCWSTR::null() } else { PCWSTR(domain.as_ptr()) };
            match unsafe {
                LogonUserW(
                    PCWSTR(user.as_ptr()),
                    domain,
                    PCWSTR(password.as_ptr()),
                    LOGON32_LOGON_INTERACTIVE,
                    LOGON32_PROVIDER_DEFAULT,
                    &mut token,
                )
            } {
                Ok(()) => {
                    unsafe {
                        let _ = CloseHandle(token);
                    }
                    true
                }
                // 账户没有设密码（空密码只允许在本机登录时用）：已经是登录状态，算通过。
                Err(e) => e.code() == ERROR_ACCOUNT_RESTRICTION.to_hresult(),
            }
        };
        password.fill(0);
        ok
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn owner(_window: &slint::Window) -> isize {
        0
    }
    pub fn verify(_owner: isize, _message: &str) -> bool {
        false
    }
}
