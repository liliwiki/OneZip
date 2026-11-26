# OneZip

[![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4)](https://github.com/liliwiki/OneZip)
[![Rust](https://img.shields.io/badge/Rust-1.93+-B7410E)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

**English** | [简体中文](README.zh-CN.md)

An all-in-one archive tool for Windows: compress, extract smartly, extract in place while freeing the archive's space, and extract while downloading. It builds on the [HatchZip](https://github.com/xevrion/hatchzip) and [Peel](https://github.com/agouin/peel) engines and adds its menu to the first level of the Windows 11 context menu. The interface is in Simplified Chinese.

> OneZip is an independent project. It is not affiliated with NanaZip, 7-Zip, HatchZip, or Peel.

## Download

Download `OneZip-<version>-Setup.exe` from the [Releases](https://github.com/liliwiki/OneZip/releases) page and run it. The same page has `OneZip-<version>-Setup.exe.sha256`; check the installer's SHA-256 before running it.

Requires Windows 10 or 11 (x64).

### Installation

- It installs for the current user, by default to `%LOCALAPPDATA%\Programs\OneZip`; the installer lets you choose another folder.
- No administrator rights are needed, except for one approval in two cases:
  - installing into a protected folder such as `C:\Program Files`;
  - the first installation on Windows 11: the context menu is registered through a sparse package signed with a self-signed certificate, which has to be added to "Trusted People". Later upgrades don't ask again.
- On Windows 10 it uses the classic per-user context menu.
- The installer is not code-signed, so SmartScreen may warn; choose "More info" → "Run anyway".
- The install folder holds only `OneZip.exe`, `onezipc.exe`, `onezip_shell.dll`, `7z.dll`, `uninstall.exe`, and a `licenses\` folder.

### Upgrading and uninstalling

- To upgrade, run the new installer; it installs to the same folder and removes files the old version left behind. OneZip can also do this itself with "Update now" (see [Updates](#updates)).
- To uninstall, use "Installed apps" in Windows Settings, or run `uninstall.exe` in the install folder. It asks for confirmation and whether to delete OneZip's settings and error log too.
- Uninstalling removes only the files the installer put there. Files you extracted or compressed are left alone.

## Features

| Feature | Details | Engine |
|---|---|---|
| Compress | 7z (LZMA2, multi-threaded, optional solid mode, AES-256 with encrypted file names), zip (Deflate, AES-256), tar.zst, tar.xz, tar.gz; written to a temporary file and renamed when done. Zip passwords use the system code page, as 7-Zip does | sevenz-rust2, zip, zstd, lzma-rust2, flate2 |
| Extract (keeping the archive) | zip, 7z, rar | 7-Zip (7z.dll) |
| Extract | tar, tar.gz, tar.bz2, tar.xz, split 7z (.7z.001) | HatchZip |
| Extract | tar.zst, tar.lz4, .zst, .lz4 | Peel |
| Extract | iso, udf, wim, esd, cab, arj, lzh, dmg, vhd, vhdx, vmdk, vdi, qcow2, msi, chm, cpio, rpm, deb, xar, squashfs, cramfs, hfs, apfs, z, lzma | 7-Zip (7z.dll) |
| Extract, then delete the archive | Extracts everything first and deletes the archive (every volume of a split archive) only if all of it succeeded. Goes to the Recycle Bin by default; Settings can make it delete permanently. On an error, a cancel, or a warning the archive is kept | OneZip |
| Extract while deleting the archive | Frees the archive's space batch by batch during extraction, so it needs far less free space than the archive itself; resumes after a power cut or crash. Works for every format above except disk images and installers (dmg, vhd, vhdx, vmdk, vdi, qcow2, msi, chm, and similar), whose layout doesn't allow it | HatchZip (also for zip, 7z, rar), Peel, OneZip |
| Extract while downloading | Parallel ranged downloads, extracts data as it arrives, releases used data right away, resumes after interruption. Works for every format above, including password-protected zip, 7z, and rar; disk images and installers release the downloaded data only when extraction finishes | Peel, OneZip |
| Smart extraction | Puts a single top-level item next to the archive, otherwise extracts into a folder named after it; can unpack nested archives too | OneZip |
| List, test, checksums | CRC-32, CRC-64, MD5, SHA-1, SHA-256, SHA-512 for files and folders | OneZip, HatchZip |

## Context menu

Right-clicking an archive shows these items in the OneZip submenu (the labels are in Chinese):

- Smart extract / Extract to "name\" / Extract here / Extract files…
- Extract, then delete the archive / Extract while deleting the archive
- Test archive
- Add to archive… / Add to "name.7z" / Add to "name.zip"
- CRC-32 / MD5 / SHA-256 / Checksums (all algorithms)

Notes:
- "Extract while deleting the archive" appears only for formats that support it. Other files and folders show only the compression and checksum items.
- Settings → Context menu can hide any item or turn the whole menu off. MD5 is hidden by default.
- Each operation opens its own progress window with elapsed time, time left, and speed; it can be paused or cancelled and closes when done. The main window appears only when you open OneZip directly.

## Settings

Open them with "Settings" in the main window; changes apply immediately. They are stored in the registry under `HKCU\Software\OneZip`.

| Page | Contents |
|---|---|
| General | Theme (follow Windows / light / dark), restore all defaults |
| Extract | Default extraction; replace, skip, or rename files that already exist; unpack nested archives; whether "extract, then delete" uses the Recycle Bin; confirm before extracting while deleting; default folder and connection count for extracting while downloading |
| Compress | Default format, level, threads, 7z solid mode, encrypted file names |
| Context menu | Master switch and per-item visibility |
| Updates | Automatic update check, check now, skipped versions, open the releases page |
| About | Version, open-source projects used, GitHub page, licenses, report a problem |

## Updates

- OneZip checks when the main window opens, at most once a day; Settings can turn this off. Settings → Updates also checks on demand.
- When a new version is out, a prompt offers "Update now", "Skip this version", or "Later".
- "Update now" downloads the new installer and runs it only if its SHA-256 matches. The installer waits for OneZip to close, installs to the same folder, and opens OneZip again.

## Safety

- "Extract while deleting the archive" deletes the archive as it extracts and cannot be undone. Progress is saved on disk so an interrupted extraction can continue, but back up anything you cannot get again. It explains this and asks before it starts (you can turn the prompt off).
- "Extract, then delete the archive" deletes only after everything extracted successfully, to the Recycle Bin by default.
- Extracting while downloading releases downloaded data as it is used, so extracting again means downloading again.

Report security issues privately as described in [SECURITY.md](SECURITY.md).

## Error log

Failed tasks and crashes are logged to `%LOCALAPPDATA%\OneZip\onezip.log`, which stays on your computer. Settings → About → "Report a problem" opens a new GitHub issue in your browser with the version, the Windows version, and the latest log entries filled in; you can read and edit it before submitting.

## Layout

```text
crates/onezip-core/      Engine: extract(), compress(), list(), test(), hash()
crates/onezip-formats/   Format table: which engine handles a format, whether it can extract in place
crates/onezip-settings/  Settings storage (registry)
crates/onezip-menu/      Context menu items
crates/onezip-cli/       onezipc.exe command line
crates/onezip-app/       OneZip.exe window (Slint): task queue, progress windows, settings, updates
crates/onezip-shell/     onezip_shell.dll, the Windows 11 context menu (IExplorerCommand)
crates/onezip-setup/     OneZip-<version>-Setup.exe installer, which is also the uninstaller
packaging/               Sparse package manifest, packaging and signing script, release script
third_party/             Modified hatchzip-core, Peel, and lzma-rust2; 7-Zip's 7z.dll
tools/uitest/            UI Automation tests for the context menu and windows
docs/                    The original design draft and release notes
```

## Development

Requires Rust 1.93 or newer (MSVC toolchain) and Visual Studio Build Tools with the C++ workload and the Windows SDK.

```powershell
cargo build --release
cargo test --workspace
powershell -ExecutionPolicy Bypass -File packaging\build-package.ps1   # target\dist\OneZip-<version>-Setup.exe
```

Command line:

```powershell
onezipc x D:\Downloads\big.zip              # smart extraction, deleting the archive as it goes; asks first
onezipc x D:\Downloads\big.zip --keep       # keep the archive
onezipc x https://example.com/data.tar.zst -o D:\data
onezipc a D:\backup.7z D:\docs -l 7 -p password
onezipc t D:\backup.7z
onezipc h D:\file.iso -a sha256
```

Ctrl+C during extraction pauses at a safe point; run the same command again to continue.

With OneZip installed, `tools\uitest\regress.ps1` tests every context menu item and the progress windows. It moves the mouse and types, so leave the computer alone while it runs.

### Releasing

Run on the maintainer's own machine (needs the [GitHub CLI](https://cli.github.com/) and the signing certificate):

```powershell
powershell -ExecutionPolicy Bypass -File packaging\release.ps1 -DryRun   # checks and packages, uploads nothing
powershell -ExecutionPolicy Bypass -File packaging\release.ps1           # checks, packages, uploads; asks you to type the version first
```

The script checks that the code is committed and pushed to `main`, that this version hasn't been released, and that the signing certificate is present; then it runs clippy and the tests, packages, and uploads the installer and its `.sha256` file. The release notes come from `docs/release-notes/v<version>-zh-CN.md`.

## Versioning

The version is in the workspace `Cargo.toml`. Releases follow [Semantic Versioning](https://semver.org/); changes are recorded in [CHANGELOG.md](CHANGELOG.md).

## Signing

Release builds are made by the maintainer on their own machine with [`packaging/release.ps1`](packaging/release.ps1) from source already pushed to `main`, and the release tag points at the commit they were built from; the [Check workflow](.github/workflows/build.yml) on GitHub Actions runs clippy and the tests only. The sparse package that registers the Windows 11 context menu is signed with a self-signed certificate (`CN=OneZip Dev`), which is why installation asks to trust it. The executables are unsigned.

### Privacy

OneZip does not collect or send personal data. It goes online only:
- when you extract an `http(s)://` address, and then only to that address;
- to check for updates, contacting GitHub (`api.github.com` and `github.com`) for the latest version, and downloading the installer when you update;
- when you click "Report a problem", which opens a GitHub issue page in your browser; nothing is sent until you submit it.

## License and third-party content

Original source and documentation are under the [MIT License](LICENSE). Third-party content, including the modified copies in `third_party/`, keeps its own terms; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg)](https://slint.dev)
