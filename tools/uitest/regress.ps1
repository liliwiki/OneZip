# 右键菜单回归测试：在 D:\OZT 造测试文件，逐项点 OneZip 子菜单，检查结果。
# 需要已安装的 OneZip（%LOCALAPPDATA%\Programs\OneZip），会操作鼠标键盘，运行时别动电脑。
# 用法：powershell -ExecutionPolicy Bypass -File regress.ps1
param([string]$Dir = 'D:\OZT')
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$here = $PSScriptRoot
$cli = "$env:LOCALAPPDATA\Programs\OneZip\onezipc.exe"
function MN { & powershell -NoProfile -ExecutionPolicy Bypass -File "$here\menu.ps1" @args | Out-Null }
function UI { & powershell -NoProfile -ExecutionPolicy Bypass -File "$here\ui.ps1" @args }
function T($p) { & powershell -NoProfile -ExecutionPolicy Bypass -File "$here\tree.ps1" -Path $p }
function WaitFor($cond, $sec = 20) { for ($i = 0; $i -lt $sec * 4; $i++) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 250 }; $false }
$results = @()
function Check($name, $ok, $detail = '') { $script:results += [pscustomobject]@{ 项 = $name; 结果 = $(if ($ok) { '通过' } else { '失败' }); 说明 = $detail } }

# ---- 准备 ----
Get-Process OneZip -EA SilentlyContinue | Stop-Process -Force
Remove-Item $Dir -Recurse -Force -EA SilentlyContinue
New-Item -ItemType Directory "$Dir\src\资料\子目录", "$Dir\src\甲", "$Dir\src\乙" -Force | Out-Null
1..4 | % { Set-Content "$Dir\src\资料\f$_.txt" ("资料 $_ " * 30000) }
Set-Content "$Dir\src\资料\子目录\深.txt" ("deep" * 5000)
Set-Content "$Dir\src\甲\a.txt" ("甲" * 9000); Set-Content "$Dir\src\乙\b.txt" ("乙" * 9000)
& $cli a "$Dir\资料.zip" "$Dir\src\资料" -t zip | Out-Null
& $cli a "$Dir\多根.7z" "$Dir\src\甲" "$Dir\src\乙" | Out-Null
& $cli a "$Dir\加密.7z" "$Dir\src\资料" -p 1234 | Out-Null
foreach ($n in '到这', '删包', '带 空格 (1)', '测试') { Copy-Item "$Dir\资料.zip" "$Dir\$n.zip" }
Set-Content "$Dir\笔记.txt" ("note " * 2000)
Copy-Item "$Dir\src\资料" "$Dir\照片" -Recurse
$ref = T "$Dir\src\资料"

# ---- 菜单内容 ----
$m = & powershell -NoProfile -ExecutionPolicy Bypass -File "$here\menu.ps1" -Folder $Dir -Items '资料.zip'
Check '压缩包菜单 9 项' (($m | Select-String '^\s\s').Count -ge 9 -and ($m -match '解压到 "资料\\"')) (($m | Select -Skip 1 -First 9) -join ' / ')
$m = & powershell -NoProfile -ExecutionPolicy Bypass -File "$here\menu.ps1" -Folder $Dir -Items '笔记.txt'
Check '普通文件只有压缩 3 项' (-not ($m -match '智能解压') -and ($m -match '添加到 "笔记.7z"')) (($m | Select -Skip 1 -First 3) -join ' / ')
$m = & powershell -NoProfile -ExecutionPolicy Bypass -File "$here\menu.ps1" -Folder $Dir -Items '资料.zip|笔记.txt'
Check '混选只有压缩项' (-not ($m -match '智能解压') -and ($m -match '添加到 "OZT.7z"'))

# ---- 逐项执行 ----
MN -Folder $Dir -Items '资料.zip' -Command '智能解压'
Check '智能解压' ((WaitFor { (T "$Dir\资料") -eq $ref }) -and (Test-Path "$Dir\资料.zip"))
Remove-Item "$Dir\资料" -Recurse -Force

MN -Folder $Dir -Items '多根.7z' -Command '解压到*多根*'
Check '解压到 "多根\"' ((WaitFor { (T "$Dir\多根\甲") -eq (T "$Dir\src\甲") }) -and (T "$Dir\多根\乙") -eq (T "$Dir\src\乙"))

MN -Folder $Dir -Items '到这.zip' -Command '解压到当前位置'
Check '解压到当前位置' (WaitFor { (T "$Dir\资料") -eq $ref })
Remove-Item "$Dir\资料" -Recurse -Force

MN -Folder $Dir -Items '删包.zip' -Command '解压后删除压缩包*'
Check '解压后删除压缩包' ((WaitFor { (T "$Dir\资料") -eq $ref -and -not (Test-Path "$Dir\删包.zip") }))
Remove-Item "$Dir\资料" -Recurse -Force

MN -Folder $Dir -Items '带 空格 (1).zip' -Command '解压到*带 空格*'
Check '带空格的文件名' (WaitFor { (T "$Dir\带 空格 (1)\资料") -eq $ref })

New-Item -ItemType Directory "$Dir\选的目录" -Force | Out-Null
MN -Folder $Dir -Items '多根.7z' -Command '解压文件…'
$fg = WaitFor { (UI -Do windows | Select-String 'foreground: 解压到哪个文件夹') } 10
# 粘贴路径（避开输入法）+ 回车进入该文件夹，再点“选择文件夹”。
Set-Clipboard "$Dir\选的目录"; UI -Do type -Text '^v' | Out-Null; Start-Sleep 1
UI -Do type -Text '{ENTER}' | Out-Null; Start-Sleep 2
UI -Do click -Window '解压到哪个文件夹' -Name '选择文件夹' | Out-Null
Check '解压文件…（选文件夹）' ($fg -and (WaitFor { (T "$Dir\选的目录\多根\甲") -eq (T "$Dir\src\甲") }))

# 测试结果留在它的进度窗口里，点“关闭”关掉。
MN -Folder $Dir -Items '测试.zip' -Command '测试压缩包'
Check '测试压缩包（结果留在进度窗口）' (WaitFor { (UI -Do dump -Window '测试 测试.zip' | Select-String '测试通过') })
UI -Do click -Window '测试 测试.zip' -Name '关闭' | Out-Null; Start-Sleep 1

MN -Folder $Dir -Items '照片' -Command '添加到*照片.7z*'
Check '添加到 "照片.7z"' (WaitFor { (Test-Path "$Dir\照片.7z") -and ((& $cli t "$Dir\照片.7z") -match '通过') })

MN -Folder $Dir -Items '笔记.txt' -Command '添加到*笔记.zip*'
Check '添加到 "笔记.zip"' (WaitFor { (Test-Path "$Dir\笔记.zip") -and ((& $cli t "$Dir\笔记.zip") -match '通过') })

MN -Folder $Dir -Items '笔记.txt' -Command '添加到压缩包…'
$fg = WaitFor { (UI -Do windows | Select-String 'foreground: 添加到压缩包') } 10
UI -Do clicktype -Window '添加到压缩包' -Type Edit -Name 1 | Out-Null; Start-Sleep 1
UI -Do type -Text '246810' | Out-Null; Start-Sleep 1
UI -Do click -Window '添加到压缩包' -Name '开始压缩' | Out-Null
Check '添加到压缩包…（对话框在最前，加密 7z）' ($fg -and (WaitFor { (Test-Path "$Dir\笔记.7z") -and ((& $cli t "$Dir\笔记.7z" -p 246810) -match '通过') }))

# ---- 校验值（平铺在 OneZip 子菜单里）----
# 结果窗口里的文字通过“复制”按钮取出来核对。
$m = & powershell -NoProfile -ExecutionPolicy Bypass -File "$here\menu.ps1" -Folder $Dir -Items '笔记.txt'
Check 'OneZip 子菜单里有校验值各项' (($m -match '^\s+CRC-32$') -and ($m -match '^\s+SHA-512$') -and ($m -match '校验值（全部算法）'))
Set-Clipboard 'x'
MN -Folder $Dir -Items '笔记.txt' -Command 'SHA-256'
$want = (Get-FileHash "$Dir\笔记.txt" -Algorithm SHA256).Hash
$ok = WaitFor { (UI -Do windows | Select-String 'foreground: 校验值') } 15
UI -Do click -Window '校验值' -Name '复制' | Out-Null; Start-Sleep 1
Check 'SHA-256（结果窗口在最前、可复制）' ($ok -and ((Get-Clipboard -Raw) -match $want)) $want
UI -Do click -Window '校验值' -Name '关闭' | Out-Null; Start-Sleep 1

Set-Clipboard 'x'
MN -Folder $Dir -Items '照片' -Command '校验值（全部算法）'
$ok = WaitFor { (UI -Do windows | Select-String 'foreground: 校验值') } 15
UI -Do click -Window '校验值' -Name '复制' | Out-Null; Start-Sleep 1
$txt = Get-Clipboard -Raw
$md5 = (Get-FileHash "$Dir\照片\f1.txt" -Algorithm MD5).Hash
Check '校验值全部算法（文件夹递归）' ($ok -and $txt -match $md5 -and $txt -match 'SHA-512' -and $txt -match 'CRC-64' -and $txt -match '共 5 个文件')
UI -Do click -Window '校验值' -Name '关闭' | Out-Null; Start-Sleep 1

# ---- 加密包：密码框自己弹出来（不需要主窗口）----
MN -Folder $Dir -Items '加密.7z' -Command '智能解压'
$pw = WaitFor { (UI -Do windows | Select-String 'foreground: 需要密码') } 15
UI -Do clicktype -Window '需要密码' -Type Edit -Name 0 | Out-Null; Start-Sleep 1
UI -Do type -Text '1234' | Out-Null; Start-Sleep 1
UI -Do click -Window '需要密码' -Name '确定' | Out-Null
Check '加密包：密码框在最前，输对后解出' ($pw -and (WaitFor { (T "$Dir\资料") -eq $ref }))
WaitFor { -not (Get-Process OneZip -EA SilentlyContinue) } 20 | Out-Null
Remove-Item "$Dir\资料" -Recurse -Force -EA SilentlyContinue

# ---- 先开对话框再点“右键菜单…”（以前会卡死）----
# 从右键菜单来的操作不再弹主窗口，这里先单独打开主窗口。
Start-Process "$env:LOCALAPPDATA\Programs\OneZip\OneZip.exe"
WaitFor { (UI -Do windows | Select-String '^OneZip \|') } 10 | Out-Null
MN -Folder $Dir -Items '笔记.txt' -Command '添加到压缩包…'
WaitFor { (UI -Do windows | Select-String 'foreground: 添加到压缩包') } 10 | Out-Null
UI -Do click -Window OneZip -Name '右键菜单…' | Out-Null
$box = WaitFor { (UI -Do windows | Select-String 'foreground: 右键菜单') } 10
UI -Do click -Window '右键菜单' -Name '确定' | Out-Null; Start-Sleep 1
UI -Do click -Window '添加到压缩包' -Name '取消' | Out-Null; Start-Sleep 1
$alive = (Get-Process OneZip).Responding -notcontains $false
Check '对话框开着时点“右键菜单…”不卡死' ($box -and $alive)
UI -Do click -Window OneZip -Name '清除已完成' | Out-Null
(Get-Process OneZip -EA SilentlyContinue).CloseMainWindow() | Out-Null
WaitFor { -not (Get-Process OneZip -EA SilentlyContinue) } 10 | Out-Null

# ---- 右键菜单的操作只弹小进度窗口，不弹主窗口；做完自动关、程序退出 ----
MN -Folder $Dir -Items '资料.zip' -Command '解压到当前位置'
Start-Sleep 1
$main = [bool](UI -Do windows | Select-String '^OneZip \|')
$gone = WaitFor { -not (Get-Process OneZip -EA SilentlyContinue) } 20
Check '菜单操作不弹主窗口，做完自动关闭并退出' (-not $main -and $gone)
Remove-Item "$Dir\资料" -Recurse -Force -EA SilentlyContinue

# ---- 7z 固实压缩：进度要逐步走，压缩中关窗口不能卡死、不能留下临时文件 ----
$big = "$Dir\大块"; New-Item -ItemType Directory $big -Force | Out-Null
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
$text = [Text.Encoding]::UTF8.GetBytes(("OneZip 固实压缩进度测试 " * 4000))
foreach ($i in 0..3) {
  $fs = [IO.File]::Create("$big\part$i.bin")
  $rb = New-Object byte[] (1MB)
  foreach ($j in 0..39) { $rng.GetBytes($rb); $fs.Write($rb, 0, $rb.Length); $fs.Write($text, 0, $text.Length) }
  $fs.Close()
}
MN -Folder $Dir -Items '大块' -Command '添加到*大块.7z*'
# 进度窗口标题是 “NN% 压缩为 大块.7z”。
$pcts = @()
foreach ($k in 1..8) {
  Start-Sleep -Milliseconds 700
  $p = (UI -Do windows | Select-String '^(\d+)% 压缩为 大块\.7z' | Select -First 1)
  if ($p) { $pcts += [int]$p.Matches[0].Groups[1].Value }
}
$distinct = @($pcts | Sort-Object -Unique).Count
Check '7z 进度逐步增加（不是一下 100%）' ($distinct -ge 3 -and ($pcts | Select -First 1) -lt 100) ($pcts -join ',')
WaitFor { -not (Get-Process OneZip -EA SilentlyContinue) } 60 | Out-Null
# 再压一次，压到一半点进度窗口的 ×：取消压缩、删掉临时文件、程序退出
Remove-Item "$Dir\大块.7z" -EA SilentlyContinue
MN -Folder $Dir -Items '大块' -Command '添加到*大块.7z*'
Start-Sleep 2
$proc = Get-Process OneZip
$t0 = Get-Date
$proc.CloseMainWindow() | Out-Null
$exited = WaitFor { $proc.HasExited } 15
$secs = ((Get-Date) - $t0).TotalSeconds
$left = @(Get-ChildItem $Dir -Filter '大块.7z*')
Check '压缩中关进度窗口：几秒内退出，不留临时文件' ($exited -and $left.Count -eq 0) ("{0:n1}s, 残留 {1}" -f $secs, ($left.Name -join ','))

$results | Format-Table -AutoSize | Out-String -Width 200
"通过 {0} / {1}" -f @($results | ? 结果 -eq '通过').Count, $results.Count
