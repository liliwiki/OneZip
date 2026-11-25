# OneZip

[![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4)](https://github.com/liliwiki/OneZip)
[![Rust](https://img.shields.io/badge/Rust-1.93+-B7410E)](https://www.rust-lang.org/)
[![许可证：MIT](https://img.shields.io/badge/许可证-MIT-green.svg)](LICENSE)

[English](README.md) | **简体中文**

Windows 一体化压缩工具：常规压缩、智能解压、本地边解压边释放原压缩包空间、边下载边解压。解压引擎来自 [HatchZip](https://github.com/xevrion/hatchzip) 和 [Peel](https://github.com/agouin/peel)，菜单直接出现在 Windows 11 右键菜单的第一层。

> OneZip 是独立项目，与 NanaZip、7-Zip、HatchZip、Peel 没有隶属关系。

## 下载

在 [Releases](https://github.com/liliwiki/OneZip/releases) 页面下载 `OneZip-<版本>-Setup.exe`，双击运行即可。

只给当前用户安装，默认装在 `%LOCALAPPDATA%\Programs\OneZip`，也可以在安装程序里换别的位置。不需要管理员权限，除非装到 `C:\Program Files` 这类受保护的文件夹（会弹一次管理员确认）。另外：Windows 11 上通过稀疏包注册右键菜单，这个包用的是自签名证书，第一次安装时会弹一次管理员确认，把证书加入"受信任的人"。Windows 10 上使用传统的按用户右键菜单。安装程序没有签名，SmartScreen 可能会拦一下，点「更多信息」→「仍要运行」。要卸载的话，在系统设置的「已安装的应用」里卸载。

需要 Windows 10 或 11（x64）。运行前请核对 Release 页面公布的 SHA-256。

## 功能

| 功能 | 说明 | 引擎 |
|---|---|---|
| 压缩 | 7z（LZMA2，多线程，可固实，AES-256 含文件名加密）、zip（Deflate，AES-256）、tar.zst、tar.xz、tar.gz；先写临时文件，完成后才改名 | sevenz-rust2、zip、zstd、lzma-rust2、flate2 |
| 解压 | zip、7z、tar、tar.gz、tar.bz2、tar.xz、分卷（.001、.z01） | HatchZip |
| 解压 | rar：RAR5、RAR4、分卷、密码、加密文件名 | HatchZip（UnRAR） |
| 解压 | tar.zst、tar.lz4、.zst、.lz4 | Peel |
| 解压 | iso、udf、wim、esd、cab、arj、lzh、dmg、vhd、vhdx、vmdk、vdi、qcow2、msi、chm、cpio、rpm、deb、xar、squashfs、cramfs、hfs、apfs、z、lzma | 7-Zip（7z.dll） |
| 边解压边删除 | 解压过程中逐批释放原压缩包的空间；断电、崩溃后可以接着解。上面所有格式都支持，只有磁盘镜像和安装包（dmg、vhd、vhdx、vmdk、vdi、qcow2、msi、chm 等）因为结构不允许而不支持 | HatchZip、Peel、OneZip |
| 边下载边解压 | 并行分段下载、数据到了就解、用过的数据立即释放、断点续传。上面所有格式都支持，带密码的 zip、7z、rar 也行；磁盘镜像和安装包下载的数据要到解完才释放 | Peel、OneZip |
| 智能解压 | 只有一个顶层项时直接放在压缩包旁边，否则放进同名文件夹；嵌套的压缩包也会接着解开 | OneZip |
| 列表、测试、校验值 | 文件和文件夹的 CRC-32、CRC-64、MD5、SHA-1、SHA-256、SHA-512 | OneZip、HatchZip |

## 右键菜单

右键压缩包时，OneZip 子菜单里有：

- 智能解压 / 解压到 "名字\" / 解压到当前位置 / 解压文件…
- 解压后删除压缩包（边解压边释放空间）
- 测试压缩包
- 添加到压缩包… / 添加到 "名字.7z" / 添加到 "名字.zip"
- CRC-32 / MD5 / SHA-256 / 校验值（全部算法）

"解压后删除压缩包"只在支持的格式上出现。其他文件和文件夹只显示压缩和校验值这几项。每个操作弹出一个自己的进度窗口，显示已用时间、剩余时间、速度，可以暂停、取消，做完自动关闭。只有直接打开 OneZip 时才会出现主窗口。

## 安全

"解压后删除压缩包"会在解压过程中删除原压缩包，无法撤销。进度保存在磁盘上，中断后可以从停下的地方继续，但无法再找回的数据请另外备份。边下载边解压会释放用过的下载数据，想再解压一次就得重新下载。

安全问题请按 [SECURITY.md](SECURITY.md) 私下报告。

## 目录结构

```text
crates/onezip-core/    引擎：extract()、compress()、list()、test()、hash()
crates/onezip-cli/     onezipc.exe 命令行
crates/onezip-app/     OneZip.exe 窗口（Slint），单实例、任务队列、进度窗口
crates/onezip-shell/   onezip_shell.dll，Windows 11 右键菜单（IExplorerCommand）
crates/onezip-setup/   安装程序 OneZip-<版本>-Setup.exe
packaging/             稀疏包清单、打包签名脚本、安装与卸载脚本
third_party/           改过的 hatchzip-core、Peel、lzma-rust2；7-Zip 的 7z.dll
tools/uitest/          用 UI 自动化测试右键菜单和窗口
docs/                  最初的设计草案
```

## 开发

需要 Rust 1.93 或更新版本（MSVC 工具链），以及装有 C++ 工作负载和 Windows SDK 的 Visual Studio Build Tools。

```powershell
cargo build --release
cargo test --workspace
powershell -ExecutionPolicy Bypass -File packaging\build-package.ps1   # 生成 target\dist\OneZip-<版本>-Setup.exe
```

命令行：

```powershell
onezipc x D:\Downloads\big.zip              # 智能解压，删除原包前会先询问
onezipc x D:\Downloads\big.zip --keep       # 保留原包
onezipc x https://example.com/data.tar.zst -o D:\data
onezipc a D:\backup.7z D:\资料 -l 7 -p 密码
onezipc t D:\backup.7z
onezipc h D:\file.iso -a sha256
```

解压时按 Ctrl+C 会在安全点暂停，再运行同样的命令就能继续。

装好 OneZip 后，`tools\uitest\regress.ps1` 会逐项测试右键菜单和进度窗口。它会移动鼠标、敲键盘，运行期间请不要操作电脑。

## 版本

版本号在工作区的 `Cargo.toml` 里。发布遵循[语义化版本](https://semver.org/lang/zh-CN/)，改动记录在 [CHANGELOG.md](CHANGELOG.md)。

## 签名

发布包由 GitHub Actions 上的 [Build 工作流](.github/workflows/build.yml)从本仓库打了标签的源码构建。注册 Windows 11 右键菜单的稀疏包用自签名证书（`CN=OneZip Dev`）签名，所以安装时需要信任它。程序本身没有代码签名。

### 隐私

OneZip 不收集、不发送个人数据。只有你解压 `http(s)://` 地址时才会联网，而且只访问这个地址。

## 许可与第三方内容

原创的源码和文档使用 [MIT 许可证](LICENSE)。第三方内容（包括 `third_party/` 里改过的副本）适用各自的条款，见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg)](https://slint.dev)
