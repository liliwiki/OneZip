//! 边解边删要知道每个文件的数据在压缩包里的位置，7z.dll 不提供，这里自己读格式头。
//!
//! 结果是一组"单元"：单元里的条目一起解出，解完后这个单元的字节区间就没人再用，
//! 可以释放。区间只覆盖数据，从不包括任何文件头、目录或索引，所以释放之后
//! 7z.dll 照样能重新打开压缩包（续传时要用）。
//!
//! 每种格式解析完都和 7z.dll 列出的条目逐一核对（路径、大小），对不上就拒绝边解边删，
//! 宁可不释放，也不能释放错。

use onezip_i18n::t;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use super::archive::{Archive, Item, clean_path};
use super::fetch::Src;
use super::ffi::kpid;

/// 一起解出、一起释放的一组条目。
#[derive(Debug, Clone)]
pub struct Unit {
    /// 7z.dll 的条目序号。
    pub items: Vec<u32>,
    /// 解完后可以释放的区间 `[起, 止)`。
    pub ranges: Vec<(u64, u64)>,
}

pub fn units(path: &Path, arc: &Archive) -> Result<Vec<Unit>> {
    let mut f = Src::open(path)?;
    let len = f.len()?;
    let units = match arc.format.as_str() {
        "Cab" => cab(&mut f, arc)?,
        "wim" => wim(&mut f, arc)?,
        "Arj" => sequential(arc, arj(&mut f, len)?)?,
        "Lzh" => sequential(arc, lzh(&mut f, len)?)?,
        other => bail!(t!("{} 格式不支持边解边删", other)),
    };
    check(&units, len)?;
    Ok(units)
}

/// 区间必须在文件内、互不重叠；每个要解出的条目恰好属于一个单元。
fn check(units: &[Unit], len: u64) -> Result<()> {
    let mut all: Vec<(u64, u64)> = units.iter().flat_map(|u| u.ranges.iter().copied()).collect();
    all.sort_unstable();
    for w in all.windows(2) {
        ensure!(w[0].1 <= w[1].0, t!("数据区间重叠"));
    }
    for &(s, e) in &all {
        ensure!(s <= e && e <= len, t!("数据区间超出文件"));
    }
    let mut seen = std::collections::HashSet::new();
    for u in units {
        for i in &u.items {
            ensure!(seen.insert(*i), t!("条目 {} 出现在两个单元里", i));
        }
    }
    Ok(())
}

fn read_at(f: &mut Src, pos: u64, buf: &mut [u8]) -> Result<()> {
    f.seek(SeekFrom::Start(pos))?;
    f.read_exact(buf)?;
    Ok(())
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

/// 要解出的条目（不含目录和跳过的）。
fn files(arc: &Archive) -> Vec<&Item> {
    arc.items.iter().filter(|i| !i.skip && !i.is_dir).collect()
}

/// 目录和没有数据的条目放进一个没有区间的单元，最先处理。
fn leftovers(arc: &Archive, used: &std::collections::HashSet<u32>) -> Option<Unit> {
    let rest: Vec<u32> = arc
        .items
        .iter()
        .filter(|i| !i.skip && !used.contains(&i.index))
        .map(|i| i.index)
        .collect();
    (!rest.is_empty()).then_some(Unit { items: rest, ranges: Vec::new() })
}

// ---- CAB ----

/// CAB：文件按"文件夹"分组压缩，同一文件夹的数据块（CFDATA）连续存放、前后依赖。
/// 一个文件夹是一个单元，区间是它全部 CFDATA 块。只支持单个 .cab（不跨卷）。
fn cab(f: &mut Src, arc: &Archive) -> Result<Vec<Unit>> {
    let mut h = [0u8; 36];
    read_at(f, 0, &mut h)?;
    ensure!(&h[0..4] == b"MSCF", t!("不是 CAB"));
    let coff_files = u32le(&h, 16) as u64;
    let folders = u16le(&h, 26) as usize;
    let nfiles = u16le(&h, 28) as usize;
    let flags = u16le(&h, 30);
    ensure!(flags & 0x3 == 0, t!("分卷的 CAB 不能边解边删"));
    let mut pos = 36u64;
    let (mut res_folder, mut res_data) = (0u64, 0u64);
    if flags & 0x4 != 0 {
        let mut r = [0u8; 4];
        read_at(f, 36, &mut r)?;
        let res_header = u16le(&r, 0) as u64;
        res_folder = r[2] as u64;
        res_data = r[3] as u64;
        pos = 40 + res_header;
    }
    // 文件夹表
    let mut folder_ranges = Vec::with_capacity(folders);
    for i in 0..folders {
        let mut e = [0u8; 8];
        read_at(f, pos + i as u64 * (8 + res_folder), &mut e)?;
        let start = u32le(&e, 0) as u64;
        let blocks = u16le(&e, 4);
        // 走一遍这个文件夹的 CFDATA 块，算出结束位置。
        let mut p = start;
        for _ in 0..blocks {
            let mut d = [0u8; 8];
            read_at(f, p, &mut d)?;
            p += 8 + res_data + u16le(&d, 4) as u64;
        }
        folder_ranges.push((start, p));
    }
    // 文件表：每个文件属于哪个文件夹、在文件夹解压后数据里的位置。文件名不用来对应
    // （CAB 的文件名是本地代码页编码，各系统解读不同），而是和 7z.dll 给的
    // 文件夹号（kpidBlock）、偏移（kpidOffset）、大小三者对应。
    let mut by_folder: Vec<Vec<(u64, u64)>> = vec![Vec::new(); folders];
    let mut p = coff_files;
    for _ in 0..nfiles {
        let mut e = [0u8; 16];
        read_at(f, p, &mut e)?;
        let size = u32le(&e, 0) as u64;
        let offset = u32le(&e, 4) as u64;
        let folder = u16le(&e, 8) as usize;
        ensure!(folder < folders, t!("文件跨卷或属于不存在的文件夹"));
        // 跳过以 0 结尾的文件名
        let mut q = p + 16;
        loop {
            let mut b = [0u8; 1];
            read_at(f, q, &mut b)?;
            q += 1;
            if b[0] == 0 {
                break;
            }
        }
        p = q;
        by_folder[folder].push((offset, size));
    }
    // 7z.dll 把 CAB 的条目按（文件夹，偏移，大小）排序（CabIn.cpp 的 CompareMvItems），
    // 只公开文件夹号。所以每个文件夹里两边都按（偏移，大小）排好，逐个对应、核对大小。
    let mut by_block: HashMap<u64, VecDeque<&Item>> = HashMap::new();
    for it in files(arc) {
        let block = it.block.with_context(|| t!("7z.dll 没有给出 {} 所在的文件夹", it.path))?;
        by_block.entry(block).or_default().push_back(it);
    }
    let mut used = std::collections::HashSet::new();
    let mut units = Vec::new();
    for (i, mut entries) in by_folder.into_iter().enumerate() {
        entries.sort_unstable();
        let mut theirs = by_block.remove(&(i as u64)).unwrap_or_default();
        ensure!(
            theirs.len() == entries.len(),
            t!("CAB 文件夹 {} 有 {} 个文件，7z.dll 列出 {} 个", i, entries.len(), theirs.len()));
        let mut items = Vec::new();
        for (offset, size) in entries {
            let it = theirs.pop_front().unwrap();
            ensure!(it.size == size, t!("CAB 文件夹 {} 偏移 {} 的文件大小和 7z.dll 列出的不一样", i, offset));
            used.insert(it.index);
            items.push(it.index);
        }
        units.push(Unit { items, ranges: vec![folder_ranges[i]] });
    }
    ensure!(by_block.is_empty(), t!("7z.dll 列出了 CAB 文件表里没有的文件夹"));
    let mut all: Vec<Unit> = leftovers(arc, &used).into_iter().collect();
    all.extend(units);
    Ok(all)
}

// ---- WIM / ESD ----

const SOLID_MAGIC_SIZE: u64 = 0x1_0000_0000;

/// WIM：文件内容按 SHA-1 去重存成"资源"，查找表（lookup table）记着每个资源的位置。
/// 一个资源是一个单元，用到它的所有条目一起解；固实资源（ESD 常见）里的所有流一起算一个单元。
/// 元数据资源（目录树）、查找表、XML 从不释放。只支持单个文件（不支持 .swm 分卷）。
fn wim(f: &mut Src, arc: &Archive) -> Result<Vec<Unit>> {
    let mut h = [0u8; 208];
    read_at(f, 0, &mut h)?;
    ensure!(&h[0..8] == b"MSWIM\0\0\0", t!("不是 WIM"));
    let total_parts = u16le(&h, 42);
    ensure!(total_parts <= 1, t!("分卷的 WIM（.swm）不能边解边删"));
    let table_size = u64le(&h, 48) & 0x00FF_FFFF_FFFF_FFFF;
    let table_flags = h[55];
    let table_offset = u64le(&h, 56);
    ensure!(table_flags & 0x04 == 0, t!("压缩过的查找表暂不支持边解边删"));
    let mut table = vec![0u8; table_size as usize];
    read_at(f, table_offset, &mut table)?;

    // 资源编号 → 区间；SHA-1 → 资源编号
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut by_hash: HashMap<[u8; 20], usize> = HashMap::new();
    let mut current_solid: Option<usize> = None;
    for e in table.as_chunks::<50>().0 {
        let size = u64le(e, 0) & 0x00FF_FFFF_FFFF_FFFF;
        let flags = e[7];
        let offset = u64le(e, 8);
        let original = u64le(e, 16);
        let hash: [u8; 20] = e[30..50].try_into().unwrap();
        let metadata = flags & 0x02 != 0;
        let solid = flags & 0x10 != 0;
        if solid && original == SOLID_MAGIC_SIZE {
            // 固实资源本身：后面的流都在它里面
            ranges.push((offset, offset + size));
            current_solid = Some(ranges.len() - 1);
            continue;
        }
        if metadata {
            continue;
        }
        let r = if solid {
            current_solid.context(t!("固实流出现在固实资源之前"))?
        } else {
            current_solid = None;
            ranges.push((offset, offset + size));
            ranges.len() - 1
        };
        by_hash.insert(hash, r);
    }

    let mut groups: HashMap<usize, Vec<u32>> = HashMap::new();
    let mut used = std::collections::HashSet::new();
    for it in files(arc) {
        if it.size == 0 {
            continue;
        }
        let sha = arc.raw_prop(it.index, kpid::SHA1).context(t!("7z.dll 没有给出 SHA-1"))?;
        let sha: [u8; 20] = sha.as_slice().try_into().context(t!("SHA-1 长度不对"))?;
        let r = *by_hash.get(&sha).with_context(|| t!("{} 的数据不在查找表里", it.path))?;
        groups.entry(r).or_default().push(it.index);
        used.insert(it.index);
    }
    // 备用数据流也用资源，没人解出它们，但同一资源可能还被别的条目用：
    // 只要一个资源有任何条目在用，就随这些条目一起释放；没人用的资源不碰。
    let mut units: Vec<Unit> = leftovers(arc, &used).into_iter().collect();
    let mut keys: Vec<usize> = groups.keys().copied().collect();
    keys.sort_unstable_by_key(|&r| ranges[r].0);
    for r in keys {
        units.push(Unit { items: groups.remove(&r).unwrap(), ranges: vec![ranges[r]] });
    }
    Ok(units)
}

// ---- ARJ / LZH：依次存放的格式 ----

/// 依次存放的格式里一个条目：路径、原始大小、数据区间。
struct Seq {
    path: String,
    size: u64,
    is_dir: bool,
    range: (u64, u64),
}

/// 按顺序和 7z.dll 的条目一一对应（两边都是按存放顺序列的），逐个核对。
fn sequential(arc: &Archive, entries: Vec<Seq>) -> Result<Vec<Unit>> {
    let items: Vec<&Item> = arc.items.iter().collect();
    ensure!(items.len() == entries.len(), t!("条目数和 7z.dll 列出的不一样（{} 对 {}）", entries.len(), items.len()));
    let mut units = Vec::new();
    let mut dirs = Vec::new();
    for (it, e) in items.iter().zip(&entries) {
        // 两边都按存放顺序列出，按位置对应；名字的编码各系统解读不同，不拿来比，
        // 只核对大小和是不是目录。
        ensure!(
            it.size == e.size && it.is_dir == e.is_dir,
            t!("第 {} 个条目（{}）和 7z.dll 列出的 {} 对不上", it.index, e.path, it.path));
        if it.skip {
            continue;
        }
        if e.is_dir || e.range.0 == e.range.1 {
            dirs.push(it.index);
        } else {
            units.push(Unit { items: vec![it.index], ranges: vec![e.range] });
        }
    }
    let mut all = Vec::new();
    if !dirs.is_empty() {
        all.push(Unit { items: dirs, ranges: Vec::new() });
    }
    all.extend(units);
    Ok(all)
}

/// ARJ：主头之后，每个文件一个本地头，紧跟着压缩数据。
fn arj(f: &mut Src, len: u64) -> Result<Vec<Seq>> {
    let mut pos = 0u64;
    let mut out = Vec::new();
    let mut first = true;
    loop {
        ensure!(pos + 4 <= len, t!("ARJ 头被截断"));
        let mut h = [0u8; 4];
        read_at(f, pos, &mut h)?;
        ensure!(h[0] == 0x60 && h[1] == 0xEA, t!("ARJ 头标记不对"));
        let size = u16le(&h, 2) as u64;
        if size == 0 {
            break; // 结束
        }
        let mut b = vec![0u8; size as usize];
        read_at(f, pos + 4, &mut b)?;
        // 头之后：4 字节 CRC，然后若干扩展头（每个 2 字节长度 + 内容 + 4 字节 CRC），长度为 0 结束
        let mut p = pos + 4 + size + 4;
        loop {
            let mut x = [0u8; 2];
            read_at(f, p, &mut x)?;
            let n = u16le(&x, 0) as u64;
            p += 2;
            if n == 0 {
                break;
            }
            p += n + 4;
        }
        if first {
            first = false;
            ensure!(b[4] & 0x04 == 0, t!("分卷的 ARJ 不能边解边删"));
            pos = p;
            continue;
        }
        let first_size = b[0] as usize;
        let flags = b[4];
        let file_type = b[6];
        let packed = u32le(&b, 12) as u64;
        let original = u32le(&b, 16) as u64;
        ensure!(flags & 0x04 == 0 && flags & 0x08 == 0, t!("分卷的 ARJ 不能边解边删"));
        let name_end = b[first_size..].iter().position(|&c| c == 0).context(t!("ARJ 文件名没有结尾"))?;
        let raw: String = b[first_size..first_size + name_end].iter().map(|&c| c as char).collect();
        let is_dir = file_type == 3;
        out.push(Seq {
            path: clean_path(&raw),
            size: if is_dir { 0 } else { original },
            is_dir,
            range: (p, p + packed),
        });
        pos = p + packed;
    }
    Ok(out)
}

/// LZH：每个文件一个头（0、1、2 级），紧跟着压缩数据。
fn lzh(f: &mut Src, len: u64) -> Result<Vec<Seq>> {
    let mut pos = 0u64;
    let mut out = Vec::new();
    while pos < len {
        let mut b = [0u8; 22];
        let n = (len - pos).min(22) as usize;
        read_at(f, pos, &mut b[..n])?;
        if b[0] == 0 {
            break; // 结束标记
        }
        ensure!(n == 22 && &b[2..4] == b"-l" && b[6] == b'-', t!("LZH 头不对"));
        let level = b[20];
        let packed = u32le(&b, 7) as u64;
        let original = u32le(&b, 11) as u64;
        let method = [b[2], b[3], b[4], b[5], b[6]];
        let is_dir = &method == b"-lhd-";
        let (data_start, data_len, name, dir_from_ext);
        match level {
            0 | 1 => {
                let hsize = b[0] as u64;
                let mut hb = vec![0u8; hsize as usize + 2];
                read_at(f, pos, &mut hb)?;
                let nlen = hb[21] as usize;
                ensure!(22 + nlen <= hb.len(), t!("LZH 文件名超出头"));
                name = hb[22..22 + nlen].iter().map(|&c| c as char).collect::<String>();
                let mut p = pos + hsize + 2;
                let mut ext_total = 0;
                let mut dir = String::new();
                if level == 1 {
                    // 扩展头链：头的最后 2 字节是第一个扩展头的长度
                    let mut next = u16le(&hb, hsize as usize) as u64;
                    while next != 0 {
                        let mut eh = vec![0u8; next as usize];
                        read_at(f, p, &mut eh)?;
                        if eh[0] == 0x02 {
                            dir = eh[1..eh.len() - 2].iter().map(|&c| if c == 0xFF { '/' } else { c as char }).collect();
                        }
                        p += next;
                        ext_total += next;
                        next = u16le(&eh, eh.len() - 2) as u64;
                    }
                }
                dir_from_ext = dir;
                data_start = p;
                data_len = packed - ext_total;
            }
            2 => {
                let total = u16le(&b, 0) as u64;
                let mut hb = vec![0u8; total as usize];
                read_at(f, pos, &mut hb)?;
                // 扩展头从 24 开始：2 字节长度（含自身）+ 类型 + 内容
                let mut p = 24usize;
                let mut fname = String::new();
                let mut dir = String::new();
                loop {
                    ensure!(p + 2 <= hb.len(), t!("LZH 扩展头越界"));
                    let n = u16le(&hb, p) as usize;
                    if n == 0 {
                        break;
                    }
                    ensure!(p + n <= hb.len(), t!("LZH 扩展头越界"));
                    let kind = hb[p + 2];
                    let body = &hb[p + 3..p + n];
                    match kind {
                        0x01 => fname = body.iter().map(|&c| c as char).collect(),
                        0x02 => dir = body.iter().map(|&c| if c == 0xFF { '/' } else { c as char }).collect(),
                        _ => {}
                    }
                    p += n;
                }
                name = fname;
                dir_from_ext = dir;
                data_start = pos + total;
                data_len = packed;
            }
            _ => bail!(t!("不支持的 LZH 头级别 {}", level)),
        }
        let full = if dir_from_ext.is_empty() {
            name
        } else if is_dir {
            dir_from_ext
        } else {
            format!("{}/{}", dir_from_ext.trim_end_matches('/'), name)
        };
        out.push(Seq {
            path: clean_path(&full.replace('\\', "/")),
            size: if is_dir { 0 } else { original },
            is_dir,
            range: (data_start, data_start + data_len),
        });
        pos = data_start + data_len;
    }
    Ok(out)
}
