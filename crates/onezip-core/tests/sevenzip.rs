//! 7z.dll 处理的格式：ISO（ISO9660/Joliet、UDF）、CAB（MSZIP、LZX）、WIM、ARJ、LZH。
//!
//! 测试文件在 `tests/fixtures/sevenzip/`：ISO 用 Windows 自带的 IMAPI2 生成，CAB 用 makecab，
//! WIM 用 7-Zip（见 make.ps1）；ARJ 由 `arj/make_arj.py` 生成；LZH 来自 libarchive 的测试集
//! （BSD 2-Clause）。
//!
//! 边解边删的核对方法：同一个包，先保留原包正常解一遍（7z.dll），再边解边删解一遍
//! （ISO 用自己的目录解析，其它格式用自己算的数据区间），两边的文件必须一字不差，
//! 解完压缩包和进度文件都要删掉。

#![cfg(windows)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use onezip_core::{Event, ExtractRequest, Observer, Outcome, Source};

fn quiet() -> Observer {
    Arc::new(|_| {})
}

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sevenzip").join(rel)
}

/// 复制到临时目录（边解边删会改动它）。
fn copy(rel: &str, dir: &Path) -> PathBuf {
    let src = fixture(rel);
    let dst = dir.join(src.file_name().unwrap());
    fs::copy(&src, &dst).unwrap();
    dst
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, m: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                m.insert(format!("{rel}/"), Vec::new());
                walk(root, &p, m);
            } else {
                m.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(root, root, &mut m);
    m
}

fn extract(archive: &Path, out: &Path, keep: bool, observer: Observer, stop: &Arc<AtomicBool>) -> anyhow::Result<Outcome> {
    let req = ExtractRequest {
        source: Source::Local(archive.to_path_buf()),
        output: Some(out.to_path_buf()),
        keep_source: keep,
        keep_free: 0,
        ..Default::default()
    };
    onezip_core::extract(&req, observer, stop, &mut |_| true)
}

fn journal(archive: &Path) -> PathBuf {
    let mut n = archive.file_name().unwrap().to_os_string();
    n.push(".onezip-progress");
    archive.with_file_name(n)
}

/// 保留原包解一遍、边解边删解一遍，结果必须相同。返回解出的文件树。
fn same_both_ways(rel: &str) -> BTreeMap<String, Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    let stop = Arc::new(AtomicBool::new(false));

    let a = copy(rel, &dir.path().join("keep").tap_mkdir());
    let out_keep = dir.path().join("out-keep");
    assert!(matches!(extract(&a, &out_keep, true, quiet(), &stop).unwrap(), Outcome::Finished { .. }));
    assert!(a.exists(), "{rel}: 保留原包时不应删除");
    assert!(!journal(&a).exists(), "{rel}: 进度文件没有清理");

    let b = copy(rel, &dir.path().join("inplace").tap_mkdir());
    let out_in = dir.path().join("out-inplace");
    assert!(matches!(extract(&b, &out_in, false, quiet(), &stop).unwrap(), Outcome::Finished { .. }));
    assert!(!b.exists(), "{rel}: 边解边删完成后压缩包应已删除");
    assert!(!journal(&b).exists(), "{rel}: 进度文件没有清理");

    let (tk, ti) = (tree(&out_keep), tree(&out_in));
    let files: Vec<_> = tk.keys().filter(|k| !k.ends_with('/')).collect();
    assert!(!files.is_empty(), "{rel}: 没有解出任何文件");
    // 边解边删不解出目录以外的特殊条目，所以按文件比较
    for (k, v) in &tk {
        if !k.ends_with('/') {
            assert_eq!(ti.get(k), Some(v), "{rel}: {k} 不一样");
        }
    }
    for k in ti.keys().filter(|k| !k.ends_with('/')) {
        assert!(tk.contains_key(k), "{rel}: 边解边删多出了 {k}");
    }
    tk
}

trait Mk {
    fn tap_mkdir(self) -> Self;
}
impl Mk for PathBuf {
    fn tap_mkdir(self) -> Self {
        fs::create_dir_all(&self).unwrap();
        self
    }
}

/// make.ps1 里的样本树。
fn check_sample(t: &BTreeMap<String, Vec<u8>>, rel: &str) {
    assert_eq!(t["docs/readme.txt"], b"OneZip fixture\r\n".repeat(200), "{rel}");
    // make.ps1 用 .NET 的 UTF8 编码写这个文件，带 BOM
    let hello = [&[0xEF, 0xBB, 0xBF][..], "你好，OneZip\n".repeat(50).as_bytes()].concat();
    if rel.contains("cab") {
        // CAB 的文件名按系统代码页存（makecab 不写 UTF-8 标志），在别的语言的 Windows 上
        // 会解成别的字，所以这里按内容找
        assert!(t.values().any(|v| *v == hello), "{rel}: 没有找到 你好.txt");
    } else {
        assert_eq!(t["中文目录/你好.txt"], hello, "{rel}");
    }
    assert_eq!(t["empty.txt"].len(), 0, "{rel}");
    assert_eq!(t["sub/deep/data.bin"].len(), 100_000, "{rel}");
    assert_eq!(t["big file.bin"].len(), 400_000, "{rel}");
}

#[test]
fn iso_joliet() {
    let t = same_both_ways("iso/joliet.iso");
    check_sample(&t, "joliet");
}

#[test]
fn iso_udf() {
    let t = same_both_ways("iso/udf.iso");
    check_sample(&t, "udf");
}

#[test]
fn iso_many_files() {
    let t = same_both_ways("iso/many.iso");
    assert_eq!(t.keys().filter(|k| !k.ends_with('/')).count(), 700);
}

#[test]
fn cab_mszip_and_lzx() {
    for rel in ["cab/mszip.cab", "cab/lzx.cab"] {
        let t = same_both_ways(rel);
        check_sample(&t, rel);
    }
}

#[test]
fn cab_many_files() {
    let t = same_both_ways("cab/many.cab");
    assert_eq!(t.keys().filter(|k| !k.ends_with('/')).count(), 700);
}

#[test]
fn wim() {
    let t = same_both_ways("wim/sample.wim");
    check_sample(&t, "wim");
}

#[test]
fn arj_stored() {
    let t = same_both_ways("arj/stored.arj");
    assert_eq!(t["readme.txt"], b"OneZip ARJ fixture\r\n".repeat(100));
    assert_eq!(t["sub/data.bin"].len(), 5000);
    assert_eq!(t["last.txt"], b"the last file\n");
}

#[test]
fn lzh_header_levels_and_methods() {
    for n in ["header0", "header1", "header2", "lh0", "lh6", "lh7"] {
        let t = same_both_ways(&format!("lzh/{n}.lzh"));
        assert!(t.contains_key("file1"), "{n}: {:?}", t.keys().collect::<Vec<_>>());
    }
}

/// 边解边删中途暂停，再接着解：结果和一口气解完相同。
#[test]
fn pause_and_resume_in_place() {
    for rel in ["iso/many.iso", "cab/many.cab"] {
        let dir = tempfile::tempdir().unwrap();
        let reference = {
            let a = copy(rel, &dir.path().join("ref").tap_mkdir());
            let out = dir.path().join("out-ref");
            extract(&a, &out, true, quiet(), &Arc::new(AtomicBool::new(false))).unwrap();
            tree(&out)
        };
        let b = copy(rel, &dir.path().join("run").tap_mkdir());
        let out = dir.path().join("out-run");
        let original = fs::metadata(&b).unwrap().len();
        // 解出一点之后叫停
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicU64::new(0));
        let least = Arc::new(AtomicU64::new(u64::MAX));
        let observer: Observer = {
            let (stop, seen, least) = (stop.clone(), seen.clone(), least.clone());
            Arc::new(move |e| match e {
                Event::Progressed(n) => {
                    // 第一批（ISO 是 512 个文件）解完、释放之后再叫停
                    if seen.fetch_add(n, Ordering::SeqCst) + n > 500_000 {
                        stop.store(true, Ordering::SeqCst);
                    }
                }
                // 原压缩包实际占用的磁盘空间
                Event::ArchiveLeft(left) => {
                    least.fetch_min(left, Ordering::SeqCst);
                }
                _ => {}
            })
        };
        let first = extract(&b, &out, false, observer, &stop).unwrap();
        assert!(matches!(first, Outcome::Paused), "{rel}: 应该暂停");
        assert!(b.exists() && journal(&b).exists(), "{rel}: 暂停时压缩包和进度文件都要在");
        assert_eq!(fs::metadata(&b).unwrap().len(), original, "{rel}: 打洞不改变文件长度");
        let least = least.load(Ordering::SeqCst);
        // CAB 的几个文件夹都在同一批里，要到最后才释放；ISO 每 512 个文件释放一次
        if rel.ends_with(".iso") {
            assert!(least < original, "{rel}: 解压过程中应已释放部分空间（占用 {least}，原大小 {original}）");
        }
        let second = extract(&b, &out, false, quiet(), &Arc::new(AtomicBool::new(false))).unwrap();
        assert!(matches!(second, Outcome::Finished { .. }), "{rel}: 应该完成");
        assert!(!b.exists() && !journal(&b).exists(), "{rel}");
        let got = tree(&out);
        for (k, v) in reference.iter().filter(|(k, _)| !k.ends_with('/')) {
            assert_eq!(got.get(k), Some(v), "{rel}: {k}");
        }
    }
}

#[test]
fn list_and_test_every_fixture() {
    for rel in [
        "iso/joliet.iso",
        "iso/udf.iso",
        "cab/mszip.cab",
        "cab/lzx.cab",
        "wim/sample.wim",
        "arj/stored.arj",
        "lzh/lh7.lzh",
    ] {
        let p = fixture(rel);
        let entries = onezip_core::list(&p, None).unwrap();
        assert!(entries.iter().any(|e| !e.is_dir), "{rel}");
        onezip_core::test(&p, None, quiet()).unwrap();
    }
}

#[test]
fn which_formats_can_extract_in_place() {
    for ok in ["a.iso", "a.wim", "a.esd", "a.cab", "a.arj", "a.lzh", "a.zip", "a.7z", "a.rar", "a.tar.gz"] {
        assert!(onezip_core::can_extract_in_place(Path::new(ok)), "{ok}");
    }
    for no in ["a.dmg", "a.vhd", "a.vhdx", "a.msi", "a.chm"] {
        assert!(!onezip_core::can_extract_in_place(Path::new(no)), "{no}");
    }
}

#[test]
fn in_place_is_refused_for_formats_that_cannot() {
    let dir = tempfile::tempdir().unwrap();
    // 把一个 ISO 改名成 .vhd：格式无所谓，按扩展名就该拒绝
    let a = dir.path().join("disk.vhd");
    fs::copy(fixture("iso/joliet.iso"), &a).unwrap();
    let err = extract(&a, &dir.path().join("out"), false, quiet(), &Arc::new(AtomicBool::new(false))).unwrap_err();
    assert!(format!("{err:#}").contains("不能边解压边删除"), "{err:#}");
    assert!(a.exists());
}
