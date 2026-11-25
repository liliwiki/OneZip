# 在资源管理器里选中文件，打开右键菜单 → OneZip → 点某一项（UI 自动化）。
# 用法：menu.ps1 -Folder D:\T -Items a.zip[,b.zip] [-Command '智能解压'] [-Out shot.png]
#   不给 -Command 时只展开 OneZip 子菜单并列出所有菜单项。
param([string]$Folder, [string[]]$Items, [string]$Command = '', [string]$Out = '', [string]$Sub = 'OneZip')
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class M {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
  public static void Front(IntPtr h) {
    ShowWindow(h, 9);
    uint fg = GetWindowThreadProcessId(GetForegroundWindow(), IntPtr.Zero), me = GetCurrentThreadId();
    AttachThreadInput(me, fg, true); BringWindowToTop(h); SetForegroundWindow(h); AttachThreadInput(me, fg, false);
  }
}
"@
[M]::SetProcessDPIAware() | Out-Null
# powershell -File 传不了数组：允许用 | 分隔多个文件名。
$Items = @($Items | % { $_ -split '\|' })
$A = [System.Windows.Automation.AutomationElement]
$root = $A::RootElement
function Find($name) {
  $c = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $name
  $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c)
}
function Click($e) {
  $r = $e.Current.BoundingRectangle
  [M]::SetCursorPos([int]($r.X + $r.Width/2), [int]($r.Y + $r.Height/2)) | Out-Null
  Start-Sleep -Milliseconds 250
  [M]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [M]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
function FindItem($name) {
  $c1 = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $name
  $c2 = New-Object System.Windows.Automation.PropertyCondition ($A::ControlTypeProperty), ([System.Windows.Automation.ControlType]::MenuItem)
  $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.AndCondition $c1, $c2))
}
function MenuItems {
  $mi = New-Object System.Windows.Automation.PropertyCondition ($A::ControlTypeProperty), ([System.Windows.Automation.ControlType]::MenuItem)
  $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $mi) | % { $_.Current.Name }
}

$sh = New-Object -ComObject Shell.Application
$want = (Resolve-Path $Folder).Path.TrimEnd('\')
$win = $null
foreach ($w in $sh.Windows()) { try { if ($w.Document.Folder.Self.Path -eq $want) { $win = $w } } catch {} }
if (-not $win) {
  Start-Process explorer.exe $want; Start-Sleep 3
  foreach ($w in $sh.Windows()) { try { if ($w.Document.Folder.Self.Path -eq $want) { $win = $w } } catch {} }
}
if (-not $win) { "no explorer window for $want"; exit 2 }
$win.Refresh(); Start-Sleep 1
[System.Windows.Forms.SendKeys]::SendWait('{ESC}')
[M]::Front([IntPtr]$win.HWND); Start-Sleep -Milliseconds 600
for ($try = 0; $try -lt 5; $try++) {
  $first = $true
  foreach ($name in $Items) {
    $it = $win.Document.Folder.ParseName($name)
    if (-not $it) { "item $name not found"; exit 3 }
    if ($first) { $win.Document.SelectItem($it, 1 + 4 + 8 + 16); $first = $false } else { $win.Document.SelectItem($it, 1) }
  }
  Start-Sleep -Milliseconds 600
  if ($win.Document.SelectedItems().Count -eq $Items.Count) { break }
  [M]::Front([IntPtr]$win.HWND); Start-Sleep -Milliseconds 500
}
# 用鼠标右键点选中的最后一项（键盘 Shift+F10 在预览窗格抢走焦点时会失效）。
$ew = $A::FromHandle([IntPtr]$win.HWND)
$c1 = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $Items[-1]
$c2 = New-Object System.Windows.Automation.PropertyCondition ($A::ControlTypeProperty), ([System.Windows.Automation.ControlType]::ListItem)
$li = $ew.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.AndCondition $c1, $c2))
if ($li) {
  $r = $li.Current.BoundingRectangle
  [M]::SetCursorPos([int]($r.X + 60), [int]($r.Y + $r.Height/2)) | Out-Null
  Start-Sleep -Milliseconds 250
  [M]::mouse_event(8,0,0,0,[UIntPtr]::Zero); [M]::mouse_event(16,0,0,0,[UIntPtr]::Zero)
} else {
  [System.Windows.Forms.SendKeys]::SendWait('+{F10}')
}
Start-Sleep -Seconds 2
$oz = FindItem $Sub
if (-not $oz) {
  "$Sub not in menu"; MenuItems
  if ($Out) {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save($Out); "saved $Out"
  }
  exit 4
}
Click $oz; Start-Sleep -Seconds 2
$all = @(MenuItems)
if (-not $Command) {
  "menu:"; $all | % { "  $_" }
  if ($Out) {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save($Out); "saved $Out"
  }
  [System.Windows.Forms.SendKeys]::SendWait('{ESC}{ESC}')
  exit 0
}
$e = $null
if ($Command.Contains('*')) {
  $mi = New-Object System.Windows.Automation.PropertyCondition ($A::ControlTypeProperty), ([System.Windows.Automation.ControlType]::MenuItem)
  $e = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $mi) | ? { $_.Current.Name -like $Command } | Select -First 1
  if ($e) { $Command = $e.Current.Name }
} else { $e = FindItem $Command }
if (-not $e) { "command '$Command' not found; menu:"; $all | % { "  $_" }; [System.Windows.Forms.SendKeys]::SendWait('{ESC}{ESC}'); exit 5 }
Click $e
"invoked '$Command'"
