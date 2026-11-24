# 在 Windows 上运行 cargo，输出写到 target\logs\<名字>.log，最后一行是退出码。
# 用法：powershell -File tools\cargo.ps1 <日志名> "<cargo 参数>"
param([string]$Name, [string]$CargoLine)
$CargoArgs = $CargoLine -split '\s+' | Where-Object { $_ }
$env:RUSTUP_HOME = 'D:\Tools\Rust\rustup'
$env:CARGO_HOME = 'D:\Tools\Rust\cargo'
$env:Path = "D:\Tools\Rust\cargo\bin;$env:Path"
$env:CARGO_TERM_COLOR = 'never'
$root = Split-Path $PSScriptRoot -Parent
$logs = Join-Path $root 'target\logs'
New-Item -ItemType Directory -Force $logs | Out-Null
$log = Join-Path $logs "$Name.log"
Set-Location $root
$ErrorActionPreference = "Continue"
& cargo @CargoArgs 2>&1 | ForEach-Object { "$_" } | Set-Content -Encoding utf8 $log
"exit=$LASTEXITCODE" | Add-Content -Encoding utf8 $log
