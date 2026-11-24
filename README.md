# OneZip（暂定名）

Windows 一体化压缩管理工具：常规压缩/解压、智能解压、本地边解压边释放空间、边下载边解压边释放空间，统一 GUI 与右键菜单。

设计见 [docs/设计方案.md](docs/设计方案.md)。

## 现状

- `crates/onezip-core`：统一引擎入口 `onezip_core::extract()`
  - 本地文件 → [HatchZip](https://github.com/xevrion/hatchzip) 的 `hatchzip-core`：边解压边截断原压缩包，可暂停/断电恢复
  - http(s) URL → [Peel](https://github.com/agouin/peel)：并行分段下载、边下边解、打洞释放、断点续传
- `crates/onezip-cli`：`onezip.exe`

尚未实现：7z.dll 接入、常规压缩、后台任务进程、WinUI 3 界面、右键菜单。

## 用法

```
onezip x D:\Downloads\big.zip              # 解到 big\，边解边删（会先确认）
onezip x D:\Downloads\big.zip --keep       # 普通解压，保留原包
onezip x https://example.com/data.tar.zst -o D:\data
```

Ctrl+C 暂停并保存进度，再运行同样的命令即可继续。

## 构建（Windows）

需要 Rust（MSVC 工具链）和 Visual Studio Build Tools（C++）。

```
cargo build --release
cargo test --workspace
```

## 许可

MIT OR Apache-2.0。依赖的上游项目及其许可见 [NOTICE](NOTICE)。
