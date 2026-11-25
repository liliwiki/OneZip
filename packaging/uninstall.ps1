<#
  卸载 OneZip（当前用户）。只删安装时记在 installed-files.txt 里的文件，文件夹空了才删；
  不会删除任何解压/压缩出来的文件，也不会删除未完成任务的进度。
  导入到"受信任的人"的开发证书保留，如需移除请在 certlm.msc 里手动删除 "OneZip Dev"。
#>
$ErrorActionPreference = "Continue"
$dest = $PSScriptRoot
$exe = Join-Path $dest 'OneZip.exe'

Get-Process OneZip, onezipc -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dest\*" } | Stop-Process -Force
Get-AppxPackage -Name OneZip.ShellExtension | Remove-AppxPackage
if (Test-Path $exe) { & $exe --unregister-menu | Out-Null }

Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) 'OneZip.lnk') -ErrorAction SilentlyContinue
Remove-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OneZip' -Recurse -ErrorAction SilentlyContinue

# dllhost 可能还占着 DLL，稍等再删。
Start-Sleep -Seconds 1
Get-Process dllhost -ErrorAction SilentlyContinue | Where-Object {
    try { $_.Modules.FileName -like "$dest\*" } catch { $false }
} | Stop-Process -Force -ErrorAction SilentlyContinue

$list = Join-Path $dest 'installed-files.txt'
if (Test-Path $list) {
    foreach ($n in Get-Content $list -Encoding UTF8) {
        if ($n -and $n -notmatch '[\\/:]|^\.\.?$') { Remove-Item (Join-Path $dest $n) -Force -ErrorAction SilentlyContinue }
    }
}
if (-not (Get-ChildItem $dest -Force -ErrorAction SilentlyContinue)) { Remove-Item $dest -Force -ErrorAction SilentlyContinue }
Write-Host "OneZip 已卸载。"
