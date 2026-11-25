# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-25

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
- `OneZip-<version>-Setup.exe` installer (per user, choice of install folder, no administrator rights except for protected folders and for trusting the sparse package certificate once on Windows 11; uninstall removes only the files it installed), and a packaging script that builds and signs the sparse package.
- Context menu regression tests driven by UI Automation.

### Changed

- Modified copy of hatchzip-core: on Windows, extracted files are flushed to disk in parallel per batch. Extracting 30,000 small files went from about 150 s to 26 s.
- Modified copy of Peel 0.8.0: WinZip AE-2 encrypted zip entries no longer fail the CRC check, and 7z archives with encrypted file names decrypt their header.
- Modified copy of lzma-rust2 0.23: the multi-threaded encoders report progress and honour cancellation.

[Unreleased]: https://github.com/liliwiki/OneZip/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/liliwiki/OneZip/releases/tag/v0.1.0
