<#
  卸载 OneZip（当前用户）。不会删除任何解压/压缩出来的文件，也不会删除未完成任务的进度。
  导入到"受信任的人"的开发证书保留，如需移除请在 certlm.msc 里手动删除 "OneZip Dev"。
#>
$ErrorActionPreference = "Continue"
$dest = Join-Path $env:LOCALAPPDATA 'Programs\OneZip'
$exe = Join-Path $dest 'OneZip.exe'

Get-Process OneZip -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dest\*" } | Stop-Process -Force
Get-AppxPackage -Name OneZip.ShellExtension | Remove-AppxPackage
if (Test-Path $exe) { & $exe --unregister-menu | Out-Null }

Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) 'OneZip.lnk') -ErrorAction SilentlyContinue
Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OneZip' -Recurse -ErrorAction SilentlyContinue

# dllhost 可能还占着 DLL，稍等再删。
Start-Sleep -Seconds 1
Get-Process dllhost -ErrorAction SilentlyContinue | Where-Object {
    $_.Modules.FileName -like "$dest\*"
} | Stop-Process -Force -ErrorAction SilentlyContinue
Remove-Item $dest -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "OneZip 已卸载。"
