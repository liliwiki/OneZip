# Third-Party Notices

The root MIT License applies only to original OneZip source and documentation created for this repository. Third-party works, including the modified copies under `third_party/`, remain subject to their own copyright and licensing terms.

## Modified copies in this repository

### HatchZip engine (hatchzip-core)

- Project: HatchZip — https://github.com/xevrion/hatchzip
- Version: 0.1.10 (crates.io), modified
- Location: `third_party/hatchzip-core/`
- Copyright: (c) 2026 Yash Bavadiya
- License: MIT OR Apache-2.0 — `third_party/hatchzip-core/LICENSE-MIT`, `third_party/hatchzip-core/LICENSE-APACHE`
- Changes: on Windows, files are flushed to disk in parallel when a batch commits instead of one at a time. See `third_party/hatchzip-core/ONEZIP-CHANGES.md`.

OneZip enables HatchZip's `rar` feature, which compiles the UnRAR source code by Alexander Roshal into OneZip.exe and onezipc.exe. UnRAR has its own license: `third_party/hatchzip-core/LICENSE-UNRAR`, shipped as `UnRAR-License.txt`. In short, the UnRAR code may be used freely to extract RAR archives, but it may not be used to re-create the RAR compression algorithm.

### lzma-rust2 0.23

- Project: lzma-rust2 — https://github.com/hasenbanck/lzma-rust2
- Version: 0.23.0 (crates.io), modified; the version used by sevenz-rust2
- Location: `third_party/lzma-rust2/`
- License: Apache-2.0 — `third_party/lzma-rust2/LICENSE`
- Changes: the multi-threaded LZMA2 and XZ encoders report progress and honour a cancel flag. See `third_party/lzma-rust2/ONEZIP-CHANGES.md`.

## Redistributed binaries

### 7-Zip (7z.dll)

- Project: 7-Zip — https://www.7-zip.org
- Version: 26.06, x64, unmodified `7z.dll` from the official distribution
- Location: `third_party/7zip/7z.dll`, installed next to OneZip.exe
- Copyright: (C) 1999-2026 Igor Pavlov
- License: GNU LGPL 2.1 or later, with parts under the BSD 3-Clause and BSD 2-Clause licenses, and the unRAR license restriction for its RAR code — `third_party/7zip/License.txt`, shipped as `7-Zip-License.txt`
- Source code: https://www.7-zip.org/download.html. OneZip loads 7z.dll at run time; you can replace it with your own build of the same version.
- Use: listing, testing and extracting ISO, UDF, WIM/ESD, CAB, DMG, VHD/VHDX, VMDK, VDI, QCOW2, MSI, ARJ, LZH, CHM, CPIO, RPM, DEB, XAR, SquashFS, CramFS, HFS, APFS, Z and LZMA archives.

## Libraries linked into OneZip

### Peel

- Project: Peel (peel-rs) — https://github.com/agouin/peel
- Version: 0.8.0, Git revision `77d2e34b6138cc6279a5e0d1a64271b07faa0219`
- Copyright: (c) 2026 Andrew Gouin
- License: MIT OR Apache-2.0
- Its RAR5 decoder references libarchive (BSD 2-Clause); see Peel's NOTICE.

### Slint

- Project: Slint — https://slint.dev
- Version: 1.18.1
- License: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0
- OneZip uses Slint under the [Royalty-free Desktop, Mobile, and Web Applications License 2.0](https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md). The attribution it requires is the "Made with Slint" badge in the README.

### Other Rust crates

Release builds also include these crates, each under its own license:

| Crate | Version | License |
|---|---|---|
| sevenz-rust2 | 0.24.2 | Apache-2.0 |
| lzma-rust2 | 0.24.0, 0.25.0 | Apache-2.0 |
| zip | 9.8.2 | MIT |
| zstd, zstd-sys (bundles the zstd C library) | 0.13.6, 0.14.5, 2.2.0 | BSD-3-Clause (zstd-sys: BSD-3-Clause AND BSD-2-Clause) |
| flate2 | 1.1.11 | MIT OR Apache-2.0 |
| tar | 0.4.46 | MIT OR Apache-2.0 |
| windows, windows-core | 0.62.2 | MIT OR Apache-2.0 |
| rfd | 0.15.4 | MIT |
| interprocess | 2.4.9 | 0BSD OR Apache-2.0 |
| winreg | 0.56.0 | MIT |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| clap | 4.6.9 | MIT OR Apache-2.0 |
| indicatif | 0.18.6 | MIT |
| ctrlc | 3.5.3 | MIT OR Apache-2.0 |
| anyhow | 1.0.105 | MIT OR Apache-2.0 |
| crc, crc32fast | 3.4.0, 1.5.6 | MIT OR Apache-2.0 |
| md-5, sha1, sha2 | 0.11 | MIT OR Apache-2.0 |

The complete list, including indirect dependencies, is in `Cargo.lock`; `cargo metadata` prints the license of each one.

## Test files

- `crates/onezip-core/tests/fixtures/rar/`: from the HatchZip test suite (MIT OR Apache-2.0).
- `crates/onezip-core/tests/fixtures/sevenzip/lzh/`: from the libarchive test suite (BSD 2-Clause), https://github.com/libarchive/libarchive.
- Other test archives were generated for this repository (Windows IMAPI2, makecab, 7-Zip; see `crates/onezip-core/tests/fixtures/sevenzip/make.ps1` and `arj/make_arj.py`) and are covered by the root license.

## Tools used to build

- Microsoft Windows SDK (`makeappx`, `makepri`, `signtool`) packages and signs the sparse package. Nothing from the SDK is redistributed.
- winresource 0.1.34 (MIT) embeds the program icon and the installer manifest at build time.

## Names and marks

NanaZip, 7-Zip, WinRAR, RAR, HatchZip, Peel, Windows, and related names belong to their respective owners. OneZip is not affiliated with or endorsed by them.
