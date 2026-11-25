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
UI -Do click -Window OneZip -Name '选择文件夹' | Out-Null
Check '解压文件…（选文件夹）' ($fg -and (WaitFor { (T "$Dir\选的目录\多根\甲") -eq (T "$Dir\src\甲") }))

# 任务列表只渲染看得见的几行：先清掉已完成的，测试任务就在最上面。
UI -Do click -Window OneZip -Name '清除已完成' | Out-Null; Start-Sleep 1
MN -Folder $Dir -Items '测试.zip' -Command '测试压缩包'
Check '测试压缩包' (WaitFor { (UI -Do dump -Window OneZip | Select-String '测试通过') })

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

# ---- 先开对话框再点“右键菜单…”（以前会卡死）----
MN -Folder $Dir -Items '笔记.txt' -Command '添加到压缩包…'
WaitFor { (UI -Do windows | Select-String 'foreground: 添加到压缩包') } 10 | Out-Null
UI -Do click -Window OneZip -Name '右键菜单…' | Out-Null
$box = WaitFor { (UI -Do windows | Select-String 'foreground: 右键菜单') } 10
UI -Do click -Window '右键菜单' -Name '确定' | Out-Null; Start-Sleep 1
UI -Do click -Window '添加到压缩包' -Name '取消' | Out-Null; Start-Sleep 1
$alive = (Get-Process OneZip).Responding -notcontains $false
Check '对话框开着时点“右键菜单…”不卡死' ($box -and $alive)

$results | Format-Table -AutoSize | Out-String -Width 200
"通过 {0} / {1}" -f @($results | ? 结果 -eq '通过').Count, $results.Count
