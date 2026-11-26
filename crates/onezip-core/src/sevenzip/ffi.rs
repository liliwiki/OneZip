//! 7z.dll 的接口声明与加载。
//!
//! 接口、属性编号照 7-Zip 26.06 的源码（CPP/7zip/IStream.h、IPassword.h、IProgress.h、
//! Archive/IArchive.h、PropID.h）。7-Zip 的接口是 COM 风格的虚表，IID 统一为
//! `23170F69-40C1-278A-0000-00gg00nn0000`（gg 是组，nn 是编号），但它并不需要 COM
//! 初始化，也不走注册表：直接从 7z.dll 导出的 `CreateObject` 创建对象。

#![allow(non_snake_case, clippy::missing_safety_doc, dead_code)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use windows_core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, interface};

// ---- 接口 ----

#[interface("23170F69-40C1-278A-0000-000000050000")]
pub unsafe trait IProgress: IUnknown {
    pub unsafe fn SetTotal(&self, total: u64) -> HRESULT;
    pub unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000300010000")]
pub unsafe trait ISequentialInStream: IUnknown {
    pub unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000300020000")]
pub unsafe trait ISequentialOutStream: IUnknown {
    pub unsafe fn Write(&self, data: *const c_void, size: u32, processed: *mut u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000300030000")]
pub unsafe trait IInStream: ISequentialInStream {
    pub unsafe fn Seek(&self, offset: i64, origin: u32, new_position: *mut u64) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000500100000")]
pub unsafe trait ICryptoGetTextPassword: IUnknown {
    pub unsafe fn CryptoGetTextPassword(&self, password: *mut *mut u16) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600100000")]
pub unsafe trait IArchiveOpenCallback: IUnknown {
    pub unsafe fn SetTotal(&self, files: *const u64, bytes: *const u64) -> HRESULT;
    pub unsafe fn SetCompleted(&self, files: *const u64, bytes: *const u64) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600200000")]
pub unsafe trait IArchiveExtractCallback: IProgress {
    pub unsafe fn GetStream(&self, index: u32, out: *mut *mut c_void, ask_mode: i32) -> HRESULT;
    pub unsafe fn PrepareOperation(&self, ask_mode: i32) -> HRESULT;
    pub unsafe fn SetOperationResult(&self, result: i32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600300000")]
pub unsafe trait IArchiveOpenVolumeCallback: IUnknown {
    pub unsafe fn GetProperty(&self, prop: u32, value: *mut PropVariant) -> HRESULT;
    pub unsafe fn GetStream(&self, name: *const u16, stream: *mut *mut c_void) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600600000")]
pub unsafe trait IInArchive: IUnknown {
    pub unsafe fn Open(&self, stream: *mut c_void, max_check: *const u64, callback: *mut c_void) -> HRESULT;
    pub unsafe fn Close(&self) -> HRESULT;
    pub unsafe fn GetNumberOfItems(&self, n: *mut u32) -> HRESULT;
    pub unsafe fn GetProperty(&self, index: u32, prop: u32, value: *mut PropVariant) -> HRESULT;
    pub unsafe fn Extract(&self, indices: *const u32, n: u32, test: i32, callback: *mut c_void) -> HRESULT;
    pub unsafe fn GetArchiveProperty(&self, prop: u32, value: *mut PropVariant) -> HRESULT;
    pub unsafe fn GetNumberOfProperties(&self, n: *mut u32) -> HRESULT;
    pub unsafe fn GetPropertyInfo(&self, index: u32, name: *mut *mut u16, prop: *mut u32, vt: *mut u16) -> HRESULT;
    pub unsafe fn GetNumberOfArchiveProperties(&self, n: *mut u32) -> HRESULT;
    pub unsafe fn GetArchivePropertyInfo(&self, index: u32, name: *mut *mut u16, prop: *mut u32, vt: *mut u16) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600700000")]
pub unsafe trait IArchiveGetRawProps: IUnknown {
    pub unsafe fn GetParent(&self, index: u32, parent: *mut u32, parent_type: *mut u32) -> HRESULT;
    pub unsafe fn GetRawProp(&self, index: u32, prop: u32, data: *mut *const c_void, size: *mut u32, kind: *mut u32) -> HRESULT;
    pub unsafe fn GetNumRawProps(&self, n: *mut u32) -> HRESULT;
    pub unsafe fn GetRawPropInfo(&self, index: u32, name: *mut *mut u16, prop: *mut u32) -> HRESULT;
}

// ---- 压缩用的接口 ----

#[interface("23170F69-40C1-278A-0000-000300040000")]
pub unsafe trait IOutStream: ISequentialOutStream {
    pub unsafe fn Seek(&self, offset: i64, origin: u32, new_position: *mut u64) -> HRESULT;
    pub unsafe fn SetSize(&self, size: u64) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000500110000")]
pub unsafe trait ICryptoGetTextPassword2: IUnknown {
    pub unsafe fn CryptoGetTextPassword2(&self, defined: *mut i32, password: *mut *mut u16) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600030000")]
pub unsafe trait ISetProperties: IUnknown {
    pub unsafe fn SetProperties(&self, names: *const *const u16, values: *const PropVariant, n: u32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600800000")]
pub unsafe trait IArchiveUpdateCallback: IProgress {
    pub unsafe fn GetUpdateItemInfo(&self, index: u32, new_data: *mut i32, new_props: *mut i32, index_in_archive: *mut u32) -> HRESULT;
    pub unsafe fn GetProperty(&self, index: u32, prop: u32, value: *mut PropVariant) -> HRESULT;
    pub unsafe fn GetStream(&self, index: u32, stream: *mut *mut c_void) -> HRESULT;
    pub unsafe fn SetOperationResult(&self, result: i32) -> HRESULT;
}

#[interface("23170F69-40C1-278A-0000-000600A00000")]
pub unsafe trait IOutArchive: IUnknown {
    pub unsafe fn UpdateItems(&self, out: *mut c_void, n: u32, callback: *mut c_void) -> HRESULT;
    pub unsafe fn GetFileTimeType(&self, kind: *mut u32) -> HRESULT;
}

// ---- 常量 ----

pub mod kpid {
    pub const PATH: u32 = 3;
    pub const NAME: u32 = 4;
    pub const IS_DIR: u32 = 6;
    pub const SIZE: u32 = 7;
    pub const PACK_SIZE: u32 = 8;
    pub const ATTRIB: u32 = 9;
    pub const MTIME: u32 = 12;
    pub const SOLID: u32 = 13;
    pub const ENCRYPTED: u32 = 15;
    pub const BLOCK: u32 = 27;
    pub const POSITION: u32 = 29;
    pub const IS_ANTI: u32 = 21;
    pub const SYM_LINK: u32 = 54;
    pub const IS_ALT_STREAM: u32 = 63;
    pub const IS_AUX: u32 = 64;
    pub const IS_DELETED: u32 = 65;
    pub const SHA1: u32 = 67;
    pub const ERROR_FLAGS: u32 = 71;
    pub const PHY_SIZE: u32 = 44;
    pub const HARD_LINK: u32 = 90;
}

pub mod handler_prop {
    pub const NAME: u32 = 0;
    pub const CLASS_ID: u32 = 1;
    pub const EXTENSION: u32 = 2;
}

pub mod ask_mode {
    pub const EXTRACT: i32 = 0;
    pub const TEST: i32 = 1;
    pub const SKIP: i32 = 2;
}

pub mod op_result {
    pub const OK: i32 = 0;
    pub const UNSUPPORTED_METHOD: i32 = 1;
    pub const DATA_ERROR: i32 = 2;
    pub const CRC_ERROR: i32 = 3;
    pub const UNAVAILABLE: i32 = 4;
    pub const UNEXPECTED_END: i32 = 5;
    pub const DATA_AFTER_END: i32 = 6;
    pub const IS_NOT_ARC: i32 = 7;
    pub const HEADERS_ERROR: i32 = 8;
    pub const WRONG_PASSWORD: i32 = 9;
}

pub const E_ABORT: HRESULT = HRESULT(0x8000_4004_u32 as i32);
pub const S_FALSE: HRESULT = HRESULT(1);
pub const S_OK: HRESULT = HRESULT(0);
pub const E_FAIL: HRESULT = HRESULT(0x8000_4005_u32 as i32);
pub const E_INVALIDARG: HRESULT = HRESULT(0x8007_0057_u32 as i32);

// ---- PROPVARIANT ----

const VT_EMPTY: u16 = 0;
const VT_I4: u16 = 3;
const VT_BOOL: u16 = 11;
const VT_UI1: u16 = 17;
const VT_UI2: u16 = 18;
const VT_UI4: u16 = 19;
const VT_I8: u16 = 20;
const VT_UI8: u16 = 21;
const VT_UINT: u16 = 23;
const VT_BSTR: u16 = 8;
const VT_FILETIME: u16 = 64;

/// PROPVARIANT 的内存布局（x64：8 字节头 + 16 字节联合体）。7-Zip 只用到标量、
/// BSTR 和 FILETIME，所以不需要完整的 PROPVARIANT 支持。
#[repr(C)]
pub struct PropVariant {
    pub vt: u16,
    pub reserved: [u16; 3],
    pub data: [u64; 2],
}

impl Default for PropVariant {
    fn default() -> Self {
        Self {
            vt: VT_EMPTY,
            reserved: [0; 3],
            data: [0; 2],
        }
    }
}

impl Drop for PropVariant {
    fn drop(&mut self) {
        if self.vt == VT_BSTR && self.data[0] != 0 {
            unsafe { SysFreeString(self.data[0] as *mut u16) };
        }
    }
}

impl PropVariant {
    pub fn is_empty(&self) -> bool {
        self.vt == VT_EMPTY
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self.vt {
            VT_UI1 => Some(self.data[0] & 0xFF),
            VT_UI2 => Some(self.data[0] & 0xFFFF),
            VT_UI4 | VT_UINT => Some(self.data[0] & 0xFFFF_FFFF),
            VT_I4 => Some((self.data[0] as u32 as i32) as i64 as u64),
            VT_UI8 | VT_I8 => Some(self.data[0]),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        (self.vt == VT_BOOL).then(|| (self.data[0] & 0xFFFF) != 0)
    }

    pub fn as_filetime(&self) -> Option<u64> {
        (self.vt == VT_FILETIME).then_some(self.data[0])
    }

    pub fn as_string(&self) -> Option<String> {
        if self.vt != VT_BSTR || self.data[0] == 0 {
            return None;
        }
        Some(bstr_to_string(self.data[0] as *const u16))
    }

    /// BSTR 里装的原始字节（7-Zip 用它传 GUID、签名）。
    pub fn as_bytes(&self) -> Option<Vec<u8>> {
        if self.vt != VT_BSTR || self.data[0] == 0 {
            return None;
        }
        let p = self.data[0] as *const u8;
        let len = unsafe { SysStringByteLen(self.data[0] as *const u16) } as usize;
        Some(unsafe { std::slice::from_raw_parts(p, len) }.to_vec())
    }

    pub fn from_bool(b: bool) -> Self {
        Self { vt: VT_BOOL, reserved: [0; 3], data: [if b { 0xFFFF } else { 0 }, 0] }
    }

    pub fn from_u32(v: u32) -> Self {
        Self { vt: VT_UI4, reserved: [0; 3], data: [u64::from(v), 0] }
    }

    pub fn from_u64(v: u64) -> Self {
        Self { vt: VT_UI8, reserved: [0; 3], data: [v, 0] }
    }

    pub fn from_filetime(ft: u64) -> Self {
        Self { vt: VT_FILETIME, reserved: [0; 3], data: [ft, 0] }
    }

    /// 自己持有的 BSTR，drop 时释放。
    pub fn from_str(s: &str) -> Self {
        Self { vt: VT_BSTR, reserved: [0; 3], data: [alloc_bstr(s) as u64, 0] }
    }

    /// 写入一个字符串（给 7-Zip 的回调返回值用，所有权交给对方）。
    pub fn set_string(value: *mut PropVariant, s: &str) {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let b = unsafe { SysAllocStringLen(wide.as_ptr(), wide.len() as u32) };
        unsafe {
            std::ptr::write(
                value,
                PropVariant {
                    vt: VT_BSTR,
                    reserved: [0; 3],
                    data: [b as u64, 0],
                },
            )
        };
    }
}

pub fn bstr_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let len = unsafe { SysStringLen(p) } as usize;
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

/// 分配一个 BSTR（给 7-Zip 要密码时返回，由它释放）。
pub fn alloc_bstr(s: &str) -> *mut u16 {
    let wide: Vec<u16> = s.encode_utf16().collect();
    unsafe { SysAllocStringLen(wide.as_ptr(), wide.len() as u32) }
}

#[link(name = "oleaut32")]
unsafe extern "system" {
    fn SysAllocStringLen(s: *const u16, len: u32) -> *mut u16;
    fn SysFreeString(s: *mut u16);
    fn SysStringLen(s: *const u16) -> u32;
    fn SysStringByteLen(s: *const u16) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

// ---- 7z.dll ----

type CreateObjectFn = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type GetNumberOfFormatsFn = unsafe extern "system" fn(*mut u32) -> HRESULT;
type GetHandlerProperty2Fn = unsafe extern "system" fn(u32, u32, *mut PropVariant) -> HRESULT;

/// 7z.dll 里的一个格式处理器。
#[derive(Debug, Clone)]
pub struct Handler {
    pub name: String,
    pub class_id: GUID,
    /// 小写扩展名，不带点。
    pub extensions: Vec<String>,
}

pub struct Library {
    create_object: CreateObjectFn,
    pub handlers: Vec<Handler>,
    pub path: PathBuf,
}

// 函数指针和只读的格式表，可以跨线程共享。
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

/// 找 7z.dll：环境变量 `ONEZIP_7Z_DLL`，程序旁边，最后是源码树里的 third_party/7zip（开发和测试用）。
fn dll_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = std::env::var_os("ONEZIP_7Z_DLL") {
        v.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        v.push(dir.join("7z.dll"));
    }
    v.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/7zip/7z.dll"));
    v
}

static LIBRARY: OnceLock<std::result::Result<Library, String>> = OnceLock::new();

pub fn library() -> Result<&'static Library> {
    LIBRARY
        .get_or_init(|| load().map_err(|e| format!("{e:#}")))
        .as_ref()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn load() -> Result<Library> {
    let path = dll_candidates()
        .into_iter()
        .find(|p| p.is_file())
        .context("找不到 7z.dll（应在 OneZip.exe 旁边）")?;
    let wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    if module.is_null() {
        bail!("加载 {} 失败", path.display());
    }
    let sym = |name: &[u8]| {
        let p = unsafe { GetProcAddress(module, name.as_ptr()) };
        (!p.is_null()).then_some(p)
    };
    let create_object: CreateObjectFn = unsafe {
        std::mem::transmute(sym(b"CreateObject\0").context("7z.dll 缺少 CreateObject")?)
    };
    let get_count: GetNumberOfFormatsFn = unsafe {
        std::mem::transmute(sym(b"GetNumberOfFormats\0").context("7z.dll 缺少 GetNumberOfFormats")?)
    };
    let get_prop: GetHandlerProperty2Fn = unsafe {
        std::mem::transmute(sym(b"GetHandlerProperty2\0").context("7z.dll 缺少 GetHandlerProperty2")?)
    };

    let mut n = 0u32;
    unsafe { get_count(&mut n) }.ok()?;
    let mut handlers = Vec::new();
    for i in 0..n {
        let mut name = PropVariant::default();
        let mut id = PropVariant::default();
        let mut ext = PropVariant::default();
        unsafe {
            get_prop(i, handler_prop::NAME, &mut name).ok()?;
            get_prop(i, handler_prop::CLASS_ID, &mut id).ok()?;
            get_prop(i, handler_prop::EXTENSION, &mut ext).ok()?;
        }
        let Some(bytes) = id.as_bytes() else { continue };
        if bytes.len() != 16 {
            continue;
        }
        let class_id = GUID::from_values(
            u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
            u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
            bytes[8..16].try_into().unwrap(),
        );
        handlers.push(Handler {
            name: name.as_string().unwrap_or_default(),
            class_id,
            extensions: ext
                .as_string()
                .unwrap_or_default()
                .split_whitespace()
                .map(|e| e.to_ascii_lowercase())
                .collect(),
        });
    }
    Ok(Library {
        create_object,
        handlers,
        path,
    })
}

impl Library {
    pub fn handler(&self, name: &str) -> Option<&Handler> {
        self.handlers.iter().find(|h| h.name.eq_ignore_ascii_case(name))
    }

    /// 创建一个能写压缩包的格式处理器对象（7z、zip 等）。
    pub fn create_out(&self, handler: &Handler) -> Result<IOutArchive> {
        let mut obj: *mut c_void = std::ptr::null_mut();
        unsafe { (self.create_object)(&handler.class_id, &<IOutArchive as windows_core::Interface>::IID, &mut obj) }.ok()?;
        if obj.is_null() {
            bail!("7z.dll 不能写 {} 格式", handler.name);
        }
        Ok(unsafe { <IOutArchive as windows_core::Interface>::from_raw(obj) })
    }

    /// 创建一个格式处理器对象。
    pub fn create(&self, handler: &Handler) -> Result<IInArchive> {
        let mut obj: *mut c_void = std::ptr::null_mut();
        unsafe { (self.create_object)(&handler.class_id, &<IInArchive as windows_core::Interface>::IID, &mut obj) }.ok()?;
        if obj.is_null() {
            bail!("7z.dll 没有创建 {} 处理器", handler.name);
        }
        Ok(unsafe { <IInArchive as windows_core::Interface>::from_raw(obj) })
    }
}
