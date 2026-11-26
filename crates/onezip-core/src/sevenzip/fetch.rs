//! 7z.dll 格式的边下载边解压。
//!
//! 远程文件分块（最大 4 MB）、多条连接分段下载到本地一个稀疏的"镜像"文件里。解压照常走
//! 本地流程（7z.dll、自己的格式解析、ISO 直拷），只是所有读取都经过 [`Src`]：读到还没
//! 下载的块就先把它排到最前面下载，等它到了再读。下载只在当前读取位置之后的一个窗口内
//! 提前进行，所以没解到的地方不会提前占满磁盘；格式支持边解边删时，解完的区间照常打洞
//! 释放，下载过的数据随用随放。
//!
//! 断点续传：已经落盘的块记在镜像旁边的 `.onezip-chunks` 里（先刷盘再记），解压进度
//! 记在本地流程自己的进度文件里。用同一个 URL、同一个输出位置再调一次就接着来。

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use peel::http::{Client, Url};
use peel::types::{ByteOffset, ByteRange};

use crate::{Event, ExtractRequest, Observer, Outcome};

/// 块大小：大约分成 64 块，但不小于 64 KB、不大于 4 MB。
fn chunk_size(len: u64) -> u64 {
    len.div_ceil(64).next_multiple_of(64 << 10).clamp(64 << 10, 4 << 20)
}
/// 读取位置之后提前下载多少。
const WINDOW: u64 = 256 << 20;
/// 默认同时下载的连接数。
const WORKERS: usize = 8;
const MAGIC: &str = "onezip-download 1";

// ---- 读取入口 ----

/// 压缩包文件。本地文件直接读；正在下载的镜像先等要读的部分下载到。
pub(crate) struct Src {
    file: File,
    pos: u64,
    mirror: Option<Arc<Mirror>>,
}

impl Src {
    pub(crate) fn open(path: &Path) -> io::Result<Src> {
        Ok(Src {
            file: File::open(path)?,
            pos: 0,
            mirror: lookup(path),
        })
    }

    pub(crate) fn len(&self) -> io::Result<u64> {
        match &self.mirror {
            Some(m) => Ok(m.len),
            None => Ok(self.file.metadata()?.len()),
        }
    }
}

impl Read for Src {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(m) = &self.mirror {
            let want = (buf.len() as u64).min(m.len.saturating_sub(self.pos));
            if want == 0 {
                return Ok(0);
            }
            m.ensure(self.pos, want)?;
        }
        let n = self.file.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Src {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = self.file.seek(to)?;
        Ok(self.pos)
    }
}

/// 正在下载的镜像：路径 → 镜像。
type Registry = Mutex<Vec<(PathBuf, Weak<Mirror>)>>;

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}

fn lookup(path: &Path) -> Option<Arc<Mirror>> {
    let mut r = registry().lock().unwrap();
    r.retain(|(_, w)| w.strong_count() > 0);
    let key = std::path::absolute(path).ok()?;
    r.iter().find(|(p, _)| *p == key).and_then(|(_, w)| w.upgrade())
}

// ---- 镜像 ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Chunk {
    Missing,
    Loading,
    /// 已写入（可以读），还没刷盘记进检查点。
    Written,
    /// 已刷盘并记进检查点。
    Durable,
}

struct State {
    chunks: Vec<Chunk>,
    /// 读取时急着要的块。
    urgent: VecDeque<usize>,
    /// 最近一次读取所在的块：提前下载从这里往后。
    cursor: usize,
    failed: Option<String>,
    quit: bool,
}

pub(crate) struct Mirror {
    chunk: u64,
    path: PathBuf,
    ckpt: PathBuf,
    url: Url,
    len: u64,
    client: Client,
    state: Mutex<State>,
    cv: Condvar,
    stop: Arc<AtomicBool>,
    downloaded: AtomicU64,
    header: String,
}

impl Mirror {
    fn chunks(&self) -> usize {
        self.len.div_ceil(self.chunk) as usize
    }

    fn range(&self, c: usize) -> (u64, u64) {
        let s = c as u64 * self.chunk;
        (s, (s + self.chunk).min(self.len))
    }

    /// 等 `[pos, pos+len)` 下载到。
    fn ensure(&self, pos: u64, len: u64) -> io::Result<()> {
        let first = (pos / self.chunk) as usize;
        let last = ((pos + len - 1) / self.chunk) as usize;
        let mut st = self.state.lock().unwrap();
        st.cursor = first;
        for c in (first..=last).rev() {
            if st.chunks[c] == Chunk::Missing && !st.urgent.contains(&c) {
                st.urgent.push_front(c);
            }
        }
        self.cv.notify_all();
        loop {
            if (first..=last).all(|c| matches!(st.chunks[c], Chunk::Written | Chunk::Durable)) {
                return Ok(());
            }
            if let Some(e) = &st.failed {
                return Err(io::Error::other(e.clone()));
            }
            if self.stop.load(Ordering::Relaxed) || st.quit {
                // 不能用 ErrorKind::Interrupted：read_exact 遇到它会自动重试，永远停不下来。
                return Err(io::Error::other("已暂停"));
            }
            st = self.cv.wait_timeout(st, Duration::from_millis(200)).unwrap().0;
        }
    }

    /// 下一块要下载的：先下急用的，再按顺序提前下载读取位置之后的窗口。
    fn next(&self, st: &mut State) -> Option<usize> {
        while let Some(c) = st.urgent.pop_front() {
            if st.chunks[c] == Chunk::Missing {
                return Some(c);
            }
        }
        let n = self.chunks();
        let ahead = (WINDOW / self.chunk) as usize;
        (st.cursor..n.min(st.cursor + ahead)).find(|&c| st.chunks[c] == Chunk::Missing)
    }

    fn worker(self: Arc<Self>) {
        let mut out = match OpenOptions::new().write(true).open(&self.path) {
            Ok(f) => f,
            Err(e) => return self.fail(format!("打开 {} 失败：{e}", self.path.display())),
        };
        loop {
            let c = {
                let mut st = self.state.lock().unwrap();
                loop {
                    if st.quit || st.failed.is_some() || self.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(c) = self.next(&mut st) {
                        st.chunks[c] = Chunk::Loading;
                        break c;
                    }
                    st = self.cv.wait_timeout(st, Duration::from_millis(200)).unwrap().0;
                }
            };
            let mut tries = 0;
            let ok = loop {
                match self.download(c, &mut out) {
                    Ok(true) => break true,
                    // 叫停了
                    Ok(false) => break false,
                    Err(e) => {
                        tries += 1;
                        if tries >= 5 {
                            self.fail(format!("下载失败：{e:#}"));
                            break false;
                        }
                        thread::sleep(Duration::from_secs(1 << tries.min(4)));
                    }
                }
            };
            let mut st = self.state.lock().unwrap();
            st.chunks[c] = if ok { Chunk::Written } else { Chunk::Missing };
            self.cv.notify_all();
        }
    }

    fn download(&self, c: usize, out: &mut File) -> Result<bool> {
        let (s, e) = self.range(c);
        let range = ByteRange::new(ByteOffset::new(s), ByteOffset::new(e)).ok_or_else(|| anyhow!("区间无效"))?;
        let mut resp = self.client.get_range(&self.url, range).map_err(|e| anyhow!(e))?;
        out.seek(SeekFrom::Start(s))?;
        let mut buf = vec![0u8; 256 << 10];
        let mut got = 0u64;
        while got < e - s {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let want = ((e - s - got) as usize).min(buf.len());
            let n = resp.body.read(&mut buf[..want])?;
            if n == 0 {
                bail!("连接提前断开（{got} / {} 字节）", e - s);
            }
            out.write_all(&buf[..n])?;
            got += n as u64;
            self.downloaded.fetch_add(n as u64, Ordering::Relaxed);
        }
        Ok(true)
    }

    fn fail(&self, msg: String) {
        let mut st = self.state.lock().unwrap();
        st.failed.get_or_insert(msg);
        self.cv.notify_all();
    }

    /// 把已写入的块刷盘，再记进检查点。
    fn commit(&self) -> Result<()> {
        let written: Vec<usize> = {
            let st = self.state.lock().unwrap();
            (0..st.chunks.len()).filter(|&c| st.chunks[c] == Chunk::Written).collect()
        };
        if written.is_empty() {
            return Ok(());
        }
        OpenOptions::new().write(true).open(&self.path)?.sync_data()?;
        let bits = {
            let mut st = self.state.lock().unwrap();
            for c in written {
                st.chunks[c] = Chunk::Durable;
            }
            let mut bits = vec![0u8; st.chunks.len().div_ceil(8)];
            for (c, k) in st.chunks.iter().enumerate() {
                if *k == Chunk::Durable {
                    bits[c / 8] |= 1 << (c % 8);
                }
            }
            bits
        };
        let tmp = self.ckpt.with_extension("tmp");
        let mut f = File::create(&tmp)?;
        f.write_all(self.header.as_bytes())?;
        f.write_all(&bits)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, &self.ckpt)?;
        Ok(())
    }
}

/// 检查点的头：URL、总长度、块大小，以及服务器给的版本标识。
fn header(url: &str, len: u64, chunk: u64, tag: &str) -> String {
    format!("{MAGIC}\n{url}\n{len}\n{chunk}\n{tag}\n")
}

/// 问服务器文件多大、支不支持分段下载。
fn probe(client: &Client, url: &Url) -> Result<(u64, String)> {
    let range = ByteRange::new(ByteOffset::new(0), ByteOffset::new(1)).unwrap();
    let resp = client.get_range(url, range).map_err(|e| anyhow!(e)).context(
        "服务器不支持分段下载（Range），这种格式没法边下载边解压；请先下载下来再解压",
    )?;
    let total = resp
        .headers
        .get("content-range")
        .and_then(|v| peel::http::range::parse_content_range(v).ok())
        .and_then(|r| r.total())
        .ok_or_else(|| anyhow!("服务器没有告诉文件大小，没法边下载边解压"))?;
    let tag = resp
        .headers
        .get("etag")
        .or_else(|| resp.headers.get("last-modified"))
        .unwrap_or("")
        .to_string();
    Ok((total, tag))
}

/// 链接里的文件名，清理成能在磁盘上用的名字。
fn url_file_name(url: &str) -> Option<String> {
    let last = super::archive::clean_path(&crate::source::url_file_name(url)?);
    (!last.is_empty() && !last.contains('/')).then_some(last)
}

/// 边下载边解压。输出目录是 `req.output`（没给就用当前目录下以文件名命名的文件夹）。
pub(crate) fn extract_url(url: &str, req: &ExtractRequest, observer: Observer, stop: &Arc<AtomicBool>) -> Result<Outcome> {
    let name = url_file_name(url).ok_or_else(|| anyhow!("无法从 URL 得到文件名"))?;
    let output = match &req.output {
        Some(p) => std::path::absolute(p)?,
        None => std::path::absolute(super::stem(Path::new(&name)))?,
    };
    // 镜像、检查点、解压进度都放在输出旁边的一个隐藏文件夹里，完成后整个删掉。
    let mut work_name = output.file_name().unwrap_or_default().to_os_string();
    work_name.push(".onezip-download");
    let work = output.with_file_name(work_name);
    let mirror_path = work.join(&name);
    let ckpt = work.join(format!("{name}.onezip-chunks"));

    let parsed = Url::parse(url).map_err(|e| anyhow!(e)).context("URL 无效")?;
    let client = Client::new().map_err(|e| anyhow!(e)).context("创建 HTTP 客户端失败")?;
    let (len, tag) = probe(&client, &parsed)?;
    if len == 0 {
        bail!("远程文件是空的");
    }
    let chunk = chunk_size(len);
    let head = header(url, len, chunk, &tag);
    let chunks = len.div_ceil(chunk) as usize;

    // 续传：检查点的头对得上才接着用，否则从头来。
    let mut state = vec![Chunk::Missing; chunks];
    let mut resuming = false;
    if let Ok(old) = fs::read(&ckpt)
        && old.starts_with(head.as_bytes())
        && mirror_path.exists()
    {
        let bits = &old[head.len()..];
        for (c, k) in state.iter_mut().enumerate() {
            if bits.get(c / 8).is_some_and(|b| b & (1 << (c % 8)) != 0) {
                *k = Chunk::Durable;
            }
        }
        resuming = true;
    } else if work.exists() {
        if output.exists() && fs::read_dir(&output).is_ok_and(|mut d| d.next().is_some()) {
            bail!(
                "远程文件和上次没下完时不一样了。请删除 {} 和 {} 后重新开始",
                output.display(),
                work.display()
            );
        }
        fs::remove_dir_all(&work).with_context(|| format!("清理 {}", work.display()))?;
    }
    if !resuming {
        fs::create_dir_all(&work).with_context(|| format!("创建 {}", work.display()))?;
        hide(&work);
        let f = File::create(&mirror_path)?;
        #[cfg(windows)]
        super::punch::set_sparse(&f)?;
        f.set_len(len)?;
    }

    let mirror = Arc::new(Mirror {
        chunk,
        path: mirror_path.clone(),
        ckpt: ckpt.clone(),
        url: parsed,
        len,
        client,
        state: Mutex::new(State {
            chunks: state,
            urgent: VecDeque::new(),
            cursor: 0,
            failed: None,
            quit: false,
        }),
        cv: Condvar::new(),
        stop: stop.clone(),
        downloaded: AtomicU64::new(0),
        header: head,
    });
    let already = {
        let st = mirror.state.lock().unwrap();
        st.chunks.iter().enumerate().filter(|(_, k)| **k == Chunk::Durable).map(|(c, _)| mirror.range(c).1 - mirror.range(c).0).sum::<u64>()
    };
    mirror.downloaded.store(already, Ordering::Relaxed);
    registry().lock().unwrap().push((std::path::absolute(&mirror_path)?, Arc::downgrade(&mirror)));

    let n_workers = if req.connections > 0 { req.connections as usize } else { WORKERS };
    let workers: Vec<_> = (0..n_workers).map(|_| {
        let m = mirror.clone();
        thread::spawn(move || m.worker())
    }).collect();

    // 进度：下载量 / 总大小，另报解出的字节数。每秒把下好的块刷盘记进检查点。
    observer(Event::Started { total: len, resuming });
    // 智能解压：解完后文件夹里只有一项就挪出来（只动这次新建的文件夹）。
    let smart = req.placement == crate::Placement::Smart && (resuming || crate::smart::is_empty_dir(&output));
    let extracted = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let ticker = {
        let (m, obs, ex, done) = (mirror.clone(), observer.clone(), extracted.clone(), done.clone());
        thread::spawn(move || {
            let mut n = 0u32;
            while !done.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(200));
                n += 1;
                if n.is_multiple_of(5)
                    && let Err(e) = m.commit()
                {
                    m.fail(format!("保存下载进度失败：{e:#}"));
                }
                obs(Event::Remote {
                    downloaded: m.downloaded.load(Ordering::Relaxed),
                    extracted: ex.load(Ordering::Relaxed),
                    total: m.len,
                });
            }
        })
    };
    let inner: Observer = {
        let (obs, ex) = (observer.clone(), extracted.clone());
        Arc::new(move |e| match e {
            Event::Progressed(n) => {
                ex.fetch_add(n, Ordering::Relaxed);
            }
            Event::Note(s) => obs(Event::Note(s)),
            _ => {}
        })
    };

    // 能边解边删的格式就边解边删（下载的数据用完即放），否则解完再删镜像。
    let in_place = onezip_formats::in_place(&name);
    let local = ExtractRequest {
        source: crate::Source::Local(mirror_path.clone()),
        output: Some(output.clone()),
        keep_source: !in_place,
        conflict: if resuming { crate::Conflict::Replace } else { req.conflict },
        ..req.clone()
    };
    let result = super::extract(&mirror_path, &local, inner, stop, &mut |_| true).map(|(o, _)| o);

    {
        let mut st = mirror.state.lock().unwrap();
        st.quit = true;
        mirror.cv.notify_all();
    }
    for w in workers {
        let _ = w.join();
    }
    done.store(true, Ordering::Relaxed);
    let _ = ticker.join();
    let failed = mirror.state.lock().unwrap().failed.clone();
    let _ = mirror.commit();
    observer(Event::Remote {
        downloaded: mirror.downloaded.load(Ordering::Relaxed),
        extracted: extracted.load(Ordering::Relaxed),
        total: len,
    });
    drop(mirror);

    match result {
        Ok(Outcome::Finished { warning, .. }) => {
            let _ = fs::remove_dir_all(&work);
            let output = if smart { crate::smart::flatten(&output) } else { output };
            Ok(Outcome::Finished { output, warning })
        }
        Ok(o) => Ok(o),
        // 叫停时读取会中途失败，按暂停处理；进度都在，下次接着来。
        Err(_) if stop.load(Ordering::Relaxed) => Ok(Outcome::Paused),
        Err(e) => match failed {
            Some(f) => Err(anyhow!(f)),
            None => Err(e),
        },
    }
}

fn hide(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // FILE_ATTRIBUTE_HIDDEN
        unsafe {
            let _ = windows::Win32::Storage::FileSystem::SetFileAttributesW(
                windows::core::PCWSTR(w.as_ptr()),
                windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = path;
}
