<#
  安装 OneZip（只对当前用户，不改系统目录）。在解压出的发布目录里运行：
    powershell -ExecutionPolicy Bypass -File install.ps1

  - 程序复制到 %LOCALAPPDATA%\Programs\OneZip
  - Windows 11：注册稀疏包，右键菜单出现在第一层。
    如果包用的是自签名证书，需要把 OneZip-Dev.cer 导入本机"受信任的人"，
    这一步会弹出一次管理员确认（UAC）。
  - Windows 10：写入按用户的传统右键菜单（本来就直接显示）。
  - 开始菜单快捷方式、"应用和功能"里的卸载项。
#>
$ErrorActionPreference = "Stop"
$src = $PSScriptRoot
$dest = Join-Path $env:LOCALAPPDATA 'Programs\OneZip'
$isWin11 = [Environment]::OSVersion.Version.Build -ge 22000

# 正在运行的 OneZip 会锁住文件。
Get-Process OneZip -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dest\*" } | Stop-Process -Force
# 旧的稀疏包会被加载了菜单 DLL 的 dllhost 占着，不先关掉它，卸载旧包会一直卡在
# "DeploymentInProgress"。
Get-Process dllhost -ErrorAction SilentlyContinue | Where-Object {
    try { $_.Modules.FileName -like "*\onezip_shell.dll" } catch { $false }
} | Stop-Process -Force -ErrorAction SilentlyContinue
Get-AppxPackage -Name OneZip.ShellExtension | Remove-AppxPackage -ErrorAction SilentlyContinue

New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item "$src\*" $dest -Recurse -Force -Exclude '*.zip'
$exe = Join-Path $dest 'OneZip.exe'

if ($isWin11) {
    $cer = Join-Path $dest 'OneZip-Dev.cer'
    if (Test-Path $cer) {
        $thumb = (New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 $cer).Thumbprint
        if (-not (Test-Path "Cert:\LocalMachine\TrustedPeople\$thumb")) {
            Write-Host "导入签名证书到“受信任的人”，需要管理员确认…"
            $cmd = "Import-Certificate -FilePath '$cer' -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null"
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
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OneZip'
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
    $_.Modules.FileName -like "$dest\*"
} | Stop-Process -Force -ErrorAction SilentlyContinue

Write-Host "OneZip 已安装到 $dest"
if ($isWin11) { Write-Host "右键菜单在第一层的 “OneZip” 里。如果没看到，重启一次资源管理器即可。" }
