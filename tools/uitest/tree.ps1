# 输出一个目录树的摘要（相对路径 + MD5），用来比较解压结果是否与源文件一致。
param([string]$Path)
if (-not (Test-Path $Path)) { "MISSING"; exit }
$p = (Resolve-Path $Path).Path
$lines = Get-ChildItem $p -Recurse -File | Sort-Object FullName | ForEach-Object {
  (Get-FileHash $_.FullName -Algorithm MD5).Hash + ' ' + $_.FullName.Substring($p.Length)
}
$all = ($lines -join "`n")
$md5 = [System.Security.Cryptography.MD5]::Create()
"{0} files, digest {1}" -f @($lines).Count, ([BitConverter]::ToString($md5.ComputeHash([Text.Encoding]::UTF8.GetBytes($all))) -replace '-','').Substring(0,12)
