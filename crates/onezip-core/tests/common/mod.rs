#![allow(dead_code)]

pub mod http;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// 可复现的伪随机数据，半可压缩，避免 deflate 把它压成几个字节。
pub fn data(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            if i % 4 == 0 { b'a' + (i % 26) as u8 } else { x as u8 }
        })
        .collect()
}

/// 测试用的文件集合：相对路径 → 内容。
pub fn sample_files() -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    for i in 0..12u64 {
        let dir = if i % 3 == 0 { "sub/" } else { "" };
        m.insert(
            format!("{dir}file{i:02}.bin"),
            data(i, 300_000 + i as usize * 50_000),
        );
    }
    m.insert("空目录旁的中文名.txt".into(), "你好，OneZip".as_bytes().to_vec());
    m
}

/// 大量小文件。HatchZip 每批最多 512 个文件，这样至少分 3 批提交，
/// 可以在两批之间暂停。
pub fn many_files() -> BTreeMap<String, Vec<u8>> {
    (0..1300u64)
        .map(|i| (format!("d{}/f{i:04}.bin", i % 7), data(1000 + i, 2_000 + (i as usize % 5) * 700)))
        .collect()
}

/// 把目录树读回成"相对路径 → 内容"，忽略目录本身。
pub fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    walk(root, root, &mut m);
    m
}

fn walk(root: &Path, dir: &Path, m: &mut BTreeMap<String, Vec<u8>>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(root, &p, m);
        } else {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            m.insert(rel, fs::read(&p).unwrap());
        }
    }
}
