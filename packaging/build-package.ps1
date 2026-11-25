<#
  构建 OneZip 发布包（在开发机上运行）。

  产出 target\dist\OneZip-<版本>-x64\ 和同名 .zip，里面有：
    OneZip.exe              主程序（窗口 + 右键菜单的执行者）
    onezipc.exe             命令行
    onezip_shell.dll        Windows 11 第一层右键菜单
    OneZip-Shell.msix       稀疏包（只含清单和图标，给 DLL 一个应用身份），已签名
    OneZip-Dev.cer          签名证书的公钥，安装时导入"受信任的人"
    install.ps1 / uninstall.ps1

  签名用的是自签名证书（CN=OneZip Dev），首次运行时在当前用户的证书库里创建。
  正式发布时换成受信任 CA 签发的代码签名证书，用 -CertThumbprint 指定即可，
  用户就不必再导入证书。
#>
param(
    [string]$Version = "",
    [string]$CertThumbprint = "",
    [string]$Publisher = "CN=OneZip Dev"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

if (-not $Version) {
    $Version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
}
# 包版本第四段每次打包递增（自 2026 年起的小时数），升级时资源管理器和任务栏会
# 重新读取图标等资源，而不是沿用同版本的缓存。
$build = [int][math]::Floor(((Get-Date) - [datetime]'2026-01-01').TotalHours) % 65535
$pkgVersion = "$Version.$build"

# ---- 编译 ----
# 本机把 Rust 装在 D:\Tools\Rust；在 GitHub Actions 等别的环境里用 PATH 上已有的 cargo。
if (Test-Path 'D:\Tools\Rust\cargo\bin\cargo.exe') {
    $env:RUSTUP_HOME = 'D:\Tools\Rust\rustup'
    $env:CARGO_HOME = 'D:\Tools\Rust\cargo'
    $env:Path = "D:\Tools\Rust\cargo\bin;$env:Path"
}
& cargo build --release -p onezip-app -p onezip-cli -p onezip-shell
if ($LASTEXITCODE -ne 0) { throw "编译失败" }

# ---- 找 Windows SDK 工具 ----
$kits = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
$sdkBin = Get-ChildItem $kits -Directory | Where-Object { $_.Name -match '^10\.' } |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64' } |
    Where-Object { Test-Path (Join-Path $_ 'makeappx.exe') } | Select-Object -First 1
if (-not $sdkBin) { throw "找不到 Windows SDK 的 makeappx.exe" }
$makeappx = Join-Path $sdkBin 'makeappx.exe'
$signtool = Join-Path $sdkBin 'signtool.exe'
$makepri = Join-Path $sdkBin 'makepri.exe'

# ---- 整理发布目录 ----
$name = "OneZip-$Version-x64"
$dist = Join-Path $root "target\dist\$name"
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force $dist | Out-Null
$rel = Join-Path $root 'target\release'
Copy-Item "$rel\OneZip.exe", "$rel\onezipc.exe", "$rel\onezip_shell.dll" $dist
Copy-Item LICENSE, THIRD_PARTY_NOTICES.md, README.md, README.zh-CN.md $dist
Copy-Item "$PSScriptRoot\install.ps1", "$PSScriptRoot\uninstall.ps1" $dist

# ---- 稀疏包 ----
$stage = Join-Path $root 'target\sparse'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\Assets" | Out-Null
Copy-Item "$PSScriptRoot\sparse\Assets\*" "$stage\Assets"
(Get-Content "$PSScriptRoot\sparse\AppxManifest.xml" -Raw -Encoding UTF8).
    Replace('__PUBLISHER__', $Publisher).Replace('__VERSION__', $pkgVersion) |
    Set-Content "$stage\AppxManifest.xml" -Encoding UTF8
# 资源索引：让任务栏按 targetsize / altform-unplated 挑选图标（tools/make-assets.py 生成）。
# 没有它，任务栏只能拿到带底板的小图，显示成一块灰色方块。
$priconfig = Join-Path $root 'target\priconfig.xml'
& $makepri createconfig /cf $priconfig /dq zh-CN /pv 10.0.0 /o | Out-Host
if ($LASTEXITCODE -ne 0) { throw "makepri createconfig 失败" }
# 默认配置会按缩放比例拆出多个资源包（给 bundle 用）；单个包要全部放进 resources.pri。
[xml]$cfg = Get-Content $priconfig -Raw
$cfg.resources.SelectNodes('packaging') | ForEach-Object { [void]$_.ParentNode.RemoveChild($_) }
$cfg.Save($priconfig)
Get-ChildItem $stage -Filter 'resources*.pri' | Remove-Item
& $makepri new /pr $stage /cf $priconfig /mn "$stage\AppxManifest.xml" /of "$stage\resources.pri" /o | Out-Host
if ($LASTEXITCODE -ne 0) { throw "makepri new 失败" }
$msix = Join-Path $dist 'OneZip-Shell.msix'
& $makeappx pack /d $stage /p $msix /nv /o | Out-Host
if ($LASTEXITCODE -ne 0) { throw "makeappx 失败" }

# ---- 签名 ----
if (-not $CertThumbprint) {
    $cert = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object { $_.Subject -eq $Publisher -and $_.NotAfter -gt (Get-Date) } |
        Select-Object -First 1
    if (-not $cert) {
        Write-Host "创建自签名证书 $Publisher"
        $cert = New-SelfSignedCertificate -Type Custom -Subject $Publisher `
            -KeyUsage DigitalSignature -FriendlyName "OneZip Dev" `
            -CertStoreLocation Cert:\CurrentUser\My `
            -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
    }
    $CertThumbprint = $cert.Thumbprint
    Export-Certificate -Cert "Cert:\CurrentUser\My\$CertThumbprint" -FilePath (Join-Path $dist 'OneZip-Dev.cer') | Out-Null
}
& $signtool sign /fd SHA256 /sha1 $CertThumbprint /s My $msix | Out-Host
if ($LASTEXITCODE -ne 0) { throw "签名失败" }

# ---- 打成 zip ----
$zip = Join-Path $root "target\dist\$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path "$dist\*" -DestinationPath $zip
Write-Host "完成：$zip"
