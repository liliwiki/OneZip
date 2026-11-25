# OneZip（暂定名）

Windows 一体化压缩管理工具：常规压缩/解压、智能解压、本地边解压边释放空间、边下载边解压边释放空间，统一图形界面与右键菜单（Windows 11 第一层菜单）。

设计见 [docs/设计方案.md](docs/设计方案.md)。

## 功能

| 功能 | 说明 | 引擎 |
|---|---|---|
| 压缩 | 7z（LZMA2，多线程，可固实，AES-256 含文件名加密）、zip（Deflate，AES-256）、tar.zst、tar.xz、tar.gz；先写临时文件，完成后才改名 | sevenz-rust2 / zip / zstd / lzma-rust2 / flate2 |
| 解压 | zip、7z、tar、tar.gz、tar.bz2、tar.xz、分卷（.001 / .z01）| [HatchZip](https://github.com/xevrion/hatchzip) `hatchzip-core` |
| 解压 | tar.zst、tar.lz4、.zst、.lz4、rar（RAR5/RAR4，无加密）| [Peel](https://github.com/agouin/peel) |
| 边解压边删除 | 逆序截断或打洞，逐批释放原压缩包空间；断电、被杀后可继续 | 两者 |
| 边下载边解压 | 并行分段下载、流式解码、已用数据立即打洞释放、断点续传 | Peel |
| 智能解压 | 只有一个顶层项时直接放旁边，否则建同名文件夹；嵌套包自动继续拆 | OneZip |
| 列表 / 测试 | 不改动压缩包 | HatchZip |

## 组成

- `crates/onezip-core`：统一引擎，`extract()` / `compress()` / `list()` / `test()`
- `crates/onezip-cli`：`onezipc.exe` 命令行
- `crates/onezip-app`：`OneZip.exe` 图形界面（Slint），单实例 + 任务队列（同时两个任务，可暂停/继续）
- `crates/onezip-shell`：`onezip_shell.dll`，Windows 11 第一层右键菜单（`IExplorerCommand`）
- `packaging/`：稀疏包清单、打包签名脚本、安装/卸载脚本

## 命令行

```
onezipc x D:\Downloads\big.zip              # 智能解压，保留原包确认后边解边删
onezipc x D:\Downloads\big.zip --keep       # 普通解压，保留原包
onezipc x https://example.com/data.tar.zst -o D:\data
onezipc a D:\backup.7z D:\资料 -l 7 -p 密码
onezipc l D:\backup.7z
onezipc t D:\backup.7z
```

Ctrl+C 暂停解压并保存进度，再运行同样的命令即可继续。

## 安装

运行 `packaging\build-package.ps1` 生成 `target\dist\OneZip-<版本>-x64.zip`，解压后运行：

```
powershell -ExecutionPolicy Bypass -File install.ps1
```

- 安装到 `%LOCALAPPDATA%\Programs\OneZip`，只对当前用户生效。
- Windows 11：注册稀疏包，右键菜单出现在第一层。开发版用自签名证书，安装时会弹一次管理员确认，把证书导入"受信任的人"。
- Windows 10：写入按用户的传统右键菜单。
- 卸载：在"应用和功能"里卸载，或运行 `uninstall.ps1`。

## 构建

需要 Rust（MSVC 工具链）和 Visual Studio Build Tools（C++、Windows SDK）。

```
cargo build --release
cargo test --workspace
```

装好之后可以跑右键菜单的界面回归测试（会操作鼠标键盘，运行时别动电脑；测试文件放在 `D:\OZT`）：

```
powershell -ExecutionPolicy Bypass -File tools\uitest\regress.ps1
```

改了 `assets/onezip-256.png` 后，用 `python tools/make-assets.py` 重新生成稀疏包的各尺寸图标。

## 许可

MIT OR Apache-2.0。依赖的上游项目及其许可见 [NOTICE](NOTICE)。
