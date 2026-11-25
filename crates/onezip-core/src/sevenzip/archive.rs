//! 用 7z.dll 打开、列出、解出压缩包。

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use windows_core::{AsImpl, HRESULT, Interface, implement};

use super::fetch::Src;
use super::ffi::*;

/// 7z.dll 报告的一个条目。
#[derive(Debug, Clone)]
pub struct Item {
    pub index: u32,
    /// 用 `/` 分隔、已经清理过的相对路径（不安全的名字会被改掉）。
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<u64>,
    /// 7z.dll 给的"块"号（CAB 是文件夹序号）。
    pub block: Option<u64>,

    /// 备用数据流、辅助条目等：不解出来。
    pub skip: bool,
}

/// 需要密码（没给）。
#[derive(Debug)]
pub struct NeedPassword;
impl std::fmt::Display for NeedPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("压缩包已加密，需要密码")
    }
}
impl std::error::Error for NeedPassword {}

/// 密码不对。
#[derive(Debug)]
pub struct BadPassword;
impl std::fmt::Display for BadPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("密码不对")
    }
}
impl std::error::Error for BadPassword {}

/// 被叫停。
#[derive(Debug)]
pub struct Stopped;
impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("已停止")
    }
}
impl std::error::Error for Stopped {}

// ---- 输入流 ----

/// 给 7z.dll 读的文件。`zero` 里的区间按全零返回（边解边删时这些地方已经打洞释放）。
#[implement(IInStream, ISequentialInStream)]
struct InStream {
    inner: Mutex<(Src, u64)>,
    len: u64,
}

impl InStream {
    fn open(path: &Path) -> Result<IInStream> {
        let file = Src::open(path)?;
        let len = file.len()?;
        Ok(InStream {
            inner: Mutex::new((file, 0)),
            len,
        }
        .into())
    }
}

impl ISequentialInStream_Impl for InStream_Impl {
    unsafe fn Read(&self, data: *mut c_void, size: u32, processed: *mut u32) -> HRESULT {
        let mut g = self.inner.lock().unwrap();
        let (file, pos) = &mut *g;
        let buf = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, size as usize) };
        let n = match file.read(buf) {
            Ok(n) => n,
            Err(_) => return E_FAIL,
        };
        *pos += n as u64;
        if !processed.is_null() {
            unsafe { *processed = n as u32 };
        }
        S_OK
    }
}

impl IInStream_Impl for InStream_Impl {
    unsafe fn Seek(&self, offset: i64, origin: u32, new_position: *mut u64) -> HRESULT {
        let mut g = self.inner.lock().unwrap();
        let (file, pos) = &mut *g;
        let target = match origin {
            0 => offset,
            1 => *pos as i64 + offset,
            2 => self.len as i64 + offset,
            _ => return E_INVALIDARG,
        };
        if target < 0 {
            return HRESULT(0x8007_0083_u32 as i32); // ERROR_NEGATIVE_SEEK
        }
        if file.seek(SeekFrom::Start(target as u64)).is_err() {
            return E_FAIL;
        }
        *pos = target as u64;
        if !new_position.is_null() {
            unsafe { *new_position = target as u64 };
        }
        S_OK
    }
}

// ---- 打开回调：密码与分卷 ----

#[implement(IArchiveOpenCallback, ICryptoGetTextPassword, IArchiveOpenVolumeCallback)]
struct OpenCallback {
    password: Option<String>,
    asked: Arc<AtomicBool>,
    /// 当前卷的路径（分卷压缩包第一卷）。
    first: PathBuf,
}

impl IArchiveOpenCallback_Impl for OpenCallback_Impl {
    unsafe fn SetTotal(&self, _files: *const u64, _bytes: *const u64) -> HRESULT {
        S_OK
    }
    unsafe fn SetCompleted(&self, _files: *const u64, _bytes: *const u64) -> HRESULT {
        S_OK
    }
}

impl ICryptoGetTextPassword_Impl for OpenCallback_Impl {
    unsafe fn CryptoGetTextPassword(&self, password: *mut *mut u16) -> HRESULT {
        self.asked.store(true, Ordering::SeqCst);
        match &self.password {
            Some(p) => {
                unsafe { *password = alloc_bstr(p) };
                S_OK
            }
            None => E_ABORT,
        }
    }
}

impl IArchiveOpenVolumeCallback_Impl for OpenCallback_Impl {
    unsafe fn GetProperty(&self, prop: u32, value: *mut PropVariant) -> HRESULT {
        if prop == kpid::NAME {
            let name = self
                .first
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            PropVariant::set_string(value, &name);
        }
        S_OK
    }
    unsafe fn GetStream(&self, name: *const u16, stream: *mut *mut c_void) -> HRESULT {
        let mut len = 0;
        while unsafe { *name.add(len) } != 0 {
            len += 1;
        }
        let name = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name, len) });
        let path = self.first.with_file_name(&name);
        match InStream::open(&path) {
            Ok(s) => {
                unsafe { *stream = s.into_raw() };
                S_OK
            }
            // 7-Zip 约定：卷不存在时返回 S_FALSE。
            Err(_) => S_FALSE,
        }
    }
}

// ---- 打开 ----

pub struct Archive {
    pub arc: IInArchive,
    pub format: String,
    pub items: Vec<Item>,
}

/// 文件扩展名（小写），`a.tar.gz` 给 `gz`。
fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// 用 7z.dll 打开。先试扩展名对应的处理器，打不开再试其余的。
pub fn open(path: &Path, password: Option<String>) -> Result<Archive> {
    let lib = library()?;
    let ext = extension(path);
    let mut order: Vec<&Handler> = lib.handlers.iter().filter(|h| h.extensions.contains(&ext)).collect();
    for h in &lib.handlers {
        // 这几种处理器几乎什么都能"打开"，只在扩展名对上时用。
        if !order.iter().any(|o| o.name == h.name)
            && !matches!(h.name.as_str(), "Split" | "Hash" | "SHA256" | "CRC" | "Base64" | "Ihex" | "PE" | "TE" | "ELF" | "MachO" | "Mub")
        {
            order.push(h);
        }
    }
    let asked = Arc::new(AtomicBool::new(false));
    for h in order {
        let arc = lib.create(h)?;
        let stream = InStream::open(path)?;
        let cb: IArchiveOpenCallback = OpenCallback {
            password: password.clone(),
            asked: asked.clone(),
            first: path.to_path_buf(),
        }
        .into();
        let max_check = 1u64 << 20;
        let hr = unsafe { arc.Open(stream.as_raw(), &max_check, cb.as_raw()) };
        if hr == S_OK {
            let items = read_items(&arc)?;
            return Ok(Archive {
                arc,
                format: h.name.clone(),
                items,
            });
        }
        unsafe {
            let _ = arc.Close();
        }
        if asked.load(Ordering::SeqCst) {
            // 文件头加密：要过密码才打得开。
            return Err(if password.is_some() { anyhow!(BadPassword) } else { anyhow!(NeedPassword) });
        }
    }
    bail!("7z.dll 不认识这个文件，可能不是压缩包或已经损坏")
}

fn prop(arc: &IInArchive, index: u32, id: u32) -> PropVariant {
    let mut v = PropVariant::default();
    unsafe {
        let _ = arc.GetProperty(index, id, &mut v);
    }
    v
}

fn read_items(arc: &IInArchive) -> Result<Vec<Item>> {
    let mut n = 0u32;
    unsafe { arc.GetNumberOfItems(&mut n) }.ok()?;
    let mut items = Vec::with_capacity(n as usize);
    for index in 0..n {
        let raw = prop(arc, index, kpid::PATH).as_string().unwrap_or_default();
        let is_dir = prop(arc, index, kpid::IS_DIR).as_bool().unwrap_or(false);
        let alt = prop(arc, index, kpid::IS_ALT_STREAM).as_bool().unwrap_or(false);
        let aux = prop(arc, index, kpid::IS_AUX).as_bool().unwrap_or(false);
        let deleted = prop(arc, index, kpid::IS_DELETED).as_bool().unwrap_or(false);
        let path = clean_path(&raw);
        items.push(Item {
            index,
            skip: alt || aux || deleted || path.is_empty(),
            path,
            is_dir,
            size: prop(arc, index, kpid::SIZE).as_u64().unwrap_or(0),
            mtime: prop(arc, index, kpid::MTIME).as_filetime(),
            block: prop(arc, index, kpid::BLOCK).as_u64(),
        });
    }
    Ok(items)
}

/// 压缩包里的路径变成安全的相对路径：去掉盘符、`..`、`.`，替换 Windows 不允许的字符，
/// 保留设备名（CON、NUL…）前加下划线。
pub fn clean_path(raw: &str) -> String {
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." || part.ends_with(':') {
            continue;
        }
        let mut s: String = part
            .chars()
            .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || (c as u32) < 32 { '_' } else { c })
            .collect();
        while s.ends_with('.') || s.ends_with(' ') {
            s.pop();
        }
        if s.is_empty() {
            s.push('_');
        }
        let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit());
        if reserved {
            s.insert(0, '_');
        }
        parts.push(s);
    }
    parts.join("/")
}

impl Archive {
    /// 原始数据（WIM 的 SHA-1）。
    pub fn raw_prop(&self, index: u32, id: u32) -> Option<Vec<u8>> {
        let raw: IArchiveGetRawProps = self.arc.cast().ok()?;
        let mut data: *const c_void = std::ptr::null();
        let mut size = 0u32;
        let mut kind = 0u32;
        unsafe { raw.GetRawProp(index, id, &mut data, &mut size, &mut kind) }.ok().ok()?;
        (!data.is_null() && size > 0)
            .then(|| unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) }.to_vec())
    }
}

impl Drop for Archive {
    fn drop(&mut self) {
        unsafe {
            let _ = self.arc.Close();
        }
    }
}

// ---- 解出 ----

/// 解出时每个条目的去向。
pub enum Target {
    /// 写到这个文件（先写 `.onezip-part`，成功后改名）。
    File(PathBuf),
    /// 只校验，不写。
    Test,
}

/// 解出过程中给调用方的通知。
pub trait Sink: Send {
    /// 条目 `index` 写到哪里；`None` 表示跳过。
    fn target(&mut self, index: u32) -> Option<Target>;
    /// 又解出了这么多字节。
    fn progress(&mut self, bytes: u64);
    /// 条目 `index` 已经完整写好（文件已改成正式名字）。
    fn done(&mut self, index: u32, path: Option<&Path>);
}

struct State {
    sink: Box<dyn Sink>,
    current: Option<(u32, PathBuf, PathBuf)>, // (index, part, final)
    mtimes: std::collections::HashMap<u32, u64>,
    completed: u64,
    error: Option<anyhow::Error>,
    asked_password: bool,
}

#[implement(ISequentialOutStream)]
struct OutStream {
    file: Mutex<Option<BufWriter<File>>>,
}

impl ISequentialOutStream_Impl for OutStream_Impl {
    unsafe fn Write(&self, data: *const c_void, size: u32, processed: *mut u32) -> HRESULT {
        let buf = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
        let mut g = self.file.lock().unwrap();
        if let Some(f) = g.as_mut()
            && f.write_all(buf).is_err()
        {
            return E_FAIL;
        }
        if !processed.is_null() {
            unsafe { *processed = size };
        }
        S_OK
    }
}

#[implement(IArchiveExtractCallback, IProgress, ICryptoGetTextPassword)]
struct ExtractCallback {
    state: Arc<Mutex<State>>,
    out: Mutex<Option<IOutHolder>>,
    stop: Arc<AtomicBool>,
    password: Option<String>,
}

/// 当前打开的输出流（我们留一份引用，结束时取回文件刷盘）。
struct IOutHolder {
    stream: ISequentialOutStream,
}

impl ExtractCallback {
    fn finish_current(&self, ok: bool) -> std::io::Result<()> {
        let holder = self.out.lock().unwrap().take();
        let mut st = self.state.lock().unwrap();
        let Some((index, part, final_path)) = st.current.take() else {
            return Ok(());
        };
        if let Some(h) = holder {
            let imp: &OutStream = unsafe { h.stream.as_impl() };
            if let Some(mut w) = imp.file.lock().unwrap().take() {
                w.flush()?;
                let f = w.into_inner().map_err(|e| e.into_error())?;
                if ok
                    && let Some(&t) = st.mtimes.get(&index)
                {
                    let _ = f.set_modified(filetime_to_system(t));
                }
                drop(f);
            }
        }
        if ok {
            if final_path.exists() {
                std::fs::remove_file(&final_path)?;
            }
            std::fs::rename(&part, &final_path)?;
            st.sink.done(index, Some(&final_path));
        } else {
            let _ = std::fs::remove_file(&part);
        }
        Ok(())
    }
}

impl IProgress_Impl for ExtractCallback_Impl {
    unsafe fn SetTotal(&self, _total: u64) -> HRESULT {
        S_OK
    }
    unsafe fn SetCompleted(&self, complete: *const u64) -> HRESULT {
        if self.stop.load(Ordering::Relaxed) {
            return E_ABORT;
        }
        if !complete.is_null() {
            let now = unsafe { *complete };
            let mut st = self.state.lock().unwrap();
            if now > st.completed {
                let d = now - st.completed;
                st.completed = now;
                st.sink.progress(d);
            }
        }
        S_OK
    }
}

impl IArchiveExtractCallback_Impl for ExtractCallback_Impl {
    unsafe fn GetStream(&self, index: u32, out: *mut *mut c_void, ask_mode: i32) -> HRESULT {
        unsafe { *out = std::ptr::null_mut() };
        if self.stop.load(Ordering::Relaxed) {
            return E_ABORT;
        }
        if ask_mode != ask_mode::EXTRACT {
            // 测试模式：不给输出流，7-Zip 照样解码并核对校验和。
            return S_OK;
        }
        let mut st = self.state.lock().unwrap();
        match st.sink.target(index) {
            Some(Target::File(final_path)) => {
                if let Some(parent) = final_path.parent()
                    && std::fs::create_dir_all(parent).is_err()
                {
                    return E_FAIL;
                }
                let mut name = final_path.file_name().unwrap_or_default().to_os_string();
                name.push(".onezip-part");
                let part = final_path.with_file_name(name);
                let file = match File::create(&part) {
                    Ok(f) => f,
                    Err(e) => {
                        st.error = Some(anyhow!(e).context(format!("创建 {}", final_path.display())));
                        return E_FAIL;
                    }
                };
                st.current = Some((index, part, final_path));
                let stream: ISequentialOutStream = OutStream {
                    file: Mutex::new(Some(BufWriter::with_capacity(1 << 20, file))),
                }
                .into();
                *self.out.lock().unwrap() = Some(IOutHolder { stream: stream.clone() });
                unsafe { *out = stream.into_raw() };
                S_OK
            }
            Some(Target::Test) | None => S_OK,
        }
    }

    unsafe fn PrepareOperation(&self, _ask_mode: i32) -> HRESULT {
        S_OK
    }

    unsafe fn SetOperationResult(&self, result: i32) -> HRESULT {
        let ok = result == op_result::OK;
        if let Err(e) = self.finish_current(ok) {
            self.state.lock().unwrap().error = Some(anyhow!(e).context("写入解出的文件"));
            return E_FAIL;
        }
        if !ok {
            let mut st = self.state.lock().unwrap();
            if st.error.is_none() {
                st.error = Some(match result {
                    op_result::WRONG_PASSWORD => anyhow!(BadPassword),
                    op_result::DATA_ERROR | op_result::CRC_ERROR if st.asked_password => anyhow!(BadPassword),
                    op_result::UNSUPPORTED_METHOD => anyhow!("压缩包用了不支持的压缩方法"),
                    op_result::CRC_ERROR => anyhow!("数据校验失败（CRC 错误），压缩包可能已损坏"),
                    op_result::DATA_ERROR => anyhow!("数据错误，压缩包可能已损坏"),
                    op_result::UNEXPECTED_END => anyhow!("压缩包不完整（数据提前结束）"),
                    op_result::UNAVAILABLE => anyhow!("缺少分卷或数据不可用"),
                    _ => anyhow!("解压出错（代码 {result}）"),
                });
            }
        }
        S_OK
    }
}

impl ICryptoGetTextPassword_Impl for ExtractCallback_Impl {
    unsafe fn CryptoGetTextPassword(&self, password: *mut *mut u16) -> HRESULT {
        self.state.lock().unwrap().asked_password = true;
        match &self.password {
            Some(p) => {
                unsafe { *password = alloc_bstr(p) };
                S_OK
            }
            None => {
                self.state.lock().unwrap().error = Some(anyhow!(NeedPassword));
                E_ABORT
            }
        }
    }
}

fn filetime_to_system(ft: u64) -> std::time::SystemTime {
    // FILETIME：自 1601-01-01 起的 100 纳秒数。
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    if ft >= EPOCH_DIFF {
        std::time::UNIX_EPOCH + std::time::Duration::from_nanos((ft - EPOCH_DIFF) * 100)
    } else {
        std::time::UNIX_EPOCH
    }
}

impl Archive {
    /// 解出（或测试）`indices` 里的条目。`indices` 为空表示全部。
    pub fn extract(
        &self,
        indices: &[u32],
        test: bool,
        password: Option<String>,
        stop: &Arc<AtomicBool>,
        sink: Box<dyn Sink>,
    ) -> Result<()> {
        let mut sorted = indices.to_vec();
        sorted.sort_unstable();
        let mtimes = self
            .items
            .iter()
            .filter_map(|i| i.mtime.map(|t| (i.index, t)))
            .collect();
        let state = Arc::new(Mutex::new(State {
            sink,
            current: None,
            mtimes,
            completed: 0,
            error: None,
            asked_password: false,
        }));
        let cb: IArchiveExtractCallback = ExtractCallback {
            state: state.clone(),
            out: Mutex::new(None),
            stop: stop.clone(),
            password,
        }
        .into();
        let (ptr, n) = if sorted.is_empty() {
            (std::ptr::null(), u32::MAX)
        } else {
            (sorted.as_ptr(), sorted.len() as u32)
        };
        let hr = unsafe { self.arc.Extract(ptr, n, i32::from(test), cb.as_raw()) };
        // 被中断时可能还有一个写了一半的文件。
        {
            let imp: &ExtractCallback = unsafe { cb.as_impl() };
            let _ = imp.finish_current(false);
        }
        let mut st = state.lock().unwrap();
        if let Some(e) = st.error.take() {
            return Err(e);
        }
        if stop.load(Ordering::Relaxed) || hr == E_ABORT {
            return Err(anyhow!(Stopped));
        }
        hr.ok().map_err(|e| anyhow!("7z.dll 解压失败：{e}"))?;
        Ok(())
    }
}
