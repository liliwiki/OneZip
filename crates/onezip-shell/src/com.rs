//! COM 部分：类工厂、根命令、子命令、子命令枚举器。

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOTIMPL, E_POINTER, HMODULE, MAX_PATH,
    S_FALSE, S_OK,
};
use windows::Win32::System::Com::{
    CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows::Win32::UI::Shell::{
    ECF_DEFAULT, ECF_HASSUBCOMMANDS, ECF_ISSEPARATOR, ECS_ENABLED, ECS_HIDDEN, IEnumExplorerCommand,
    IEnumExplorerCommand_Impl, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray,
    SHStrDupW, SIGDN_FILESYSPATH,
};
use windows_core::{BOOL, GUID, HRESULT, HSTRING, Interface, PWSTR, Ref, Result, implement};

use crate::{CLSID_TEXT, Entry, MENU, visible};

static MODULE: OnceLock<usize> = OnceLock::new();

fn clsid() -> GUID {
    GUID::try_from(CLSID_TEXT).expect("CLSID")
}

/// OneZip.exe 与本 DLL 在同一目录。
fn exe_path() -> Option<PathBuf> {
    let module = HMODULE(*MODULE.get()? as *mut c_void);
    let mut buf = [0u16; MAX_PATH as usize * 4];
    let n = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    if n == 0 {
        return None;
    }
    let dll = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
    Some(dll.with_file_name("OneZip.exe"))
}

fn dup(s: &str) -> Result<PWSTR> {
    unsafe { SHStrDupW(&HSTRING::from(s)) }
}

/// 选中项的文件系统路径。
fn paths(items: Option<&IShellItemArray>) -> Vec<String> {
    let Some(items) = items else {
        return Vec::new();
    };
    let count = unsafe { items.GetCount() }.unwrap_or(0);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let Ok(item) = (unsafe { items.GetItemAt(i) }) else {
            continue;
        };
        if let Ok(name) = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) } {
            if let Ok(s) = unsafe { name.to_string() } {
                out.push(s);
            }
            unsafe { CoTaskMemFree(Some(name.0 as *const c_void)) };
        }
    }
    out
}

#[implement(IExplorerCommand)]
struct Root;

impl IExplorerCommand_Impl for Root_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        dup("OneZip")
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        match exe_path() {
            Some(p) => dup(&format!("{},0", p.display())),
            None => Err(E_NOTIMPL.into()),
        }
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(clsid())
    }

    fn GetState(&self, items: Ref<IShellItemArray>, _slow: BOOL) -> Result<u32> {
        // 没有可用路径（例如虚拟文件夹里的项）时不显示。
        if paths(items.as_ref()).is_empty() {
            Ok(ECS_HIDDEN.0 as u32)
        } else {
            Ok(ECS_ENABLED.0 as u32)
        }
    }

    fn Invoke(&self, _items: Ref<IShellItemArray>, _ctx: Ref<IBindCtx>) -> Result<()> {
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_HASSUBCOMMANDS.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        let subs: Vec<IExplorerCommand> = MENU.iter().map(|&entry| Sub { entry }.into()).collect();
        Ok(Enum {
            subs,
            pos: AtomicUsize::new(0),
        }
        .into())
    }
}

#[implement(IExplorerCommand)]
struct Sub {
    entry: Entry,
}

fn is_dir(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

impl IExplorerCommand_Impl for Sub_Impl {
    fn GetTitle(&self, items: Ref<IShellItemArray>) -> Result<PWSTR> {
        match self.entry {
            Entry::Separator(_) => dup(""),
            Entry::Command(c) => dup(&c.title(&paths(items.as_ref()), is_dir)),
        }
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Err(E_NOTIMPL.into())
    }

    fn GetState(&self, items: Ref<IShellItemArray>, _slow: BOOL) -> Result<u32> {
        let show = visible(self.entry, &paths(items.as_ref()));
        Ok(if show { ECS_ENABLED.0 } else { ECS_HIDDEN.0 } as u32)
    }

    fn Invoke(&self, items: Ref<IShellItemArray>, _ctx: Ref<IBindCtx>) -> Result<()> {
        let Entry::Command(command) = self.entry else {
            return Ok(());
        };
        let paths = paths(items.as_ref());
        if paths.is_empty() {
            return Ok(());
        }
        let Some(exe) = exe_path() else {
            return Err(E_POINTER.into());
        };
        // 资源管理器把前台权限交给了我们；转交给 OneZip.exe，它的窗口才能出现在最前面，
        // 而不是被压在资源管理器后面。
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }
        std::process::Command::new(exe)
            .args(command.args())
            .args(&paths)
            .spawn()
            .map_err(|e| {
                windows_core::Error::new(
                    HRESULT::from_win32(e.raw_os_error().unwrap_or(0) as u32),
                    "OneZip.exe",
                )
            })?;
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(match self.entry {
            Entry::Separator(_) => ECF_ISSEPARATOR.0 as u32,
            Entry::Command(_) => ECF_DEFAULT.0 as u32,
        })
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

#[implement(IEnumExplorerCommand)]
struct Enum {
    subs: Vec<IExplorerCommand>,
    pos: AtomicUsize,
}

impl IEnumExplorerCommand_Impl for Enum_Impl {
    fn Next(&self, celt: u32, out: *mut Option<IExplorerCommand>, fetched: *mut u32) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        let mut n = 0u32;
        while n < celt {
            let i = self.pos.fetch_add(1, Ordering::SeqCst);
            let Some(cmd) = self.subs.get(i) else {
                self.pos.store(self.subs.len(), Ordering::SeqCst);
                break;
            };
            unsafe { out.add(n as usize).write(Some(cmd.clone())) };
            n += 1;
        }
        if !fetched.is_null() {
            unsafe { fetched.write(n) };
        }
        if n == celt { S_OK } else { S_FALSE }
    }

    fn Skip(&self, celt: u32) -> Result<()> {
        self.pos.fetch_add(celt as usize, Ordering::SeqCst);
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        self.pos.store(0, Ordering::SeqCst);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumExplorerCommand> {
        Ok(Enum {
            subs: self.subs.clone(),
            pos: AtomicUsize::new(self.pos.load(Ordering::SeqCst)),
        }
        .into())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<windows_core::IUnknown>,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        if riid.is_null() || object.is_null() {
            return Err(E_POINTER.into());
        }
        let root: IExplorerCommand = Root.into();
        unsafe { root.query(riid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        let _ = MODULE.set(module.0 as usize);
    }
    true.into()
}

#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = std::ptr::null_mut() };
    if unsafe { *rclsid } != clsid() {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory.into();
    unsafe { factory.query(riid, ppv) }
}

#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    // 简单起见一直留在内存里；菜单扩展很小。
    S_FALSE
}
