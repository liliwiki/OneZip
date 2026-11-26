//! ISO / UDF 光盘镜像的边解边删。
//!
//! 镜像里的文件不压缩、按扇区连续存放，目录在别处。所以这里自己读目录
//! （有 UDF 就用 UDF——Windows 安装镜像就是 UDF，也只有 UDF 能放下 4 GB 以上的文件；
//! 否则用 ISO9660，优先 Joliet 的长文件名），直接把文件的扇区拷出来，拷完把这些扇区
//! 在 NTFS 上打洞释放。目录、文件项等元数据所在的扇区都记下来，绝不释放；
//! 文件区间要是碰到它们，就拒绝边解边删。
//!
//! 普通（保留原文件）解压不走这里，交给 7z.dll。

use onezip_i18n::t;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};

use super::fetch::Src;
use super::{Journal, choose_output, flush_all, plan_clashes, reported_output, target_of};
use crate::{Confirm, Event, ExtractRequest, Observer, Outcome};

const SECTOR: u64 = 2048;

#[derive(Debug, Clone)]
struct IsoFile {
    path: String,
    size: u64,
    /// （字节偏移，长度，是否全零/未记录）
    extents: Vec<(u64, u64, bool)>,
    /// 内嵌在文件项里的小文件内容（UDF）。
    embedded: Option<Vec<u8>>,
    mtime: Option<SystemTime>,
}

struct Image {
    files: Vec<IsoFile>,
    dirs: Vec<String>,
    /// 元数据占用的扇区（字节区间），不能释放。
    meta: Vec<(u64, u64)>,
}

struct Reader {
    f: Src,
    len: u64,
    meta: Vec<(u64, u64)>,
}

impl Reader {
    fn sector(&mut self, n: u64) -> Result<Vec<u8>> {
        self.bytes(n * SECTOR, SECTOR as usize)
    }
    fn bytes(&mut self, pos: u64, len: usize) -> Result<Vec<u8>> {
        ensure!(pos + len as u64 <= self.len, t!("镜像被截断（读到 {} 之外）", pos + len as u64));
        let mut b = vec![0u8; len];
        self.f.seek(SeekFrom::Start(pos))?;
        self.f.read_exact(&mut b)?;
        self.meta.push((pos, pos + len as u64));
        Ok(b)
    }
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn civil(year: i64, month: u32, day: u32, h: u32, mi: u32, s: u32) -> Option<SystemTime> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
        return None;
    }
    // 公历日期 → 自 1970-01-01 起的天数
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + (h as i64) * 3600 + (mi as i64) * 60 + s as i64;
    (secs >= 0).then(|| UNIX_EPOCH + Duration::from_secs(secs as u64))
}

fn open(path: &Path) -> Result<Image> {
    let f = Src::open(path)?;
    let len = f.len()?;
    let mut r = Reader { f, len, meta: Vec::new() };
    // 卷识别序列：有 NSR02 / NSR03 就是 UDF
    let mut udf = false;
    for n in 16..32u64 {
        if (n + 1) * SECTOR > len {
            break;
        }
        let s = r.sector(n)?;
        let id = &s[1..6];
        if id == b"NSR02" || id == b"NSR03" {
            udf = true;
        }
        if id == b"TEA01" {
            break;
        }
    }
    let mut img = if udf {
        udf_open(&mut r).context(t!("读 UDF 目录失败"))?
    } else {
        iso9660_open(&mut r).context(t!("读 ISO9660 目录失败"))?
    };
    img.meta = std::mem::take(&mut r.meta);
    Ok(img)
}

// ---- ISO9660 / Joliet ----

fn iso9660_open(r: &mut Reader) -> Result<Image> {
    let mut primary = None;
    let mut joliet = None;
    for n in 16..64u64 {
        let s = r.sector(n)?;
        ensure!(&s[1..6] == b"CD001", t!("不是 ISO9660"));
        match s[0] {
            1 => primary = Some(s),
            2 if &s[88..90] == b"%/" && matches!(s[90], b'@' | b'C' | b'E') => joliet = Some(s),
            255 => break,
            _ => {}
        }
    }
    let (desc, is_joliet) = match (joliet, primary) {
        (Some(j), _) => (j, true),
        (None, Some(p)) => (p, false),
        _ => bail!(t!("没有找到卷描述符")),
    };
    let root = &desc[156..190];
    let mut img = Image { files: Vec::new(), dirs: Vec::new(), meta: Vec::new() };
    let mut seen = HashSet::new();
    walk_iso(r, u32le(root, 2) as u64, u32le(root, 10) as u64, "", is_joliet, &mut img, &mut seen, 0)?;
    Ok(img)
}

#[allow(clippy::too_many_arguments)]
fn walk_iso(
    r: &mut Reader,
    lba: u64,
    size: u64,
    prefix: &str,
    joliet: bool,
    img: &mut Image,
    seen: &mut HashSet<u64>,
    depth: usize,
) -> Result<()> {
    ensure!(depth < 64 && seen.insert(lba), t!("目录有环或嵌套太深"));
    let data = r.bytes(lba * SECTOR, size as usize)?;
    let mut p = 0usize;
    let mut pending: Option<IsoFile> = None;
    while p < data.len() {
        let len = data[p] as usize;
        if len == 0 {
            p = (p / SECTOR as usize + 1) * SECTOR as usize; // 下一扇区
            continue;
        }
        ensure!(p + len <= data.len() && len >= 34, t!("目录记录越界"));
        let rec = &data[p..p + len];
        p += len;
        let nlen = rec[32] as usize;
        let raw = &rec[33..33 + nlen];
        if nlen == 1 && (raw[0] == 0 || raw[0] == 1) {
            continue; // . 和 ..
        }
        let name = if joliet {
            let u: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
            String::from_utf16_lossy(&u)
        } else {
            raw.iter().map(|&c| c as char).collect()
        };
        let mut name = name.split(';').next().unwrap_or("").to_string();
        if !joliet && name.ends_with('.') {
            name.pop();
        }
        let name = super::archive::clean_path(&name);
        if name.is_empty() {
            continue;
        }
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let extent = u32le(rec, 2) as u64;
        let dsize = u32le(rec, 10) as u64;
        let flags = rec[25];
        let d = &rec[18..25];
        let mtime = civil(1900 + d[0] as i64, d[1] as u32, d[2] as u32, d[3] as u32, d[4] as u32, d[5] as u32);
        if flags & 0x02 != 0 {
            img.dirs.push(path.clone());
            walk_iso(r, extent, dsize, &path, joliet, img, seen, depth + 1)?;
            continue;
        }
        // 多段文件：同名的连续记录，最后一段不带 0x80
        let file = match pending.take() {
            Some(mut f) if f.path == path => {
                f.size += dsize;
                f.extents.push((extent * SECTOR, dsize, false));
                f
            }
            Some(f) => {
                img.files.push(f);
                IsoFile { path, size: dsize, extents: vec![(extent * SECTOR, dsize, false)], embedded: None, mtime }
            }
            None => IsoFile { path, size: dsize, extents: vec![(extent * SECTOR, dsize, false)], embedded: None, mtime },
        };
        if flags & 0x80 != 0 {
            pending = Some(file);
        } else {
            img.files.push(file);
        }
    }
    if let Some(f) = pending {
        img.files.push(f);
    }
    Ok(())
}

// ---- UDF ----

struct Udf {
    part_start: u64,
}

fn udf_open(r: &mut Reader) -> Result<Image> {
    let avdp = r.sector(256)?;
    ensure!(u16le(&avdp, 0) == 2, t!("没有 UDF 锚点"));
    let vds_len = u32le(&avdp, 16) as u64;
    let vds_loc = u32le(&avdp, 20) as u64;
    let mut part = None;
    let mut fsd_lbn = None;
    for i in 0..vds_len.div_ceil(SECTOR).min(64) {
        let s = r.sector(vds_loc + i)?;
        match u16le(&s, 0) {
            5 => part = Some(u32le(&s, 188) as u64),
            6 => {
                ensure!(u32le(&s, 212) as u64 == SECTOR, t!("不支持的 UDF 块大小"));
                let nmaps = u32le(&s, 268);
                let mut p = 440;
                for _ in 0..nmaps {
                    let kind = s[p];
                    let len = s[p + 1] as usize;
                    ensure!(kind == 1, t!("不支持的 UDF 分区映射（元数据分区、虚拟分区等）"));
                    p += len.max(6);
                }
                fsd_lbn = Some(u32le(&s, 252) as u64);
            }
            8 => break,
            _ => {}
        }
    }
    let u = Udf { part_start: part.context(t!("没有分区描述符"))? };
    let fsd = r.sector(u.part_start + fsd_lbn.context(t!("没有逻辑卷描述符"))?)?;
    ensure!(u16le(&fsd, 0) == 256, t!("文件集描述符不对"));
    let root_lbn = u32le(&fsd, 404) as u64;
    let mut img = Image { files: Vec::new(), dirs: Vec::new(), meta: Vec::new() };
    let mut seen = HashSet::new();
    udf_dir(r, &u, root_lbn, "", &mut img, &mut seen, 0)?;
    Ok(img)
}

/// 一个 UDF 文件项：类型、大小、修改时间、数据区间或内嵌数据。
struct Fe {
    kind: u8,
    size: u64,
    extents: Vec<(u64, u64, bool)>,
    embedded: Option<Vec<u8>>,
    mtime: Option<SystemTime>,
}

fn udf_time(b: &[u8]) -> Option<SystemTime> {
    let year = i16::from_le_bytes([b[2], b[3]]) as i64;
    civil(year, b[4] as u32, b[5] as u32, b[6] as u32, b[7] as u32, b[8] as u32)
}

fn udf_fe(r: &mut Reader, u: &Udf, lbn: u64) -> Result<Fe> {
    let s = r.sector(u.part_start + lbn)?;
    let tag = u16le(&s, 0);
    let (l_ea_at, base, mt) = match tag {
        261 => (168, 176, 84),
        266 => (208, 216, 92),
        _ => bail!(t!("文件项标记不对（{}）", tag)),
    };
    let kind = s[27];
    let ad_type = u16le(&s, 34) & 7;
    let size = u64le(&s, 56);
    let l_ea = u32le(&s, l_ea_at) as usize;
    let l_ad = u32le(&s, l_ea_at + 4) as usize;
    let start = base + l_ea;
    ensure!(start + l_ad <= s.len(), t!("文件项越界"));
    let ads = s[start..start + l_ad].to_vec();
    let mut fe = Fe { kind, size, extents: Vec::new(), embedded: None, mtime: udf_time(&s[mt..mt + 12]) };
    match ad_type {
        3 => fe.embedded = Some(ads[..(size as usize).min(ads.len())].to_vec()),
        0 | 1 => read_ads(r, u, &ads, ad_type, &mut fe.extents, 0)?,
        _ => bail!(t!("不支持的 UDF 分配描述符类型 {}", ad_type)),
    }
    Ok(fe)
}

fn read_ads(r: &mut Reader, u: &Udf, ads: &[u8], ad_type: u16, out: &mut Vec<(u64, u64, bool)>, depth: usize) -> Result<()> {
    ensure!(depth < 16, t!("分配描述符链太长"));
    let step = if ad_type == 0 { 8 } else { 16 };
    for ad in ads.chunks_exact(step) {
        let raw = u32le(ad, 0);
        let len = (raw & 0x3FFF_FFFF) as u64;
        let kind = raw >> 30;
        let lbn = u32le(ad, 4) as u64;
        if len == 0 {
            break;
        }
        if ad_type == 1 {
            ensure!(u16le(ad, 8) == 0, t!("跨分区的 UDF 文件"));
        }
        match kind {
            0 => out.push(((u.part_start + lbn) * SECTOR, len, false)),
            1 | 2 => out.push((0, len, true)), // 已分配未记录 / 未分配：读出来是零
            _ => {
                // 下一段分配描述符
                let s = r.sector(u.part_start + lbn)?;
                ensure!(u16le(&s, 0) == 258, t!("分配扩展描述符不对"));
                let l = u32le(&s, 20) as usize;
                read_ads(r, u, &s[24..24 + l.min(s.len() - 24)], ad_type, out, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn read_extents(r: &mut Reader, ext: &[(u64, u64, bool)], size: u64) -> Result<Vec<u8>> {
    let mut v = Vec::with_capacity(size as usize);
    for &(pos, len, zero) in ext {
        if zero {
            v.resize(v.len() + len as usize, 0);
        } else {
            v.extend(r.bytes(pos, len as usize)?);
        }
    }
    v.truncate(size as usize);
    Ok(v)
}

#[allow(clippy::too_many_arguments)]
fn udf_dir(
    r: &mut Reader,
    u: &Udf,
    lbn: u64,
    prefix: &str,
    img: &mut Image,
    seen: &mut HashSet<u64>,
    depth: usize,
) -> Result<()> {
    ensure!(depth < 64 && seen.insert(lbn), t!("目录有环或嵌套太深"));
    let fe = udf_fe(r, u, lbn)?;
    let data = match fe.embedded {
        Some(d) => d,
        None => read_extents(r, &fe.extents, fe.size)?,
    };
    let mut p = 0usize;
    while p + 38 <= data.len() {
        ensure!(u16le(&data, p) == 257, t!("目录项标记不对"));
        let chars = data[p + 18];
        let l_fi = data[p + 19] as usize;
        let child = u32le(&data, p + 24) as u64;
        let l_iu = u16le(&data, p + 36) as usize;
        let name_at = p + 38 + l_iu;
        ensure!(name_at + l_fi <= data.len(), t!("目录项越界"));
        let raw = &data[name_at..name_at + l_fi];
        p += (38 + l_iu + l_fi).div_ceil(4) * 4;
        if chars & 0x08 != 0 || chars & 0x04 != 0 || l_fi == 0 {
            continue; // 父目录、已删除
        }
        let name = match raw[0] {
            16 => {
                let u16s: Vec<u16> = raw[1..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                String::from_utf16_lossy(&u16s)
            }
            _ => raw[1..].iter().map(|&c| c as char).collect(),
        };
        let name = super::archive::clean_path(&name);
        if name.is_empty() {
            continue;
        }
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if chars & 0x02 != 0 {
            img.dirs.push(path.clone());
            udf_dir(r, u, child, &path, img, seen, depth + 1)?;
        } else {
            let f = udf_fe(r, u, child)?;
            if f.kind == 12 {
                continue; // 符号链接：和 7-Zip 一样不当普通文件
            }
            img.files.push(IsoFile { path, size: f.size, extents: f.extents, embedded: f.embedded, mtime: f.mtime });
        }
    }
    Ok(())
}

// ---- 单元：共用数据的文件归到一起 ----

struct IsoUnit {
    files: Vec<usize>,
    ranges: Vec<(u64, u64)>,
}

fn units(img: &Image) -> Result<Vec<IsoUnit>> {
    // 每段数据区间 → 文件；重叠的区间把它们的文件并成一组
    let mut segs: Vec<(u64, u64, usize)> = Vec::new();
    for (i, f) in img.files.iter().enumerate() {
        for &(pos, len, zero) in &f.extents {
            if !zero && len > 0 {
                segs.push((pos, pos + len.div_ceil(SECTOR) * SECTOR, i));
            }
        }
    }
    segs.sort_unstable();
    let mut parent: Vec<usize> = (0..img.files.len()).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut x = x;
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    let mut merged: Vec<(u64, u64, usize)> = Vec::new();
    for (s, e, i) in segs {
        if let Some(last) = merged.last_mut()
            && s < last.1
        {
            let (a, b) = (find(&mut parent, last.2), find(&mut parent, i));
            parent[a] = b;
            last.1 = last.1.max(e);
            continue;
        }
        merged.push((s, e, i));
    }
    // 元数据扇区绝不能在任何文件区间里
    let mut meta = img.meta.clone();
    meta.sort_unstable();
    for &(s, e, _) in &merged {
        let hit = meta.iter().any(|&(ms, me)| ms < e && s < me);
        ensure!(!hit, t!("文件数据和目录信息挤在同一处，不能边解边删"));
    }
    let mut groups: std::collections::BTreeMap<usize, IsoUnit> = Default::default();
    for (s, e, i) in merged {
        let root = find(&mut parent, i);
        groups.entry(root).or_insert_with(|| IsoUnit { files: Vec::new(), ranges: Vec::new() }).ranges.push((s, e));
    }
    for i in 0..img.files.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_insert_with(|| IsoUnit { files: Vec::new(), ranges: Vec::new() }).files.push(i);
    }
    let mut v: Vec<IsoUnit> = groups.into_values().collect();
    v.sort_by_key(|u| u.ranges.first().map_or(0, |r| r.0));
    Ok(v)
}

// ---- 解出 ----

fn copy_file(src: &mut Src, f: &IsoFile, dest: &Path, observer: &Observer, stop: &AtomicBool) -> Result<bool> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".onezip-part");
    let part = dest.with_file_name(name);
    let mut out = File::create(&part).with_context(|| t!("创建 {}", dest.display()))?;
    let mut left = f.size;
    let mut buf = vec![0u8; 1 << 20];
    if let Some(e) = &f.embedded {
        out.write_all(e)?;
        observer(Event::Progressed(e.len() as u64));
        left = 0;
    }
    for &(pos, len, zero) in &f.extents {
        let mut n = len.min(left);
        left -= n;
        if !zero {
            src.seek(SeekFrom::Start(pos))?;
        }
        while n > 0 {
            if stop.load(Ordering::Relaxed) {
                drop(out);
                let _ = fs::remove_file(&part);
                return Ok(false);
            }
            let k = (n as usize).min(buf.len());
            if zero {
                buf[..k].fill(0);
            } else {
                src.read_exact(&mut buf[..k])?;
            }
            out.write_all(&buf[..k])?;
            observer(Event::Progressed(k as u64));
            n -= k as u64;
        }
    }
    if let Some(t) = f.mtime {
        let _ = out.set_modified(t);
    }
    drop(out);
    if dest.exists() {
        fs::remove_file(dest)?;
    }
    fs::rename(&part, dest)?;
    Ok(true)
}

pub(super) fn extract_in_place(
    archive: &Path,
    req: &ExtractRequest,
    journal: Option<Journal>,
    observer: Observer,
    stop: &Arc<AtomicBool>,
    confirm: &mut dyn FnMut(&Confirm) -> bool,
) -> Result<Outcome> {
    let img = open(archive).context(t!("读不懂这个镜像的目录，不能边解边删；请用保留原文件的方式解压"))?;
    let units = units(&img)?;
    let paths: Vec<&str> = img.files.iter().map(|f| f.path.as_str()).chain(img.dirs.iter().map(String::as_str)).collect();
    let resuming = journal.is_some();
    let mut journal = match journal {
        Some(j) => j,
        None => {
            let output = choose_output(archive, req, &paths);
            let files: Vec<(u32, &str)> = img.files.iter().enumerate().map(|(n, f)| (n as u32, f.path.as_str())).collect();
            let plan = plan_clashes(&output, &files, req.conflict)?;
            if !confirm(&Confirm { archive: &archive.file_name().unwrap_or_default().to_string_lossy(), output: &output }) {
                bail!(t!("已取消，镜像没有任何改动"));
            }
            Journal::create(archive, &output, true, plan)?
        }
    };
    let output = journal.output.clone();
    fs::create_dir_all(&output)?;
    for d in &img.dirs {
        fs::create_dir_all(target_of(&output, d))?;
    }
    let todo: Vec<(u64, &IsoUnit)> =
        units.iter().enumerate().map(|(n, u)| (n as u64, u)).filter(|(n, _)| !journal.done.contains(n)).collect();
    let total: u64 = todo.iter().flat_map(|(_, u)| u.files.iter()).map(|&i| img.files[i].size).sum();
    observer(Event::Started { total, resuming });

    let mut src = Src::open(archive)?;
    // 打洞用另一个可写的句柄
    let hole = OpenOptions::new().read(true).write(true).open(archive)?;
    let mut can_punch = true;
    let mut start = 0;
    while start < todo.len() {
        let mut end = start;
        let mut bytes = 0u64;
        while end < todo.len() && end - start < super::BATCH_UNITS {
            let b: u64 = todo[end].1.files.iter().map(|&i| img.files[i].size).sum();
            if end > start && bytes + b > super::BATCH_BYTES {
                break;
            }
            bytes += b;
            end += 1;
        }
        let batch = &todo[start..end];
        start = end;
        let mut written = Vec::new();
        for (_, u) in batch {
            for &i in &u.files {
                let f = &img.files[i];
                let Some(dest) = journal.target(i as u32, &f.path) else { continue };
                if !copy_file(&mut src, f, &dest, &observer, stop)? {
                    return Ok(Outcome::Paused);
                }
                written.push(dest);
            }
        }
        flush_all(&written)?;
        journal.record(&batch.iter().map(|(n, _)| *n).collect::<Vec<_>>())?;
        #[cfg(windows)]
        if can_punch {
            for (s, e) in super::merged(batch.iter().flat_map(|(_, u)| u.ranges.iter())) {
                if !super::punch::punch(&hole, s, e)? {
                    can_punch = false;
                    observer(Event::Note(t!("这个磁盘不支持释放部分空间，镜像会在解压完成后删除").into()));
                    break;
                }
            }
            if let Some(left) = super::punch::allocated(archive) {
                observer(Event::ArchiveLeft(left));
            }
        }
        if stop.load(Ordering::Relaxed) && start < todo.len() {
            return Ok(Outcome::Paused);
        }
    }
    let _ = can_punch;
    drop(src);
    drop(hole);
    let reported = reported_output(archive, &output, &paths);
    fs::remove_file(archive).with_context(|| t!("删除 {}", archive.display()))?;
    journal.remove();
    Ok(Outcome::Finished { output: reported, warning: None })
}
