<#
  在开发机上发布 OneZip：检查 → 打包 → 算 SHA-256 → 上传到 GitHub Release。

  用法（在仓库根目录）：
    .\packaging\release.ps1 -DryRun      只检查和打包，不上传（随时可以跑）
    .\packaging\release.ps1              正式发布，上传前要求输入版本号确认
    .\packaging\release.ps1 -Yes         正式发布，不再询问（确认过再用）

  版本号取自 Cargo.toml，标签为 v<版本>。上传两个文件，名字是 OneZip 检查更新时认的：
    OneZip-<版本>-Setup.exe
    OneZip-<版本>-Setup.exe.sha256   内容："<小写 SHA-256>  OneZip-<版本>-Setup.exe"
  发布说明取 docs/release-notes/v<版本>-zh-CN.md，末尾没有 SHA-256 一节就自动补上。

  正式发布前要求：
    - 工作区干净，当前在 main 分支，并且已经推送（HEAD 和 origin/main 相同），
      这样标签指向的就是公开的代码；
    - GitHub 上还没有这个版本的 Release 和标签（已发布的版本不会被覆盖）；
    - 本机证书库里有签名证书 CN=OneZip Dev。不会新建证书：换了证书，
      已经装过 OneZip 的 Windows 11 用户升级后右键菜单会失效。
#>
param(
    [switch]$DryRun,
    [switch]$Yes,
    [switch]$SkipChecks
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$repo = "liliwiki/OneZip"
$publisher = "CN=OneZip Dev"

function Step($text) { Write-Host "`n==> $text" -ForegroundColor Cyan }
function Fail($text) { Write-Host "`n发布中止：$text" -ForegroundColor Red; exit 1 }
# 外部命令的 stderr 在 Windows PowerShell 5.1 里会被当成错误，这里只看退出码。
function Run([scriptblock]$cmd) {
    $old = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { $out = & $cmd 2>&1 | ForEach-Object { "$_" } } finally { $ErrorActionPreference = $old }
    return @{ Code = $LASTEXITCODE; Out = ($out -join "`n").Trim() }
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$tag = "v$version"
$setupName = "OneZip-$version-Setup.exe"
Write-Host "OneZip $version（标签 $tag）$(if ($DryRun) { '—— 试运行，不上传' })"

# ---- 发布前检查 ----
Step "检查 Git 和 GitHub"
$problems = @()
if ((Run { git status --porcelain }).Out) { $problems += "工作区有没提交的改动" }
$branch = (Run { git rev-parse --abbrev-ref HEAD }).Out
if ($branch -ne "main") { $problems += "当前分支是 $branch，不是 main" }
$fetch = Run { git fetch origin main --tags }
if ($fetch.Code -ne 0) { $problems += "git fetch 失败：$($fetch.Out)" }
$head = (Run { git rev-parse HEAD }).Out
$remote = (Run { git rev-parse origin/main }).Out
if ($head -ne $remote) { $problems += "HEAD（$($head.Substring(0, 7))）和 origin/main 不同，先推送" }
$remoteTag = Run { git ls-remote --tags origin "refs/tags/$tag" }
if ($remoteTag.Out) { $problems += "GitHub 上已有标签 $tag" }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    $problems += "找不到 gh（GitHub CLI）"
} else {
    if ((Run { gh release view $tag -R $repo }).Code -eq 0) { $problems += "GitHub 上已有 Release $tag" }
}
# 和 build-package.ps1 一样：有好几张时用最晚到期的那张。
$cert = Get-ChildItem Cert:\CurrentUser\My |
    Where-Object { $_.Subject -eq $publisher -and $_.NotAfter -gt (Get-Date) } |
    Sort-Object NotAfter -Descending | Select-Object -First 1
if (-not $cert) { Fail "证书库里没有有效的签名证书 $publisher。请先从备份导入，不要新建" }
Write-Host "签名证书：$($cert.Thumbprint)，有效期到 $($cert.NotAfter.ToString('yyyy-MM-dd'))"
$notesFile = "docs\release-notes\$tag-zh-CN.md"
if (-not (Test-Path $notesFile)) { $problems += "没有发布说明 $notesFile" }
if ($problems) {
    $problems | ForEach-Object { Write-Host "  - $_" -ForegroundColor Yellow }
    if (-not $DryRun) { Fail "上面的问题要先解决" }
    Write-Host "（试运行，继续往下走）"
} else {
    Write-Host "没有问题"
}

# ---- 编译检查和测试 ----
if (Test-Path 'D:\Tools\Rust\cargo\bin\cargo.exe') {
    $env:RUSTUP_HOME = 'D:\Tools\Rust\rustup'
    $env:CARGO_HOME = 'D:\Tools\Rust\cargo'
    $env:Path = "D:\Tools\Rust\cargo\bin;$env:Path"
}
if ($SkipChecks) {
    if (-not $DryRun) { Fail "正式发布不能跳过检查" }
    Write-Host "`n（跳过 clippy 和测试）"
} else {
    Step "cargo clippy"
    & cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { Fail "clippy 没通过" }
    Step "cargo test"
    & cargo test --workspace
    if ($LASTEXITCODE -ne 0) { Fail "测试没通过" }
}

# ---- 打包 ----
Step "打包"
& "$PSScriptRoot\build-package.ps1" -Version $version
$setup = Join-Path $root "target\dist\$setupName"
if (-not (Test-Path $setup)) { Fail "没有生成 $setupName" }
$cer = Join-Path $root "target\dist\OneZip-$version-x64\setup\OneZip-Dev.cer"
$packed = New-Object Security.Cryptography.X509Certificates.X509Certificate2 -ArgumentList $cer
if ($packed.Thumbprint -ne $cert.Thumbprint) { Fail "安装包里的证书不是 $($cert.Thumbprint)" }

# ---- SHA-256 和发布说明 ----
Step "SHA-256"
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
$shaFile = "$setup.sha256"
[IO.File]::WriteAllText($shaFile, "$hash  $setupName`n", [Text.Encoding]::ASCII)
Write-Host "$hash  $setupName"

$notesOut = Join-Path $root "target\dist\release-notes-$tag.md"
if (Test-Path $notesFile) {
    $notes = Get-Content $notesFile -Raw -Encoding UTF8
    if ($notes -notmatch '## SHA-256') {
        $fence = '```'
        $notes = $notes.TrimEnd() + "`n`n## SHA-256`n`n${fence}text`n$($hash.ToUpper())  $setupName`n$fence`n"
    }
    [IO.File]::WriteAllText($notesOut, $notes, (New-Object Text.UTF8Encoding $false))
}

$size = "{0:N1} MB" -f ((Get-Item $setup).Length / 1MB)
Write-Host "`n准备发布：" -ForegroundColor Green
Write-Host "  Release   $tag（$repo），标签建在 $head"
Write-Host "  文件      $setupName（$size）"
Write-Host "            $setupName.sha256"
Write-Host "  发布说明  $notesOut"
if ($DryRun) {
    Write-Host "`n试运行结束，没有上传。" -ForegroundColor Green
    exit 0
}

# ---- 上传 ----
if (-not $Yes) {
    $answer = Read-Host "`n发布后所有人都能下载，已安装的 OneZip 也会提示更新。确认发布请输入版本号 $version"
    if ($answer -ne $version) { Fail "没有确认" }
}
Step "上传到 GitHub"
$create = Run { gh release create $tag $setup $shaFile -R $repo --target $head --title "OneZip $version" --notes-file $notesOut }
Write-Host $create.Out
if ($create.Code -ne 0) { Fail "gh release create 失败" }

# 核对 GitHub 上的文件
$view = Run { gh release view $tag -R $repo --json assets }
$assets = ($view.Out | ConvertFrom-Json).assets | ForEach-Object { "$($_.name) $($_.size)" }
$want = @("$setupName $((Get-Item $setup).Length)", "$setupName.sha256 $((Get-Item $shaFile).Length)")
foreach ($w in $want) {
    if ($assets -notcontains $w) { Fail "Release 上的文件和本地不一致（缺少 $w），请到网页上检查" }
}
Run { git fetch origin --tags } | Out-Null
Write-Host "`n发布完成：https://github.com/$repo/releases/tag/$tag" -ForegroundColor Green
