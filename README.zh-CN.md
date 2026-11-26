# OneZip

[![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4)](https://github.com/liliwiki/OneZip)
[![Rust](https://img.shields.io/badge/Rust-1.93+-B7410E)](https://www.rust-lang.org/)
[![许可证：MIT](https://img.shields.io/badge/许可证-MIT-green.svg)](LICENSE)

[English](README.md) | **简体中文**

Windows 一体化压缩工具：常规压缩、智能解压、本地边解压边释放原压缩包空间、边下载边解压。解压引擎来自 [HatchZip](https://github.com/xevrion/hatchzip) 和 [Peel](https://github.com/agouin/peel)，菜单直接出现在 Windows 11 右键菜单的第一层。

> OneZip 是独立项目，与 NanaZip、7-Zip、HatchZip、Peel 没有隶属关系。

## 下载

在 [Releases](https://github.com/liliwiki/OneZip/releases) 页面下载 `OneZip-<版本>-Setup.exe`，双击运行即可。同一页面还有 `OneZip-<版本>-Setup.exe.sha256`，运行前请核对安装程序的 SHA-256。

需要 Windows 10 或 11（x64）。

### 安装

- 只给当前用户安装，默认装在 `%LOCALAPPDATA%\Programs\OneZip`，也可以在安装程序里换别的位置。
- 不需要管理员权限。只有两种情况会弹一次管理员确认：
  - 装到 `C:\Program Files` 这类受保护的文件夹；
  - Windows 11 上第一次安装：右键菜单通过稀疏包注册，这个包用的是自签名证书，要把证书加入"受信任的人"。以后升级不再需要。
- Windows 10 上使用传统的按用户右键菜单。
- 安装程序没有代码签名，SmartScreen 可能会拦一下，点「更多信息」→「仍要运行」。
- 安装目录里只有 `OneZip.exe`、`onezipc.exe`、`onezip_shell.dll`、`7z.dll`、`uninstall.exe` 和 `licenses\` 文件夹。

### 升级和卸载

- 新版本直接运行新的安装程序即可，会装到原来的位置，旧版本多出来的文件会被清掉。也可以在 OneZip 里点"立即更新"（见下面的[检查更新](#检查更新)）。
- 卸载：在系统设置的「已安装的应用」里卸载，或者运行安装目录里的 `uninstall.exe`。卸载前会确认，并可以选择是否同时删除 OneZip 的设置和出错记录。
- 卸载只删安装时放进去的文件。解压、压缩出来的文件不受影响。

## 功能

| 功能 | 说明 | 引擎 |
|---|---|---|
| 压缩 | 7z（LZMA2，多线程，可固实，AES-256 含文件名加密）、zip（Deflate，AES-256）、tar.zst、tar.xz、tar.gz；先写临时文件，完成后才改名。zip 的密码按系统代码页编码，和 7-Zip 一致 | sevenz-rust2、zip、zstd、lzma-rust2、flate2 |
| 解压（保留原压缩包） | zip、7z、rar | 7-Zip（7z.dll） |
| 解压 | tar、tar.gz、tar.bz2、tar.xz、分卷 7z（.7z.001） | HatchZip |
| 解压 | tar.zst、tar.lz4、.zst、.lz4 | Peel |
| 解压 | iso、udf、wim、esd、cab、arj、lzh、dmg、vhd、vhdx、vmdk、vdi、qcow2、msi、chm、cpio、rpm、deb、xar、squashfs、cramfs、hfs、apfs、z、lzma | 7-Zip（7z.dll） |
| 解压后删除压缩包 | 先完整解压，全部成功后再删除压缩包（分卷的每一卷都删）。默认移到回收站，可以在设置里改成直接删除。出错、取消或有警告时压缩包原样保留 | OneZip |
| 边解压边删除压缩包 | 解压过程中逐批释放原压缩包的空间，需要的额外空间远小于压缩包本身；断电、崩溃后可以接着解。上面所有格式都支持，只有磁盘镜像和安装包（dmg、vhd、vhdx、vmdk、vdi、qcow2、msi、chm 等）因为结构不允许而不支持 | HatchZip（zip、7z、rar 也用它）、Peel、OneZip |
| 边下载边解压 | 并行分段下载、数据到了就解、用过的数据立即释放、断点续传。上面所有格式都支持，带密码的 zip、7z、rar 也行；磁盘镜像和安装包下载的数据要到解完才释放 | Peel、OneZip |
| 智能解压 | 只有一个顶层项时直接放在压缩包旁边，否则放进同名文件夹；嵌套的压缩包也可以接着解开 | OneZip |
| 列表、测试、校验值 | 文件和文件夹的 CRC-32、CRC-64、MD5、SHA-1、SHA-256、SHA-512 | OneZip、HatchZip |

## 右键菜单

右键压缩包时，OneZip 子菜单里有：

- 智能解压 / 解压到 "名字\" / 解压到当前位置 / 解压文件…
- 解压后删除压缩包 / 边解压边删除压缩包
- 测试压缩包
- 添加到压缩包… / 添加到 "名字.7z" / 添加到 "名字.zip"
- CRC-32 / MD5 / SHA-256 / 校验值（全部算法）

说明：
- "边解压边删除压缩包"只在支持的格式上出现。其他文件和文件夹只显示压缩和校验值这几项。
- 每一项都可以在设置的"右键菜单"页里隐藏，也可以把整个菜单关掉。MD5 默认隐藏。
- 每个操作弹出一个自己的进度窗口，显示已用时间、剩余时间、速度，可以暂停、取消，做完自动关闭。只有直接打开 OneZip 时才会出现主窗口。

## 设置

在主窗口点"设置"打开，改动立即生效。设置保存在注册表 `HKCU\Software\OneZip` 里。

| 页 | 内容 |
|---|---|
| 常规 | 主题（跟随系统 / 浅色 / 深色）、恢复全部默认 |
| 解压 | 默认解压方式；同名文件替换、跳过或自动改名；解开套着的压缩包；"解压后删除"的压缩包移到回收站还是直接删除；边解压边删除前是否确认；边下载边解压的默认保存位置和连接数 |
| 压缩 | 默认格式、级别、线程数、7z 固实压缩、加密文件名 |
| 右键菜单 | 总开关，逐项显示或隐藏 |
| 更新 | 自动检查更新、立即检查、已跳过的版本、打开发布页 |
| 关于 | 版本、用到的开源项目、GitHub 主页、许可证、报告问题 |

## 检查更新

- 打开主窗口时自动检查，每天最多一次，可以在设置里关掉。也可以在设置的"更新"页手动检查。
- 有新版本时会弹出提示，可以选"立即更新""跳过这个版本"或"以后再说"。
- "立即更新"会下载新的安装程序，核对 SHA-256 一致后才运行；安装程序等 OneZip 退出后装到原来的位置，再重新打开 OneZip。

## 安全

- "边解压边删除压缩包"会在解压过程中删除原压缩包，无法撤销。进度保存在磁盘上，中断后可以从停下的地方继续，但无法再找回的数据请另外备份。开始前会弹出说明，确认后才开始（可以选择以后不再提醒）。
- "解压后删除压缩包"在全部解压成功后才删，默认移到回收站。
- 边下载边解压会释放用过的下载数据，想再解压一次就得重新下载。

安全问题请按 [SECURITY.md](SECURITY.md) 私下报告。

## 出错记录

任务失败和程序崩溃会记到 `%LOCALAPPDATA%\OneZip\onezip.log`，只保存在本机。设置 → 关于 →"报告问题"会在浏览器里打开 GitHub 的新问题页，自动填好版本号、Windows 版本和最近的出错记录，提交前可以先看、可以删改。

## 目录结构

```text
crates/onezip-core/      引擎：extract()、compress()、list()、test()、hash()
crates/onezip-formats/   格式表：哪个格式用哪个引擎、能不能边解压边删除
crates/onezip-settings/  设置的读写（注册表）
crates/onezip-menu/      右键菜单的菜单项
crates/onezip-cli/       onezipc.exe 命令行
crates/onezip-app/       OneZip.exe 窗口（Slint）：任务队列、进度窗口、设置、检查更新
crates/onezip-shell/     onezip_shell.dll，Windows 11 右键菜单（IExplorerCommand）
crates/onezip-setup/     安装程序 OneZip-<版本>-Setup.exe，也是卸载程序
packaging/               稀疏包清单、打包签名脚本、发布脚本
third_party/             改过的 hatchzip-core、Peel、lzma-rust2；7-Zip 的 7z.dll
tools/uitest/            用 UI 自动化测试右键菜单和窗口
docs/                    最初的设计草案、发布说明
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
onezipc x D:\Downloads\big.zip              # 智能解压，边解压边删除，开始前会先询问
onezipc x D:\Downloads\big.zip --keep       # 保留原包
onezipc x https://example.com/data.tar.zst -o D:\data
onezipc a D:\backup.7z D:\资料 -l 7 -p 密码
onezipc t D:\backup.7z
onezipc h D:\file.iso -a sha256
```

解压时按 Ctrl+C 会在安全点暂停，再运行同样的命令就能继续。

装好 OneZip 后，`tools\uitest\regress.ps1` 会逐项测试右键菜单和进度窗口。它会移动鼠标、敲键盘，运行期间请不要操作电脑。

### 发布

在维护者自己的电脑上运行（需要 [GitHub CLI](https://cli.github.com/) 和签名证书）：

```powershell
powershell -ExecutionPolicy Bypass -File packaging\release.ps1 -DryRun   # 只检查和打包，不上传
powershell -ExecutionPolicy Bypass -File packaging\release.ps1           # 检查、打包、上传，上传前要求输入版本号确认
```

脚本会检查代码已经提交并推送到 `main`、这个版本还没发布过、本机有签名证书，然后跑 clippy 和测试、打包，上传安装程序和 `.sha256` 文件。发布说明取自 `docs/release-notes/v<版本>-zh-CN.md`。

## 版本

版本号在工作区的 `Cargo.toml` 里。发布遵循[语义化版本](https://semver.org/lang/zh-CN/)，改动记录在 [CHANGELOG.md](CHANGELOG.md)。

## 签名

发布包由维护者在自己的电脑上用 [`packaging/release.ps1`](packaging/release.ps1) 从已推送到 `main` 的源码构建并上传，Release 的标签指向这次构建所用的提交；GitHub Actions 上的 [Check 工作流](.github/workflows/build.yml)只做 clippy 和测试。注册 Windows 11 右键菜单的稀疏包用自签名证书（`CN=OneZip Dev`）签名，所以安装时需要信任它。程序本身没有代码签名。

### 隐私

OneZip 不收集、不发送个人数据。只在这些时候联网：
- 解压 `http(s)://` 地址时，只访问这个地址；
- 检查更新时，访问 GitHub（`api.github.com` 和 `github.com`），只查询最新版本，下载更新时下载安装程序；
- 你点"报告问题"时，在浏览器里打开 GitHub 的新问题页，内容要你自己提交才会发出。

## 许可与第三方内容

原创的源码和文档使用 [MIT 许可证](LICENSE)。第三方内容（包括 `third_party/` 里改过的副本）适用各自的条款，见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-light.svg)](https://slint.dev)
