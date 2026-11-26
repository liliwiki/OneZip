# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-26

First release.

### Added

- One engine over HatchZip (local in-place extraction) and Peel (extract while downloading), with smart placement, nested archive unpacking, listing, testing, and resume after interruption.
- RAR extraction through HatchZip's UnRAR support: RAR5 and RAR4, volumes, passwords, and encrypted file names.
- Extraction of iso, udf, wim, esd, cab, arj, lzh, dmg, vhd, vhdx, vmdk, vdi, qcow2, msi, chm, cpio, rpm, deb, xar, squashfs, hfs, apfs, and more through 7-Zip's 7z.dll (26.06).
- In-place extraction for iso (ISO9660, Joliet, UDF), wim/esd, cab, arj, and lzh: OneZip reads each format's layout, checks it against 7z.dll, and frees the archive's space batch by batch with resume after interruption. The context menu offers it only for formats that support it.
- Extract while downloading for every supported format: the 7z.dll formats download into a sparse file in parallel, extract as the data arrives, and release it as they go; zip, 7z, and rar from a URL accept passwords.
- Compression to 7z, zip, tar.zst, tar.xz, and tar.gz, with AES-256 for 7z and zip. Multi-threaded 7z and tar.xz report real progress and stop promptly when cancelled.
- Checksums: CRC-32, CRC-64, MD5, SHA-1, SHA-256, and SHA-512 for files and folders.
- `OneZip.exe` window with a task queue, pause and resume, password prompts, and compression options.
- Progress windows for context menu operations, in the style of 7-Zip; they close when the work is done.
- Windows 11 first-level context menu through a sparse package and `IExplorerCommand`, with the classic menu on Windows 10.
- `onezipc.exe` command line.
- `OneZip-<version>-Setup.exe` installer that installs by itself, without PowerShell scripts: per user, choice of install folder, no administrator rights except for protected folders and for trusting the sparse package certificate once on Windows 11. The install folder holds only the program, `uninstall.exe`, and `licenses\`; the list of installed files is kept in the registry, so an upgrade removes files the old version left behind and uninstalling removes only what was installed. Uninstalling asks for confirmation and whether to delete the settings too.
- "Extract, then delete the archive": deletes the archive, every volume of a split archive, only after the whole extraction succeeded, to the Recycle Bin by default. The in-place mode is now called "extract while deleting the archive" and explains its risk in a confirmation window before it starts.
- Settings window with General, Extract, Compress, Context menu, Updates, and About pages, including light and dark themes and per-item control of the context menu.
- Automatic and manual update checks against GitHub releases; "Update now" verifies the installer's SHA-256, installs over the current version, and reopens OneZip.
- Error log in `%LOCALAPPDATA%\OneZip\onezip.log`, and "Report a problem", which opens a prefilled GitHub issue.
- `packaging/release.ps1`, which checks, packages, and uploads a release from the maintainer's machine.
- Context menu regression tests driven by UI Automation.

### Changed

- Modified copy of hatchzip-core: on Windows, extracted files are flushed to disk in parallel per batch. Extracting 30,000 small files went from about 150 s to 26 s.
- Extraction that keeps the archive hands zip, 7z, and rar to 7z.dll, as fast as 7-Zip.
- Zip passwords with non-ASCII characters are encoded in the system code page, as 7-Zip does, so 7-Zip opens OneZip's encrypted zips; extraction accepts both that and UTF-8.
- The GitHub workflow only runs clippy and the tests; releases are built and uploaded locally so the sparse package keeps the same signing certificate.
- Modified copy of Peel 0.8.0: WinZip AE-2 encrypted zip entries no longer fail the CRC check, and 7z archives with encrypted file names decrypt their header.
- Modified copy of lzma-rust2 0.23: the multi-threaded encoders report progress and honour cancellation.

[Unreleased]: https://github.com/liliwiki/OneZip/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/liliwiki/OneZip/releases/tag/v0.1.0
