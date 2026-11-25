//! 常规压缩：zip / 7z / tar.zst / tar.xz / tar.gz。
//!
//! 先写到旁边的临时文件，完成后再改名为目标名，中途取消或出错不会留下半截压缩包，
//! 也不会覆盖已有的同名文件（除非明确要求）。

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};

use crate::{Event, Observer};

/// 压缩格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    SevenZ,
    TarZst,
    TarXz,
    TarGz,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::SevenZ,
        Format::Zip,
        Format::TarZst,
        Format::TarXz,
        Format::TarGz,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZ => "7z",
            Format::TarZst => "tar.zst",
            Format::TarXz => "tar.xz",
            Format::TarGz => "tar.gz",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('.').to_ascii_lowercase();
        Format::ALL.into_iter().find(|f| f.extension() == s)
    }

    /// 是否支持密码。tar 系列本身没有加密。
    pub fn supports_password(self) -> bool {
        matches!(self, Format::Zip | Format::SevenZ)
    }
}

/// 一次压缩请求。
#[derive(Debug, Clone)]
pub struct CompressRequest {
    /// 要压缩的文件和文件夹。文件夹连同自身名字一起放进压缩包。
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub format: Format,
    /// 0（只存储）到 9（最强）。
    pub level: u32,
    pub password: Option<String>,
    /// 7z：固实压缩（压缩率更高，但取单个文件要解前面的数据）。
    pub solid: bool,
    /// 目标已存在时覆盖。
    pub overwrite: bool,
}

impl CompressRequest {
    pub fn new(inputs: Vec<PathBuf>, output: PathBuf, format: Format) -> Self {
        Self {
            inputs,
            output,
            format,
            level: 5,
            password: None,
            solid: true,
            overwrite: false,
        }
    }
}

/// 压缩包里的一项。
struct Item {
    /// 压缩包内的名字，用 `/` 分隔。
    name: String,
    /// 磁盘上的路径；`None` 不会出现（目录也有路径），留作将来的虚拟项。
    path: PathBuf,
    dir: bool,
    size: u64,
    modified: Option<SystemTime>,
}

/// 默认输出名：单个输入用它自己的名字，多个输入用它们所在文件夹的名字。
pub fn default_output(inputs: &[PathBuf], format: Format) -> Option<PathBuf> {
    let first = inputs.first()?;
    let parent = first.parent()?.to_path_buf();
    let stem = if inputs.len() == 1 {
        if first.is_dir() {
            first.file_name()?.to_string_lossy().into_owned()
        } else {
            first.file_stem()?.to_string_lossy().into_owned()
        }
    } else {
        parent
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "压缩包".into())
    };
    Some(parent.join(format!("{stem}.{}", format.extension())))
}

pub fn compress(req: &CompressRequest, observer: Observer, stop: &Arc<AtomicBool>) -> Result<()> {
    if req.inputs.is_empty() {
        bail!("没有要压缩的文件");
    }
    if req.password.is_some() && !req.format.supports_password() {
        bail!("{} 格式不支持密码，请改用 7z 或 zip", req.format.extension());
    }
    if req.output.exists() && !req.overwrite {
        bail!("{} 已存在", req.output.display());
    }
    let output = std::path::absolute(&req.output)?;
    let items = collect(&req.inputs, &output)?;
    let total: u64 = items.iter().map(|i| i.size).sum();
    observer(Event::Started {
        total,
        resuming: false,
    });

    let tmp = temp_path(&output);
    let result = (|| -> Result<()> {
        let file = File::create(&tmp).with_context(|| format!("创建 {}", tmp.display()))?;
        // 多线程 LZMA2/XZ：进度由编码器按块报告。
        let encoder_reports = threads() >= 2
            && match req.format {
                Format::SevenZ => req.level > 0,
                Format::TarXz => true,
                _ => false,
            };
        let ctx = Ctx {
            observer: &observer,
            stop,
            reader_progress: !encoder_reports,
        };
        let _hooks = encoder_reports.then(|| EncoderProgress::install(&ctx, total));
        match req.format {
            Format::Zip => write_zip(file, &items, req, &ctx)?,
            Format::SevenZ => write_7z(file, &items, req, &ctx)?,
            Format::TarZst => {
                let mut enc = zstd::stream::Encoder::new(BufWriter::new(file), zstd_level(req.level))?;
                let _ = enc.multithread(threads());
                let enc = write_tar(enc, &items, &ctx)?;
                finish(enc.finish()?)?;
            }
            Format::TarXz => {
                let mut opts = lzma_mt::XzOptions::with_preset(req.level.min(9));
                opts.set_block_size(NonZeroU64::new(64 << 20));
                let enc = lzma_mt::XzWriterMt::new(BufWriter::new(file), opts, threads())?;
                let enc = write_tar(enc, &items, &ctx)?;
                finish(enc.finish()?)?;
            }
            Format::TarGz => {
                let enc = flate2::write::GzEncoder::new(
                    BufWriter::new(file),
                    flate2::Compression::new(req.level.min(9)),
                );
                let enc = write_tar(enc, &items, &ctx)?;
                finish(enc.finish()?)?;
            }
        }
        if stop.load(Ordering::Relaxed) {
            bail!(Cancelled);
        }
        if req.overwrite && output.exists() {
            fs::remove_file(&output)?;
        }
        fs::rename(&tmp, &output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        // 被叫停时编码器返回的是各自的错误（如 encoder cancelled），统一报告成取消。
        if stop.load(Ordering::Relaxed) {
            bail!(Cancelled);
        }
    }
    result
}

/// 用户取消。
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("已取消")
    }
}

impl std::error::Error for Cancelled {}

struct Ctx<'a> {
    observer: &'a Observer,
    stop: &'a Arc<AtomicBool>,
    /// 读文件时就报告进度。多线程 LZMA2/XZ 编码器会先把几百 MB 读进内存再压，
    /// 那时改由编码器每压完一块报告一次（见 [`EncoderProgress`]）。
    reader_progress: bool,
}

impl Ctx<'_> {
    fn open(&self, item: &Item) -> io::Result<Counting> {
        Ok(Counting {
            inner: File::open(&item.path)?,
            observer: self.reader_progress.then(|| self.observer.clone()),
            stop: self.stop.clone(),
        })
    }
}

/// 在本线程上安装多线程 LZMA2/XZ 编码器的进度与取消钩子，离开作用域时撤掉。
/// （钩子是 third_party/lzma-rust2 里加的。）
struct EncoderProgress;

impl EncoderProgress {
    /// `total` 是要压缩的文件总大小。tar 流比它多出每个文件的头和对齐填充，
    /// 所以按 tar 流报告的量要封顶，免得进度超过 100%。
    fn install(ctx: &Ctx, total: u64) -> Self {
        use std::sync::atomic::AtomicU64;
        let observer = ctx.observer.clone();
        let reported = AtomicU64::new(0);
        lzma_mt::hooks::set(
            Arc::new(move |n| {
                let before = reported.fetch_add(n, Ordering::SeqCst);
                let delta = (before + n).min(total) - before.min(total);
                if delta > 0 {
                    observer(Event::Progressed(delta));
                }
            }),
            Some(ctx.stop.clone()),
        );
        EncoderProgress
    }
}

impl Drop for EncoderProgress {
    fn drop(&mut self) {
        lzma_mt::hooks::clear();
    }
}

/// 读文件时报告进度，被叫停时返回错误让编码器尽快停下。
struct Counting {
    inner: File,
    observer: Option<Observer>,
    stop: Arc<AtomicBool>,
}

impl Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(io::Error::other(Cancelled));
        }
        let n = self.inner.read(buf)?;
        if n > 0
            && let Some(observer) = &self.observer
        {
            observer(Event::Progressed(n as u64));
        }
        Ok(n)
    }
}

fn finish(mut w: BufWriter<File>) -> Result<()> {
    w.flush()?;
    let f = w.into_inner().map_err(|e| e.into_error())?;
    f.sync_all()?;
    Ok(())
}

fn temp_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    output.with_file_name(format!("{name}.onezip-tmp"))
}

fn threads() -> u32 {
    std::thread::available_parallelism().map_or(1, |n| n.get() as u32)
}

fn zstd_level(level: u32) -> i32 {
    // 0..=9 映射到 zstd 的 1..=19。
    match level {
        0 => 1,
        l => (l.min(9) as i32) * 2 + 1,
    }
}

/// 展开输入，得到按名字排序的项目列表。跳过正在写的输出文件本身。
fn collect(inputs: &[PathBuf], output: &Path) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for input in inputs {
        let input = std::path::absolute(input)?;
        let meta = fs::symlink_metadata(&input)
            .with_context(|| format!("读取 {}", input.display()))?;
        let name = input
            .file_name()
            .with_context(|| format!("{} 没有文件名", input.display()))?
            .to_string_lossy()
            .into_owned();
        walk(&input, name, &meta, output, &mut items)?;
    }
    let mut seen = std::collections::HashSet::new();
    for i in &items {
        if !seen.insert(i.name.to_lowercase()) {
            bail!("压缩包里会出现重名的 {}", i.name);
        }
    }
    Ok(items)
}

fn walk(path: &Path, name: String, meta: &fs::Metadata, output: &Path, items: &mut Vec<Item>) -> Result<()> {
    if meta.file_type().is_symlink() {
        // 不跟随链接：避免把链接指向的整个目录或系统文件意外打进去。
        return Ok(());
    }
    if meta.is_dir() {
        items.push(Item {
            name: name.clone(),
            path: path.to_path_buf(),
            dir: true,
            size: 0,
            modified: meta.modified().ok(),
        });
        let mut children: Vec<_> = fs::read_dir(path)?.collect::<io::Result<_>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let p = child.path();
            if p == output || p == temp_path(output) {
                continue;
            }
            let m = fs::symlink_metadata(&p)?;
            let child_name = format!("{name}/{}", child.file_name().to_string_lossy());
            walk(&p, child_name, &m, output, items)?;
        }
    } else if meta.is_file() {
        if path == output {
            return Ok(());
        }
        items.push(Item {
            name,
            path: path.to_path_buf(),
            dir: false,
            size: meta.len(),
            modified: meta.modified().ok(),
        });
    }
    Ok(())
}

fn write_zip(file: File, items: &[Item], req: &CompressRequest, ctx: &Ctx) -> Result<()> {
    use zip::write::SimpleFileOptions;
    use zip::{AesMode, CompressionMethod, ZipWriter};

    let mut zw = ZipWriter::new(BufWriter::new(file));
    for item in items {
        let mut opts = SimpleFileOptions::default();
        if let Some(t) = item.modified.and_then(zip_time) {
            opts = opts.last_modified_time(t);
        }
        if item.dir {
            zw.add_directory(format!("{}/", item.name), opts)?;
            continue;
        }
        opts = if req.level == 0 {
            opts.compression_method(CompressionMethod::Stored)
        } else {
            opts.compression_method(CompressionMethod::Deflated)
                .compression_level(Some(req.level.min(9) as i64))
        };
        opts = opts.large_file(item.size >= 0xFFFF_FFFF);
        if let Some(pw) = &req.password {
            opts = opts.with_aes_encryption(AesMode::Aes256, pw);
        }
        zw.start_file(item.name.as_str(), opts)?;
        io::copy(&mut ctx.open(item)?, &mut zw)
            .with_context(|| format!("压缩 {}", item.path.display()))?;
    }
    finish(zw.finish()?)
}

/// 一个固实块最多装这么多，块太大时取单个文件很慢，也占内存。
const SOLID_BLOCK_BYTES: u64 = 2 << 30;
const SOLID_BLOCK_FILES: usize = 4096;

fn write_7z(file: File, items: &[Item], req: &CompressRequest, ctx: &Ctx) -> Result<()> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password};

    let mut w = ArchiveWriter::new(BufWriter::new(file))?;
    let mut methods: Vec<EncoderConfiguration> = Vec::new();
    if let Some(pw) = &req.password {
        methods.push(AesEncoderOptions::new(Password::from(pw.as_str())).into());
        w.set_encrypt_header(true);
    }
    if req.level == 0 {
        methods.push(EncoderMethod::COPY.into());
    } else {
        methods.push(Lzma2Options::from_level_mt(req.level.min(9), threads(), 16 << 20).into());
    }
    w.set_content_methods(methods);

    let entry = |item: &Item| ArchiveEntry::from_path(&item.path, item.name.clone());

    for item in items.iter().filter(|i| i.dir) {
        w.push_archive_entry::<&[u8]>(entry(item), None)?;
    }
    let files: Vec<&Item> = items.iter().filter(|i| !i.dir).collect();
    if req.solid {
        let mut start = 0;
        while start < files.len() {
            let mut end = start;
            let mut bytes = 0;
            while end < files.len()
                && end - start < SOLID_BLOCK_FILES
                && (end == start || bytes + files[end].size <= SOLID_BLOCK_BYTES)
            {
                bytes += files[end].size;
                end += 1;
            }
            let block = &files[start..end];
            let entries = block.iter().map(|i| entry(i)).collect();
            let readers = block
                .iter()
                .map(|i| ctx.open(i).map(Into::into))
                .collect::<io::Result<Vec<_>>>()?;
            w.push_archive_entries(entries, readers)?;
            start = end;
        }
    } else {
        for item in files {
            w.push_archive_entry(entry(item), Some(ctx.open(item)?))?;
        }
    }
    finish(w.finish()?)
}

fn write_tar<W: Write>(enc: W, items: &[Item], ctx: &Ctx) -> Result<W> {
    let mut tar = tar::Builder::new(enc);
    tar.follow_symlinks(false);
    for item in items {
        let mut h = tar::Header::new_gnu();
        h.set_mtime(
            item.modified
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs()),
        );
        if item.dir {
            h.set_entry_type(tar::EntryType::Directory);
            h.set_mode(0o755);
            h.set_size(0);
            tar.append_data(&mut h, format!("{}/", item.name), io::empty())?;
        } else {
            h.set_entry_type(tar::EntryType::Regular);
            h.set_mode(0o644);
            h.set_size(item.size);
            tar.append_data(&mut h, &item.name, ctx.open(item)?)
                .with_context(|| format!("压缩 {}", item.path.display()))?;
        }
    }
    Ok(tar.into_inner()?)
}

/// 把 UTC 时间换成 zip 的日期时间（zip 格式本身不带时区）。
fn zip_time(t: SystemTime) -> Option<zip::DateTime> {
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant 的 civil_from_days。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    zip::DateTime::from_date_and_time(
        u16::try_from(y).ok()?,
        m as u8,
        d as u8,
        (rem / 3600) as u8,
        (rem % 3600 / 60) as u8,
        (rem % 60) as u8,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(Format::parse(".7Z"), Some(Format::SevenZ));
        assert_eq!(Format::parse("tar.zst"), Some(Format::TarZst));
        assert_eq!(Format::parse("rar"), None);
    }

    #[test]
    fn zip_epoch_time() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let z = zip_time(t).unwrap();
        assert_eq!((z.year(), z.month(), z.day()), (2023, 11, 14));
    }
}
