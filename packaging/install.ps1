<#
  安装 OneZip（只对当前用户）。由 OneZip-<版本>-Setup.exe 解到临时目录后调用，
  也可以在发布目录里手动运行：
    powershell -ExecutionPolicy Bypass -File install.ps1 [-Dest <安装位置>]

  - 程序复制到 -Dest（默认：已安装的位置，否则 %LOCALAPPDATA%\Programs\OneZip）。
    选定的文件夹叫 OneZip 且是空的（或还不存在）、或者本来就装着 OneZip 时直接装进去，
    否则装进它下面的 OneZip 子文件夹，免得卸载时碰到别的文件。
    装到需要管理员权限的地方（如 C:\Program Files）时，弹一次管理员确认，把这个文件夹
    交给当前用户修改；其余步骤仍以当前用户身份进行，以后升级、卸载都不再要管理员。
  - 换了位置时，旧位置里装过的文件会删掉。
  - Windows 11：注册稀疏包，右键菜单出现在第一层。
    如果包用的是自签名证书，需要把 OneZip-Dev.cer 导入本机"受信任的人"，
    这一步会弹出一次管理员确认（UAC）。
  - Windows 10：写入按用户的传统右键菜单（本来就直接显示）。
  - 开始菜单快捷方式、"应用和功能"里的卸载项。
  - 装了哪些文件记在安装目录的 installed-files.txt 里，卸载只删这些。
#>
param([string]$Dest = "")
$ErrorActionPreference = "Stop"
$src = $PSScriptRoot
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OneZip'
$isWin11 = [Environment]::OSVersion.Version.Build -ge 22000

$old = $null
if (Test-Path $key) { $old = (Get-ItemProperty $key -ErrorAction SilentlyContinue).InstallLocation }
if (-not $Dest) {
    $Dest = if ($old) { $old } else { Join-Path $env:LOCALAPPDATA 'Programs\OneZip' }
}
# 装进选定的文件夹本身，只有它叫 OneZip 且是空的/还不存在、或者本来就装着 OneZip；否则装进它下面的
# OneZip 子文件夹。这样卸载时按文件名删除不会碰到别人的同名文件。
function Test-Install($d) { (Test-Path (Join-Path $d 'installed-files.txt')) -or ($old -and ([IO.Path]::GetFullPath($old).TrimEnd('\') -eq $d)) }
function Test-Empty($d) { -not (Test-Path $d) -or -not (Get-ChildItem $d -Force -ErrorAction SilentlyContinue) }
$Dest = [IO.Path]::GetFullPath($Dest).TrimEnd('\')
if (-not ((Test-Install $Dest) -or ((Split-Path $Dest -Leaf) -eq 'OneZip' -and (Test-Empty $Dest)))) {
    $sub = Join-Path $Dest 'OneZip'
    if (-not ((Test-Install $sub) -or (Test-Empty $sub))) {
        throw "$sub 已经存在并且有别的文件，请换一个位置。"
    }
    $Dest = $sub
}
$dest = $Dest
$sameAsOld = $old -and ([IO.Path]::GetFullPath($old).TrimEnd('\') -eq $dest)

# 能不能写：不能的话请管理员把这个文件夹交给当前用户（只这一步提权）。
function Test-Writable($dir) {
    try {
        New-Item -ItemType Directory -Force $dir -ErrorAction Stop | Out-Null
        $probe = Join-Path $dir ".onezip-write-test"
        [IO.File]::WriteAllText($probe, "x")
        Remove-Item $probe -Force
        return $true
    } catch { return $false }
}
if (-not (Test-Writable $dest)) {
    Write-Host "安装位置需要管理员权限，请在弹出的窗口里确认…"
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $q = $dest.Replace("'", "''")
    $cmd = "New-Item -ItemType Directory -Force '$q' | Out-Null; icacls '$q' /grant '*${sid}:(OI)(CI)M' | Out-Null; exit `$LASTEXITCODE"
    try {
        $p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile', '-Command', $cmd
    } catch {
        throw "没有得到管理员确认，无法安装到 $dest。请换一个位置。"
    }
    if ($p.ExitCode -ne 0 -or -not (Test-Writable $dest)) { throw "无法写入 $dest，请换一个位置。" }
}

# 正在运行的 OneZip 会锁住文件（新旧两个位置都看）。
$dirs = @($dest) + @($old | Where-Object { $_ })
Get-Process OneZip, onezipc -ErrorAction SilentlyContinue | Where-Object {
    $p = $_.Path; $dirs | Where-Object { $p -like "$_\*" }
} | Stop-Process -Force
# 旧的稀疏包会被加载了菜单 DLL 的 dllhost 占着，不先关掉它，卸载旧包会一直卡在
# "DeploymentInProgress"。
Get-Process dllhost -ErrorAction SilentlyContinue | Where-Object {
    try { $_.Modules.FileName -like "*\onezip_shell.dll" } catch { $false }
} | Stop-Process -Force -ErrorAction SilentlyContinue
Get-AppxPackage -Name OneZip.ShellExtension | Remove-AppxPackage -ErrorAction SilentlyContinue

# 换了位置：删掉旧位置里装过的文件（只删清单里的），文件夹空了就删掉。
function Remove-Installed($dir) {
    $list = Join-Path $dir 'installed-files.txt'
    $names = if (Test-Path $list) { Get-Content $list -Encoding UTF8 } else {
        # 早期版本没有清单：按当时的固定文件名删
        'OneZip.exe', 'onezipc.exe', 'onezip_shell.dll', '7z.dll', 'OneZip-Shell.msix', 'OneZip-Dev.cer',
        'install.ps1', 'uninstall.ps1', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'README.md', 'README.zh-CN.md',
        '7-Zip-License.txt', 'UnRAR-License.txt', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE'
    }
    foreach ($n in $names) {
        if ($n -and $n -notmatch '[\\/:]|^\.\.?$') { Remove-Item (Join-Path $dir $n) -Force -ErrorAction SilentlyContinue }
    }
    Remove-Item $list -Force -ErrorAction SilentlyContinue
    if ((Test-Path $dir) -and -not (Get-ChildItem $dir -Force)) { Remove-Item $dir -Force -ErrorAction SilentlyContinue }
}
if ($old -and -not $sameAsOld -and (Test-Path $old)) { Remove-Installed $old }

$files = Get-ChildItem $src -File | Where-Object { $_.Extension -ne '.zip' }
Copy-Item $files.FullName $dest -Force
$files.Name + 'installed-files.txt' | Set-Content (Join-Path $dest 'installed-files.txt') -Encoding UTF8
$exe = Join-Path $dest 'OneZip.exe'

if ($isWin11) {
    $cer = Join-Path $dest 'OneZip-Dev.cer'
    if (Test-Path $cer) {
        $thumb = (New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 $cer).Thumbprint
        if (-not (Test-Path "Cert:\LocalMachine\TrustedPeople\$thumb")) {
            Write-Host "导入签名证书到“受信任的人”，需要管理员确认…"
            $cmd = "Import-Certificate -FilePath '$($cer.Replace("'", "''"))' -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null"
            $p = Start-Process powershell -Verb RunAs -Wait -PassThru -ArgumentList '-NoProfile', '-Command', $cmd
            if ($p.ExitCode -ne 0) { throw "导入证书失败或被取消" }
        }
    }
    Add-AppxPackage -Path (Join-Path $dest 'OneZip-Shell.msix') -ExternalLocation $dest
    # 第一层菜单生效后，不再写传统菜单，免得"显示更多选项"里重复出现。
    & $exe --unregister-menu | Out-Null
} else {
    & $exe --register-menu /quiet | Out-Null
}

# 开始菜单快捷方式
$lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'OneZip.lnk'
$shell = New-Object -ComObject WScript.Shell
$s = $shell.CreateShortcut($lnk)
$s.TargetPath = $exe
$s.WorkingDirectory = $dest
$s.IconLocation = "$exe,0"
$s.Save()

# 卸载项
New-Item -Path $key -Force | Out-Null
$ver = (Get-Item $exe).VersionInfo.ProductVersion
Set-ItemProperty $key DisplayName 'OneZip'
Set-ItemProperty $key DisplayIcon "$exe,0"
Set-ItemProperty $key DisplayVersion "$ver"
Set-ItemProperty $key Publisher 'OneZip'
Set-ItemProperty $key InstallLocation $dest
Set-ItemProperty $key UninstallString "powershell -NoProfile -ExecutionPolicy Bypass -File `"$dest\uninstall.ps1`""
Set-ItemProperty $key NoModify 1 -Type DWord
Set-ItemProperty $key NoRepair 1 -Type DWord

# 让资源管理器重新加载菜单
Get-Process dllhost -ErrorAction SilentlyContinue | Where-Object {
    try { $_.Modules.FileName -like "$dest\*" } catch { $false }
} | Stop-Process -Force -ErrorAction SilentlyContinue

Write-Host "OneZip 已安装到 $dest"
if ($isWin11) { Write-Host "右键菜单在第一层的 “OneZip” 里。如果没看到，重启一次资源管理器即可。" }
