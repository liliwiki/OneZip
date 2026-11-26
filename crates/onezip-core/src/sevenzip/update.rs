//! 用 7z.dll 压缩成 7z 或 zip（和 7-Zip 同一个编码器，多线程更快）。

#![allow(non_snake_case)]

use onezip_i18n::t;
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use windows_core::{HRESULT, Interface, implement};

use super::ffi::*;
use crate::{Event, Observer};

/// 要放进压缩包的一项。
pub struct Item {
    /// 压缩包内的名字，用 `\` 分隔。
    pub name: String,
    pub path: PathBuf,
    pub dir: bool,
    pub size: u64,
    /// FILETIME（UTC，100 纳秒）。
    pub mtime: Option<u64>,
    /// Windows 文件属性。
    pub attrib: u32,
}

pub struct Options {
    /// 0（只存储）到 9。
    pub level: u32,
    /// 0 表示让 7z.dll 按 CPU 核数决定。
    pub threads: u32,
    /// 7z：固实压缩。
    pub solid: bool,
    pub password: Option<String>,
    /// 7z：连文件名一起加密。
    pub encrypt_names: bool,
}

/// 把 `items` 压缩成 `format`（"7z" 或 "zip"）格式，写到 `out`。
pub fn compress(
    out: &Path,
    format: &str,
    items: Vec<Item>,
    opts: &Options,
    observer: &Observer,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    let lib = library()?;
    let handler = lib.handler(format).with_context(|| t!("7z.dll 不支持 {} 格式", format))?;
    let arc = lib.create_out(handler)?;
    set_properties(&arc, format, opts)?;

    let file = Arc::new(Mutex::new(File::create(out).with_context(|| t!("创建 {}", out.display()))?));
    let stream: IOutStream = OutFile { file: file.clone() }.into();
    let total: u64 = items.iter().filter(|i| !i.dir).map(|i| i.size).sum();
    let n = items.len() as u32;
    let state = Arc::new(Mutex::new(State { reported: 0, total, error: None }));
    let cb: IArchiveUpdateCallback = UpdateCallback {
        items: Arc::new(items),
        password: opts.password.clone(),
        observer: observer.clone(),
        stop: stop.clone(),
        state: state.clone(),
    }
    .into();
    let hr = unsafe { arc.UpdateItems(stream.as_raw(), n, cb.as_raw()) };
    drop(cb);
    drop(stream);
    if let Some(e) = state.lock().unwrap().error.take() {
        return Err(e);
    }
    if stop.load(Ordering::Relaxed) || hr == E_ABORT {
        bail!(crate::compress::Cancelled);
    }
    hr.ok().map_err(|e| anyhow!(t!("7z.dll 压缩失败：{}", e)))?;
    file.lock().unwrap().sync_all()?;
    Ok(())
}

/// 压缩参数，名字和 7-Zip 命令行的 `-m` 开关一样。
fn set_properties(arc: &IOutArchive, format: &str, opts: &Options) -> Result<()> {
    let mut props: Vec<(&str, PropVariant)> = vec![("x", PropVariant::from_u32(opts.level.min(9)))];
    if opts.threads > 0 {
        props.push(("mt", PropVariant::from_u32(opts.threads)));
    }
    if format == "7z" {
        props.push(("s", PropVariant::from_bool(opts.solid)));
        if opts.password.is_some() {
            props.push(("he", PropVariant::from_bool(opts.encrypt_names)));
        }
    } else {
        // 文件名一律用 UTF-8（不随系统代码页），加密用 AES-256（7-Zip 默认的 ZipCrypto 不安全）。
        props.push(("cu", PropVariant::from_bool(true)));
        if opts.password.is_some() {
            props.push(("em", PropVariant::from_str("AES256")));
        }
    }
    let names: Vec<Vec<u16>> = props.iter().map(|(n, _)| n.encode_utf16().chain(Some(0)).collect()).collect();
    let name_ptrs: Vec<*const u16> = names.iter().map(|n| n.as_ptr()).collect();
    let values: Vec<PropVariant> = props.into_iter().map(|(_, v)| v).collect();
    let setter: ISetProperties = arc.cast().context(t!("7z.dll 不接受压缩参数"))?;
    unsafe { setter.SetProperties(name_ptrs.as_ptr(), values.as_ptr(), values.len() as u32) }
        .ok()
        .map_err(|e| anyhow!(t!("7z.dll 不接受压缩参数：{}", e)))
}

// ---- 输出文件 ----

#[implement(IOutStream, ISequentialOutStream)]
struct OutFile {
    file: Arc<Mutex<File>>,
}

impl ISequentialOutStream_Impl for OutFile_Impl {
    unsafe fn Write(&self, data: *const c_void, size: u32, processed: *mut u32) -> HRESULT {
        let buf = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
        if self.file.lock().unwrap().write_all(buf).is_err() {
            return E_FAIL;
        }
        if !processed.is_null() {
            unsafe { *processed = size };
        }
        S_OK
    }
}

impl IOutStream_Impl for OutFile_Impl {
    unsafe fn Seek(&self, offset: i64, origin: u32, new_position: *mut u64) -> HRESULT {
        let pos = match origin {
            0 if offset >= 0 => SeekFrom::Start(offset as u64),
            0 => return HRESULT(0x8007_0083_u32 as i32), // ERROR_NEGATIVE_SEEK
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return E_INVALIDARG,
        };
        match self.file.lock().unwrap().seek(pos) {
            Ok(p) => {
                if !new_position.is_null() {
                    unsafe { *new_position = p };
                }
                S_OK
            }
            Err(_) => E_FAIL,
        }
    }

    unsafe fn SetSize(&self, size: u64) -> HRESULT {
        if self.file.lock().unwrap().set_len(size).is_err() { E_FAIL } else { S_OK }
    }
}

// ---- 输入文件 ----

#[implement(ISequentialInStream)]
struct InFile {
    file: Mutex<File>,
    stop: Arc<AtomicBool>,
}

impl ISequentialInStream_Impl for InFile_Impl {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        if self.stop.load(Ordering::Relaxed) {
            return E_ABORT;
        }
        let buf = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, size as usize) };
        let n = match self.file.lock().unwrap().read(buf) {
            Ok(n) => n,
            Err(_) => return E_FAIL,
        };
        if !processed.is_null() {
            unsafe { *processed = n as u32 };
        }
        S_OK
    }
}

// ---- 更新回调 ----

struct State {
    /// 已经报告给界面的字节数。
    reported: u64,
    total: u64,
    error: Option<anyhow::Error>,
}

#[implement(IArchiveUpdateCallback, IProgress, ICryptoGetTextPassword2)]
struct UpdateCallback {
    items: Arc<Vec<Item>>,
    password: Option<String>,
    observer: Observer,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
}

impl IProgress_Impl for UpdateCallback_Impl {
    unsafe fn SetTotal(&self, _total: u64) -> HRESULT {
        S_OK
    }

    unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT {
        if self.stop.load(Ordering::Relaxed) {
            return E_ABORT;
        }
        if !complete.is_null() {
            // 7z.dll 报告的是已经读入的字节数（累计）；不超过总量。
            let done = unsafe { *complete };
            let mut st = self.state.lock().unwrap();
            let done = done.min(st.total);
            if done > st.reported {
                let delta = done - st.reported;
                st.reported = done;
                drop(st);
                (self.observer)(Event::Progressed(delta));
            }
        }
        S_OK
    }
}

impl IArchiveUpdateCallback_Impl for UpdateCallback_Impl {
    unsafe fn GetUpdateItemInfo(&self, _index: u32, new_data: *mut i32, new_props: *mut i32, index_in_archive: *mut u32) -> HRESULT {
        unsafe {
            if !new_data.is_null() {
                *new_data = 1;
            }
            if !new_props.is_null() {
                *new_props = 1;
            }
            if !index_in_archive.is_null() {
                *index_in_archive = u32::MAX;
            }
        }
        S_OK
    }

    unsafe fn GetProperty(&self, index: u32, prop: u32, value: *mut PropVariant) -> HRESULT {
        let Some(item) = self.items.get(index as usize) else { return E_INVALIDARG };
        let v = match prop {
            kpid::PATH => {
                PropVariant::set_string(value, &item.name);
                return S_OK;
            }
            kpid::IS_DIR => PropVariant::from_bool(item.dir),
            kpid::SIZE if !item.dir => PropVariant::from_u64(item.size),
            kpid::ATTRIB => PropVariant::from_u32(item.attrib),
            kpid::MTIME => match item.mtime {
                Some(t) => PropVariant::from_filetime(t),
                None => PropVariant::default(),
            },
            kpid::IS_ANTI => PropVariant::from_bool(false),
            _ => PropVariant::default(),
        };
        unsafe { std::ptr::write(value, v) };
        S_OK
    }

    unsafe fn GetStream(&self, index: u32, stream: *mut *mut c_void) -> HRESULT {
        if self.stop.load(Ordering::Relaxed) {
            return E_ABORT;
        }
        let Some(item) = self.items.get(index as usize) else { return E_INVALIDARG };
        if item.dir {
            return S_OK;
        }
        match File::open(&item.path) {
            Ok(file) => {
                let s: ISequentialInStream = InFile { file: Mutex::new(file), stop: self.stop.clone() }.into();
                unsafe { *stream = s.into_raw() };
                S_OK
            }
            Err(e) => {
                self.state.lock().unwrap().error = Some(anyhow!(t!("压缩 {}：{}", item.path.display(), e)));
                E_FAIL
            }
        }
    }

    unsafe fn SetOperationResult(&self, _result: i32) -> HRESULT {
        S_OK
    }
}

impl ICryptoGetTextPassword2_Impl for UpdateCallback_Impl {
    unsafe fn CryptoGetTextPassword2(&self, defined: *mut i32, password: *mut *mut u16) -> HRESULT {
        unsafe {
            *defined = i32::from(self.password.is_some());
            *password = alloc_bstr(self.password.as_deref().unwrap_or(""));
        }
        S_OK
    }
}
