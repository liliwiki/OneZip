# OneZip

[![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4)](https://github.com/liliwiki/OneZip)
[![Rust](https://img.shields.io/badge/Rust-1.93+-B7410E)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

**English** | [简体中文](README.zh-CN.md)

An all-in-one archive tool for Windows: compress, extract smartly, extract in place while freeing the archive's space, and extract while downloading. It builds on the [HatchZip](https://github.com/xevrion/hatchzip) and [Peel](https://github.com/agouin/peel) engines and adds its menu to the first level of the Windows 11 context menu. The interface is in Simplified Chinese.

> OneZip is an independent project. It is not affiliated with NanaZip, 7-Zip, HatchZip, or Peel.

## Download

Get `OneZip-<version>-x64.zip` from the [Releases](https://github.com/liliwiki/OneZip/releases) page, extract it, and run in that folder:

```powershell
powershell -ExecutionPolicy Bypass -File install.ps1
```

It installs to `%LOCALAPPDATA%\Programs\OneZip` for the current user. On Windows 11 the context menu is registered through a sparse package; release builds are signed with a self-signed certificate, so the installer asks once for administrator approval to add it to "Trusted People". On Windows 10 the classic per-user context menu is used. To uninstall, use "Installed apps" in Windows Settings or run `uninstall.ps1`.

Requires Windows 10 or 11 (x64). Verify the SHA-256 published on the Release page before running it.

## Features

| Feature | Details | Engine |
|---|---|---|
| Compress | 7z (LZMA2, multi-threaded, solid, AES-256 with encrypted file names), zip (Deflate, AES-256), tar.zst, tar.xz, tar.gz. Written to a temporary file and renamed only when complete | sevenz-rust2, zip, zstd, lzma-rust2, flate2 |
| Extract | zip, 7z, tar, tar.gz, tar.bz2, tar.xz, split archives (.001, .z01) | HatchZip |
| Extract | tar.zst, tar.lz4, .zst, .lz4, rar (RAR5 and RAR4, unencrypted) | Peel |
| Extract in place | Frees the archive's space batch by batch while extracting; resumes after a power cut or a crash | HatchZip, Peel |
| Extract while downloading | Parallel ranged download, streaming decode, used data released at once, resumable | Peel |
| Smart extraction | A single top-level item goes next to the archive, otherwise into a folder named after it; nested archives are unpacked too | OneZip |
| List, test, checksums | CRC-32, CRC-64, MD5, SHA-1, SHA-256, SHA-512 for files and folders | OneZip, HatchZip |

## Context menu

Right-clicking an archive opens the OneZip submenu:

- Smart extraction / Extract to "name\" / Extract here / Extract files…
- Extract and delete the archive (frees space while extracting)
- Test archive
- Add to archive… / Add to "name.7z" / Add to "name.zip"
- CRC-32 / MD5 / SHA-256 / all algorithms

Other files and folders show only the compression and checksum items. Each operation opens its own progress window with elapsed time, remaining time, speed, pause and cancel; it closes when the work is done. The main window opens only when you start OneZip directly.

## Safety

"Extract and delete the archive" removes the archive while it extracts and cannot be undone. Progress is saved on disk, so an interrupted extraction continues from where it stopped, but keep a backup of anything you cannot replace. Extracting while downloading releases downloaded data as it is used, so it cannot be extracted again without downloading again.

Report security problems privately as described in [SECURITY.md](SECURITY.md).

## Layout

```text
crates/onezip-core/    Engine: extract(), compress(), list(), test(), hash()
crates/onezip-cli/     onezipc.exe command line
crates/onezip-app/     OneZip.exe window (Slint), single instance, task queue, progress windows
crates/onezip-shell/   onezip_shell.dll, Windows 11 context menu (IExplorerCommand)
packaging/             Sparse package manifest, build and signing script, install and uninstall scripts
third_party/           Modified copies of hatchzip-core and lzma-rust2
tools/uitest/          Context menu and window tests driven by UI Automation
docs/                  Original design notes (Chinese)
```

## Development

Requires Rust 1.93 or later with the MSVC toolchain, and Visual Studio Build Tools with the C++ workload and the Windows SDK.

```powershell
cargo build --release
cargo test --workspace
powershell -ExecutionPolicy Bypass -File packaging\build-package.ps1   # target\dist\OneZip-<version>-x64.zip
```

Command line:

```powershell
onezipc x D:\Downloads\big.zip              # smart extraction; asks before deleting the archive
onezipc x D:\Downloads\big.zip --keep       # keep the archive
onezipc x https://example.com/data.tar.zst -o D:\data
onezipc a D:\backup.7z D:\Data -l 7 -p secret
onezipc t D:\backup.7z
onezipc h D:\file.iso -a sha256
```

Ctrl+C pauses an extraction at a safe point; run the same command again to continue.

With OneZip installed, `tools\uitest\regress.ps1` exercises every context menu item and the progress windows. It moves the mouse and types, so leave the computer alone while it runs.

## Versioning

The version is in the workspace `Cargo.toml`. Releases follow [Semantic Versioning](https://semver.org/); changes are recorded in [CHANGELOG.md](CHANGELOG.md).

## Signing

Release builds are produced by the [Build workflow](.github/workflows/build.yml) on GitHub Actions from the tagged source in this repository. The sparse package that registers the Windows 11 context menu is signed with a self-signed certificate (`CN=OneZip Dev`), which is why installation asks to trust it. The executables are unsigned.

### Privacy

OneZip does not collect or send personal data. It contacts the network only when you extract an `http(s)://` address, and only that address.

## License and third-party content

Original source and documentation are under the [MIT License](LICENSE). Third-party content, including the modified copies in `third_party/`, keeps its own terms; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg)](https://slint.dev)
