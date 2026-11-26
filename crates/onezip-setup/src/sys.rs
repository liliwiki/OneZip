//! 安装 / 卸载要用到的 Windows 功能，都直接调系统接口，不经过 PowerShell。

use onezip_i18n::t;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W, Process32FirstW,
    Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW,
    TerminateProcess, WaitForSingleObject,
};
use windows::core::{HSTRING, PCWSTR, PWSTR, w};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\OneZip";
pub const PACKAGE_NAME: &str = "OneZip.ShellExtension";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------- 进程

struct Proc {
    pid: u32,
    name: String,
}

fn processes() -> Vec<Proc> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push(Proc { pid: e.th32ProcessID, name: String::from_utf16_lossy(&e.szExeFile[..len]) });
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 32_768];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// 进程加载的模块里有没有满足 `wanted` 的。
fn has_module(pid: u32, wanted: impl Fn(&Path) -> bool) -> bool {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) else { return false };
        let mut e = MODULEENTRY32W { dwSize: size_of::<MODULEENTRY32W>() as u32, ..Default::default() };
        let mut ok = Module32FirstW(snap, &mut e).is_ok();
        let mut found = false;
        while ok && !found {
            let len = e.szExePath.iter().position(|&c| c == 0).unwrap_or(e.szExePath.len());
            found = wanted(Path::new(&String::from_utf16_lossy(&e.szExePath[..len])));
            ok = Module32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

pub fn in_dir(path: &Path, dir: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    path.parent().is_some_and(|p| norm(p) == norm(dir))
}

/// 从 `dirs` 里运行的 OneZip.exe / onezipc.exe。
pub fn onezip_running(dirs: &[PathBuf]) -> Vec<u32> {
    processes()
        .into_iter()
        .filter(|p| p.name.eq_ignore_ascii_case("OneZip.exe") || p.name.eq_ignore_ascii_case("onezipc.exe"))
        .filter(|p| image_path(p.pid).is_some_and(|img| dirs.iter().any(|d| in_dir(&img, d))))
        .map(|p| p.pid)
        .collect()
}

fn open(pid: u32) -> Option<HANDLE> {
    unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid).ok() }
}

pub fn kill(pid: u32) {
    if let Some(h) = open(pid) {
        unsafe {
            let _ = TerminateProcess(h, 1);
            WaitForSingleObject(h, 5000);
            let _ = CloseHandle(h);
        }
    }
}

/// 等 `dirs` 里的 OneZip 自己退出（更新时 OneZip 启动安装程序后马上退出），最多等 `limit`，
/// 还没退出的结束掉。
pub fn close_onezip(dirs: &[PathBuf], limit: Duration) {
    let start = Instant::now();
    while !onezip_running(dirs).is_empty() && start.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(200));
    }
    for pid in onezip_running(dirs) {
        kill(pid);
    }
}

/// 结束加载了 onezip_shell.dll 的 dllhost（资源管理器用它跑右键菜单）。不先结束它，
/// DLL 换不掉，旧的稀疏包也卸不掉（一直"DeploymentInProgress"）。资源管理器下次右键时会重新加载。
pub fn release_menu_dll() {
    for p in processes().into_iter().filter(|p| p.name.eq_ignore_ascii_case("dllhost.exe")) {
        let loaded = has_module(p.pid, |m| {
            m.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("onezip_shell.dll"))
        });
        if loaded {
            kill(p.pid);
        }
    }
}

/// 运行程序（不显示窗口）并等它结束，返回退出码。
pub fn run_hidden(exe: &Path, args: &[&str]) -> Result<i32> {
    use std::os::windows::process::CommandExt;
    let status = Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .with_context(|| t!("运行 {}", exe.display()))?;
    Ok(status.code().unwrap_or(-1))
}

// ---------------------------------------------------------------- 管理员权限

/// 以管理员身份运行本程序（弹 UAC），等它结束。用户拒绝时返回错误。
pub fn run_elevated(args: &[String]) -> Result<()> {
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let exe = HSTRING::from(std::env::current_exe()?.as_os_str());
    let params = HSTRING::from(args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|_| anyhow!(t!("没有得到管理员确认")))?;
        if info.hProcess.is_invalid() {
            bail!(t!("没有得到管理员确认"));
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if code != 0 {
            bail!(t!("管理员操作没有成功（{}）", code));
        }
    }
    Ok(())
}

/// 命令行参数加引号（路径里不会有引号；结尾的反斜杠要成对，否则会转义掉引号）。
fn quote(a: &str) -> String {
    if a.ends_with('\\') { format!("\"{a}\\\"") } else { format!("\"{a}\"") }
}

/// 当前用户的 SID，如 `S-1-5-21-…`。
pub fn user_sid() -> Result<String> {
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let mut need = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut need);
        let mut buf = vec![0u8; need as usize];
        let r = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), need, &mut need);
        let _ = CloseHandle(token);
        r?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s)?;
        let text = s.to_string()?;
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        Ok(text)
    }
}

/// （管理员身份）建好文件夹并交给 `sid` 修改，以后升级、卸载都不用管理员。
pub fn grant(dir: &Path, sid: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let icacls = system32("icacls.exe");
    let code = run_hidden(&icacls, &[&dir.display().to_string(), "/grant", &format!("*{sid}:(OI)(CI)M")])?;
    if code != 0 {
        bail!(t!("icacls 失败（{}）", code));
    }
    Ok(())
}

fn system32(name: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    root.join("System32").join(name)
}

// ---------------------------------------------------------------- 证书

mod cert {
    use onezip_i18n::t;
    use std::ffi::c_void;

    use anyhow::{Result, bail};
    use windows::Win32::Security::Cryptography::{
        CERT_CONTEXT, CERT_FIND_EXISTING, CERT_OPEN_STORE_FLAGS, CERT_QUERY_ENCODING_TYPE, CERT_STORE_ADD_REPLACE_EXISTING,
        CERT_STORE_PROV_SYSTEM_W, CERT_STORE_READONLY_FLAG, CERT_SYSTEM_STORE_LOCAL_MACHINE, CertAddCertificateContextToStore,
        CertCloseStore, CertCreateCertificateContext, CertFindCertificateInStore, CertFreeCertificateContext, CertOpenStore,
        HCERTSTORE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    use windows::core::w;

    fn encoding() -> CERT_QUERY_ENCODING_TYPE {
        CERT_QUERY_ENCODING_TYPE(X509_ASN_ENCODING.0 | PKCS_7_ASN_ENCODING.0)
    }

    fn open_store(write: bool) -> Result<HCERTSTORE> {
        let mut flags = CERT_SYSTEM_STORE_LOCAL_MACHINE;
        if !write {
            flags |= CERT_STORE_READONLY_FLAG.0;
        }
        let store = unsafe {
            CertOpenStore(
                CERT_STORE_PROV_SYSTEM_W,
                CERT_QUERY_ENCODING_TYPE(0),
                None,
                CERT_OPEN_STORE_FLAGS(flags),
                Some(w!("TrustedPeople").as_ptr() as *const c_void),
            )?
        };
        Ok(store)
    }

    fn context(der: &[u8]) -> Result<*mut CERT_CONTEXT> {
        let ctx = unsafe { CertCreateCertificateContext(encoding(), der) };
        if ctx.is_null() {
            bail!(t!("证书文件看不懂"));
        }
        Ok(ctx)
    }

    /// 本机"受信任的人"里有没有这张证书（普通权限就能看）。
    pub fn trusted(der: &[u8]) -> Result<bool> {
        let ctx = context(der)?;
        let store = open_store(false)?;
        unsafe {
            let found = CertFindCertificateInStore(store, encoding(), 0, CERT_FIND_EXISTING, Some(ctx as *const c_void), None);
            let yes = !found.is_null();
            if yes {
                let _ = CertFreeCertificateContext(Some(found));
            }
            let _ = CertFreeCertificateContext(Some(ctx));
            let _ = CertCloseStore(Some(store), 0);
            Ok(yes)
        }
    }

    /// （管理员身份）把证书加进本机"受信任的人"。
    pub fn trust(der: &[u8]) -> Result<()> {
        let ctx = context(der)?;
        let store = open_store(true)?;
        unsafe {
            let r = CertAddCertificateContextToStore(Some(store), ctx, CERT_STORE_ADD_REPLACE_EXISTING, None);
            let _ = CertFreeCertificateContext(Some(ctx));
            let _ = CertCloseStore(Some(store), 0);
            r?;
        }
        Ok(())
    }
}

pub use cert::{trust, trusted};

// ---------------------------------------------------------------- 稀疏包（Windows 11 第一层右键菜单）

/// 注册稀疏包，程序文件在 `external`（安装目录）。
pub fn add_shell_package(msix: &Path, external: &Path) -> Result<()> {
    use windows::Foundation::Uri;
    use windows::Management::Deployment::{AddPackageOptions, PackageManager};
    let pm = PackageManager::new()?;
    let options = AddPackageOptions::new()?;
    options.SetExternalLocationUri(&Uri::CreateUri(&HSTRING::from(file_uri(external)))?)?;
    options.SetForceUpdateFromAnyVersion(true)?;
    let result = pm.AddPackageByUriAsync(&Uri::CreateUri(&HSTRING::from(file_uri(msix)))?, &options)?.join()?;
    let code = result.ExtendedErrorCode()?;
    if code.is_err() {
        bail!(t!("注册右键菜单失败：{}（{}）", result.ErrorText()?, code));
    }
    Ok(())
}

/// 卸载当前用户的 OneZip 稀疏包（没有就什么也不做）。
pub fn remove_shell_package() -> Result<()> {
    use windows::Management::Deployment::PackageManager;
    let pm = PackageManager::new()?;
    let mut names = Vec::new();
    for p in pm.FindPackagesByUserSecurityId(&HSTRING::new())? {
        let id = p.Id()?;
        if id.Name()? == PACKAGE_NAME {
            names.push(id.FullName()?);
        }
    }
    for full in names {
        let result = pm.RemovePackageAsync(&full)?.join()?;
        let code = result.ExtendedErrorCode()?;
        if code.is_err() {
            bail!(t!("卸载旧的右键菜单失败：{}（{}）", result.ErrorText()?, code));
        }
    }
    Ok(())
}

fn file_uri(p: &Path) -> String {
    format!("file:///{}", p.display().to_string().replace('\\', "/"))
}

// ---------------------------------------------------------------- 开始菜单、注册表

fn programs_folder() -> Result<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Programs, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None)?;
        let s = p.to_string();
        CoTaskMemFree(Some(p.0 as *const _));
        Ok(PathBuf::from(s?))
    }
}

pub fn shortcut_path() -> Result<PathBuf> {
    Ok(programs_folder()?.join("OneZip.lnk"))
}

/// 开始菜单里的 OneZip 快捷方式。
pub fn create_shortcut(exe: &Path) -> Result<()> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::Interface;
    let lnk = shortcut_path()?;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        if let Some(dir) = exe.parent() {
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        link.cast::<IPersistFile>()?.Save(&HSTRING::from(lnk.as_os_str()), true)?;
    }
    Ok(())
}

/// 本线程要用 COM（快捷方式、稀疏包）。
pub fn init_com() {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// "应用和功能"里的卸载项。
pub fn uninstall_key() -> Option<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY).ok()
}

pub fn write_uninstall_key(dest: &Path, version: &str, files: &[String], size_kb: u32) -> Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY)?;
    let exe = dest.join("OneZip.exe");
    let uninstall = dest.join("uninstall.exe");
    key.set_value("DisplayName", &"OneZip")?;
    key.set_value("DisplayIcon", &format!("{},0", exe.display()))?;
    key.set_value("DisplayVersion", &version)?;
    key.set_value("Publisher", &"OneZip")?;
    key.set_value("InstallLocation", &dest.display().to_string())?;
    key.set_value("UninstallString", &format!("\"{}\"", uninstall.display()))?;
    key.set_value("URLInfoAbout", &"https://github.com/liliwiki/OneZip")?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    key.set_value("EstimatedSize", &size_kb)?;
    key.set_value("InstalledFiles", &files.to_vec())?;
    Ok(())
}

pub fn delete_uninstall_key() {
    let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY);
}

/// 内部版本 22000 起是 Windows 11。RtlGetVersion 不受兼容性清单影响，总是返回真实版本。
pub fn is_win11() -> bool {
    #[repr(C)]
    struct OsVersionInfoW {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }
    let mut info = OsVersionInfoW { size: size_of::<OsVersionInfoW>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
    // SAFETY: info 是按 RTL_OSVERSIONINFOW 布局初始化好的结构，size 字段已填。
    unsafe { RtlGetVersion(&mut info) == 0 && info.build >= 22000 }
}

/// 卸载程序删不掉正在运行的自己：等它退出后由 cmd 删掉，再删掉空了的安装目录。
pub fn delete_after_exit(exe: &Path, dir: &Path) {
    use std::os::windows::process::CommandExt;
    let script = format!(
        "/c ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"",
        exe.display(),
        dir.display()
    );
    let _ = Command::new(system32("cmd.exe"))
        .raw_arg(script)
        .current_dir(std::env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
