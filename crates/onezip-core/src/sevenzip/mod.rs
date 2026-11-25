//! 7z.dll 处理的格式：iso、wim/esd、cab、dmg、vhd/vhdx、msi、arj、lzh、chm 等。
//!
//! 普通解压：7z.dll 解码，OneZip 负责放置、进度、续传。
//!
//! 边解边删：只对结构上允许的格式（ISO/UDF、CAB、WIM/ESD、ARJ、LZH）。7z.dll 不告诉
//! 我们每个文件的数据在压缩包里的位置，所以由 [`layout`] 自己读这几种格式的头，
//! 把条目分成"单元"，每个单元带着它独占的字节区间。按批处理：解出一批 → 文件刷盘 →
//! 记进进度文件 → 把这批的区间在 NTFS 上打洞释放。打过洞的地方读出来是零，
//! 7z.dll 重新打开时只读文件头，不碰这些区间，所以中断后可以接着来。

mod archive;
mod fetch;
mod ffi;
mod iso;
mod layout;

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};

pub(crate) use archive::{BadPassword, NeedPassword};
pub(crate) use fetch::{extract_url, handles_url};
use archive::{Sink, Stopped, Target};

use crate::{Confirm, Entry, Event, ExtractRequest, Observer, Outcome, Placement};

/// 交给 7z.dll 的扩展名（小写，不带点）。
pub(crate) const EXTENSIONS: &[&str] = &[
    "iso", "img", "udf", "wim", "swm", "esd", "cab", "dmg", "vhd", "vhdx", "vmdk", "vdi", "qcow",
    "qcow2", "msi", "msp", "msu", "arj", "lzh", "lha", "chm", "chi", "chw", "cpio", "rpm", "deb",
    "xar", "pkg", "squashfs", "cramfs", "hfs", "hfsx", "apfs", "z", "taz", "lzma",
];

/// 能边解边删的扩展名。
pub(crate) const IN_PLACE_EXTENSIONS: &[&str] = &["iso", "udf", "wim", "esd", "cab", "arj", "lzh", "lha"];

fn ext(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

pub(crate) fn handles(path: &Path) -> bool {
    EXTENSIONS.contains(&ext(path).as_str())
}

pub(crate) fn list(path: &Path, password: Option<String>) -> Result<Vec<Entry>> {
    let arc = archive::open(path, password)?;
    Ok(arc
        .items
        .iter()
        .filter(|i| !i.skip)
        .map(|i| Entry {
            path: PathBuf::from(i.path.replace('/', "\\")),
            size: i.size,
            is_dir: i.is_dir,
        })
        .collect())
}

pub(crate) fn test(path: &Path, password: Option<String>, observer: Observer) -> Result<()> {
    let arc = archive::open(path, password.clone())?;
    let total = arc.items.iter().filter(|i| !i.skip && !i.is_dir).map(|i| i.size).sum();
    observer(Event::Started { total, resuming: false });
    struct T(Observer);
    impl Sink for T {
        fn target(&mut self, _: u32) -> Option<Target> {
            Some(Target::Test)
        }
        fn progress(&mut self, b: u64) {
            (self.0)(Event::Progressed(b));
        }
        fn done(&mut self, _: u32, _: Option<&Path>) {}
    }
    let stop = Arc::new(AtomicBool::new(false));
    arc.extract(&[], true, password, &stop, Box::new(T(observer)))
}

// ---- 进度文件 ----

/// `<压缩包>.onezip-progress`：第一行版本，第二行模式（keep / inplace），第三行输出目录，
/// 之后每行一个已完成的单元（keep 模式下是条目序号）。追加后立即刷盘。
struct Journal {
    path: PathBuf,
    output: PathBuf,
    in_place: bool,
    done: BTreeSet<u64>,
}

const JOURNAL_MAGIC: &str = "onezip-progress 1";

impl Journal {
    fn path_for(archive: &Path) -> PathBuf {
        let mut name = archive.file_name().unwrap_or_default().to_os_string();
        name.push(".onezip-progress");
        archive.with_file_name(name)
    }

    fn load(archive: &Path) -> Result<Option<Journal>> {
        let path = Self::path_for(archive);
        let Ok(f) = File::open(&path) else { return Ok(None) };
        let mut lines = BufReader::new(f).lines();
        let magic = lines.next().transpose()?.unwrap_or_default();
        if magic != JOURNAL_MAGIC {
            bail!("进度文件 {} 不是本版本写的，删掉它可以从头解压", path.display());
        }
        let mode = lines.next().transpose()?.unwrap_or_default();
        let output = PathBuf::from(lines.next().transpose()?.unwrap_or_default());
        let mut done = BTreeSet::new();
        for line in lines {
            if let Ok(n) = line?.trim().parse() {
                done.insert(n);
            }
        }
        Ok(Some(Journal {
            path,
            output,
            in_place: mode == "inplace",
            done,
        }))
    }

    fn create(archive: &Path, output: &Path, in_place: bool) -> Result<Journal> {
        let path = Self::path_for(archive);
        let mut f = File::create(&path).with_context(|| format!("创建 {}", path.display()))?;
        writeln!(f, "{JOURNAL_MAGIC}")?;
        writeln!(f, "{}", if in_place { "inplace" } else { "keep" })?;
        writeln!(f, "{}", output.display())?;
        f.sync_all()?;
        Ok(Journal {
            path,
            output: output.to_path_buf(),
            in_place,
            done: BTreeSet::new(),
        })
    }

    fn record(&mut self, ids: &[u64]) -> Result<()> {
        let mut f = OpenOptions::new().append(true).open(&self.path)?;
        let mut s = String::new();
        for id in ids {
            s.push_str(&format!("{id}\n"));
        }
        f.write_all(s.as_bytes())?;
        f.sync_all()?;
        self.done.extend(ids.iter().copied());
        Ok(())
    }

    fn remove(self) {
        let _ = fs::remove_file(&self.path);
    }
}

// ---- 放置 ----

fn top_names<'a>(paths: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    paths
        .filter_map(|p| p.split('/').next().filter(|s| !s.is_empty()))
        .map(|s| s.to_lowercase())
        .collect()
}

fn stem(archive: &Path) -> String {
    let name = archive.file_name().unwrap_or_default().to_string_lossy().into_owned();
    match name.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s.to_string(),
        _ => format!("{name}-extracted"),
    }
}

fn choose_output(archive: &Path, req: &ExtractRequest, paths: &[&str]) -> PathBuf {
    if let Some(o) = &req.output {
        return o.clone();
    }
    let here = archive.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    match req.placement {
        Placement::Here => here,
        Placement::NewFolder => here.join(stem(archive)),
        Placement::Smart if top_names(paths.iter().copied()).len() == 1 => here,
        Placement::Smart => here.join(stem(archive)),
    }
}

fn target_of(output: &Path, rel: &str) -> PathBuf {
    let mut p = output.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
    }
    p
}

/// 完成时报告的位置：直接放在压缩包旁边且只有一个顶层项时报告那一项。
fn reported_output(archive: &Path, output: &Path, paths: &[&str]) -> PathBuf {
    let tops = top_names(paths.iter().copied());
    if tops.len() == 1 && Some(output) == archive.parent()
        && let Some(first) = paths.iter().find_map(|p| p.split('/').next())
    {
        return output.join(first);
    }
    output.to_path_buf()
}

// ---- 刷盘与打洞 ----

/// 一批单元的区间排好序，首尾相接的并成一段。NTFS 打洞要按 64 KB 对齐，
/// 小文件各自只占一两个扇区，单独打洞对齐后什么都不剩；连起来才释放得掉。
/// 中间有空隙的不并，空隙里的数据可能还属于别的单元。
fn merged<'a>(ranges: impl Iterator<Item = &'a (u64, u64)>) -> Vec<(u64, u64)> {
    let mut v: Vec<(u64, u64)> = ranges.copied().filter(|(s, e)| s < e).collect();
    v.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(v.len());
    for (s, e) in v {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// 把一批写好的文件刷到磁盘（并行，见 hatchzip-core 的同类改动）。
fn flush_all(paths: &[PathBuf]) -> Result<()> {
    let chunk = paths.len().div_ceil(16).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || -> std::io::Result<()> {
                    for p in c {
                        match OpenOptions::new().write(true).open(p) {
                            Ok(f) => f.sync_all()?,
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => return Err(e),
                        }
                    }
                    Ok(())
                })
            })
            .collect();
        for h in handles {
            h.join().map_err(|_| anyhow!("刷盘线程出错"))??;
        }
        Ok(())
    })
}

#[cfg(windows)]
mod punch {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::{FILE_ZERO_DATA_INFORMATION, FSCTL_SET_SPARSE, FSCTL_SET_ZERO_DATA};

    /// 在 NTFS 上释放 `[start, end)`（按 64 KB 对齐向内收）。不支持时返回 false。
    pub fn punch(file: &File, start: u64, end: u64) -> std::io::Result<bool> {
        const ALIGN: u64 = 64 << 10;
        let s = start.next_multiple_of(ALIGN);
        let e = end / ALIGN * ALIGN;
        if s >= e {
            return Ok(true);
        }
        let h = HANDLE(file.as_raw_handle());
        let mut ret = 0u32;
        let unsupported = |e: windows::core::Error| {
            // ERROR_INVALID_FUNCTION / ERROR_NOT_SUPPORTED：FAT32、exFAT
            if matches!(e.code().0 as u32, 0x8007_0001 | 0x8007_0032) { Ok(false) } else { Err(std::io::Error::other(e)) }
        };
        if let Err(e) = unsafe { DeviceIoControl(h, FSCTL_SET_SPARSE, None, 0, None, 0, Some(&mut ret), None) } {
            return unsupported(e);
        }
        let info = FILE_ZERO_DATA_INFORMATION {
            FileOffset: s as i64,
            BeyondFinalZero: e as i64,
        };
        if let Err(e) = unsafe {
            DeviceIoControl(
                h,
                FSCTL_SET_ZERO_DATA,
                Some(&info as *const _ as *const _),
                size_of::<FILE_ZERO_DATA_INFORMATION>() as u32,
                None,
                0,
                Some(&mut ret),
                None,
            )
        } {
            return unsupported(e);
        }
        Ok(true)
    }

    /// 设成稀疏文件：没写过的地方不占磁盘（边下载边解压的镜像文件用）。
    pub fn set_sparse(file: &File) -> std::io::Result<()> {
        let mut ret = 0u32;
        // 不支持稀疏文件的磁盘（FAT32、exFAT）照样能下载，只是没写的地方也占空间。
        let _ = unsafe {
            DeviceIoControl(HANDLE(file.as_raw_handle()), FSCTL_SET_SPARSE, None, 0, None, 0, Some(&mut ret), None)
        };
        Ok(())
    }

    /// 文件实际占用的磁盘空间。
    pub fn allocated(path: &std::path::Path) -> Option<u64> {
        use windows::Win32::Storage::FileSystem::GetCompressedFileSizeW;
        let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let mut high = 0u32;
        let low = unsafe { GetCompressedFileSizeW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut high)) };
        (low != u32::MAX).then(|| (u64::from(high) << 32) | u64::from(low))
    }
}

// ---- 解出 ----

/// 往 7z.dll 解出时的去向表与进度转发。
struct Collect {
    targets: HashMap<u32, PathBuf>,
    written: Arc<Mutex<Vec<(u32, PathBuf)>>>,
    observer: Observer,
}

impl Sink for Collect {
    fn target(&mut self, index: u32) -> Option<Target> {
        self.targets.get(&index).cloned().map(Target::File)
    }
    fn progress(&mut self, b: u64) {
        (self.observer)(Event::Progressed(b));
    }
    fn done(&mut self, index: u32, path: Option<&Path>) {
        if let Some(p) = path {
            self.written.lock().unwrap().push((index, p.to_path_buf()));
        }
    }
}

/// 批的大小上限：和 HatchZip 一样，最多 512 个单元或 2 GB。
const BATCH_UNITS: usize = 512;
const BATCH_BYTES: u64 = 2 << 30;

pub(crate) fn extract(
    archive_path: &Path,
    req: &ExtractRequest,
    observer: Observer,
    stop: &Arc<AtomicBool>,
    confirm: &mut dyn FnMut(&Confirm) -> bool,
) -> Result<Outcome> {
    let journal = Journal::load(archive_path)?;
    let resuming = journal.is_some();
    let in_place = match &journal {
        Some(j) => j.in_place,
        None => !req.keep_source,
    };
    if in_place && !IN_PLACE_EXTENSIONS.contains(&ext(archive_path).as_str()) {
        bail!("这种格式不能边解压边删除；请用保留原压缩包的方式解压");
    }
    // ISO/UDF 边解边删：数据不压缩，自己读目录、自己拷数据，不经过 7z.dll。
    if in_place && matches!(ext(archive_path).as_str(), "iso" | "udf") {
        return iso::extract_in_place(archive_path, req, journal, observer, stop, confirm);
    }

    let arc = archive::open(archive_path, req.password.clone())?;
    // 拷一份条目信息：解完要先关掉 7z.dll 的文件句柄，才能删除压缩包。
    let items: Vec<archive::Item> = arc.items.iter().filter(|i| !i.skip).cloned().collect();
    let paths: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();

    // 单元：边解边删时来自对格式头的解析，否则每个条目自成一个单元。
    let units: Vec<layout::Unit> = if in_place {
        layout::units(archive_path, &arc).context("读不懂这个压缩包的结构，不能边解边删；请用保留原压缩包的方式解压")?
    } else {
        items.iter().map(|i| layout::Unit { items: vec![i.index], ranges: Vec::new() }).collect()
    };

    let mut journal = match journal {
        Some(j) => j,
        None => {
            let output = choose_output(archive_path, req, &paths);
            if !req.replace {
                let clash: Vec<_> = items
                    .iter()
                    .filter(|i| !i.is_dir)
                    .map(|i| target_of(&output, &i.path))
                    .filter(|p| p.exists())
                    .collect();
                if let Some(first) = clash.first() {
                    bail!(
                        "{} 里已有 {} 个同名文件（例如 {}）。什么都没改动；如要替换请选择替换",
                        output.display(),
                        clash.len(),
                        first.display()
                    );
                }
            }
            if in_place
                && !confirm(&Confirm {
                    archive: &archive_path.file_name().unwrap_or_default().to_string_lossy(),
                    output: &output,
                })
            {
                bail!("已取消，压缩包没有任何改动");
            }
            Journal::create(archive_path, &output, in_place)?
        }
    };
    let output = journal.output.clone();
    fs::create_dir_all(&output).with_context(|| format!("创建 {}", output.display()))?;
    for i in items.iter().filter(|i| i.is_dir) {
        fs::create_dir_all(target_of(&output, &i.path))?;
    }

    let by_index: HashMap<u32, &archive::Item> = items.iter().map(|i| (i.index, i)).collect();
    let mut todo: Vec<(u64, &layout::Unit)> = units
        .iter()
        .enumerate()
        .map(|(n, u)| (n as u64, u))
        .filter(|(n, _)| !journal.done.contains(n))
        .collect();
    // 按数据在文件里的先后处理：边下载边解压时读取是顺序的，下载可以一直走在前面。
    todo.sort_by_key(|(_, u)| u.ranges.first().map_or(0, |r| r.0));
    let unit_bytes = |u: &layout::Unit| -> u64 {
        u.items.iter().filter_map(|i| by_index.get(i)).filter(|i| !i.is_dir).map(|i| i.size).sum()
    };
    let total: u64 = todo.iter().map(|(_, u)| unit_bytes(u)).sum();
    observer(Event::Started { total, resuming });

    let file = if in_place {
        Some(OpenOptions::new().read(true).write(true).open(archive_path)?)
    } else {
        None
    };
    let mut can_punch = true;

    let mut start = 0;
    while start < todo.len() {
        // 组一批
        let mut end = start;
        let mut bytes = 0;
        while end < todo.len()
            && end - start < if in_place { BATCH_UNITS } else { 4096 }
            && (end == start || bytes + unit_bytes(todo[end].1) <= BATCH_BYTES)
        {
            bytes += unit_bytes(todo[end].1);
            end += 1;
        }
        let batch = &todo[start..end];
        start = end;

        let mut targets = HashMap::new();
        for (_, u) in batch {
            for idx in &u.items {
                if let Some(it) = by_index.get(idx)
                    && !it.is_dir
                {
                    targets.insert(*idx, target_of(&output, &it.path));
                }
            }
        }
        let indices: Vec<u32> = targets.keys().copied().collect();
        let written = Arc::new(Mutex::new(Vec::new()));
        if !indices.is_empty() {
            let sink = Collect {
                targets,
                written: written.clone(),
                observer: observer.clone(),
            };
            match arc.extract(&indices, false, req.password.clone(), stop, Box::new(sink)) {
                Ok(()) => {}
                Err(e) if e.is::<Stopped>() => {
                    // 已经写好的文件留着；这一批没记进度，下次整批重来（覆盖它们）。
                    return Ok(Outcome::Paused);
                }
                Err(e) => return Err(e),
            }
        }
        let written: Vec<PathBuf> = written.lock().unwrap().iter().map(|(_, p)| p.clone()).collect();
        if in_place {
            flush_all(&written)?;
        }
        let ids: Vec<u64> = batch.iter().map(|(n, _)| *n).collect();
        journal.record(&ids)?;

        #[cfg(windows)]
        if let Some(f) = &file
            && can_punch
        {
            for (s, e) in merged(batch.iter().flat_map(|(_, u)| u.ranges.iter())) {
                if !punch::punch(f, s, e)? {
                    can_punch = false;
                    observer(Event::Note("这个磁盘不支持释放部分空间，原压缩包会在解压完成后删除".into()));
                    break;
                }
            }
            if let Some(left) = punch::allocated(archive_path) {
                observer(Event::ArchiveLeft(left));
            }
        }
        if stop.load(Ordering::Relaxed) && start < todo.len() {
            return Ok(Outcome::Paused);
        }
    }
    let _ = can_punch;

    drop(file);
    drop(arc);
    let reported = reported_output(archive_path, &output, &paths);
    if in_place {
        fs::remove_file(archive_path).with_context(|| format!("删除 {}", archive_path.display()))?;
    }
    journal.remove();
    Ok(Outcome::Finished {
        output: reported,
        warning: None,
    })
}
