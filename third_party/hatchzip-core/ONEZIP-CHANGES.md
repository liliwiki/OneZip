# OneZip 对 hatchzip-core 的修改

来源：crates.io 上的 `hatchzip-core` 0.1.10（<https://github.com/xevrion/hatchzip>），
许可 MIT OR Apache-2.0，与 OneZip 相同。

## Windows：批次提交时并行刷盘

原版在 Windows 上每写完一个文件就调用一次 `FlushFileBuffers`（`sync_all`），
刷盘一个接一个排队，每个约 3 ms。解压 3 万个小文件要两分半钟，其中大半在等刷盘。

改为：

- 文件写完、改成正式名字后只记下路径（`platform::defer_sync`），不立即刷盘；
- 批次提交（`platform::sync_batch`，在写日志之前调用）时，用 32 个线程并行刷完
  这一批的所有文件。

日志依旧只记录已经落盘的文件，断电、被杀后的续传与"边解边删"的安全性不变。
Linux 与 macOS 的行为没有改动。

涉及文件：`src/platform/windows.rs`、`src/platform/linux.rs`、
`src/platform/macos.rs`、`src/engine.rs`（搜索 `DEFER_FILE_SYNC`）。
