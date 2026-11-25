# UI 自动化测试小工具：按名字点控件、列出窗口内容、截屏。
# 用法：ui.ps1 -Do click -Name '右键菜单…' [-Window OneZip]
#       ui.ps1 -Do dump [-Window OneZip]
#       ui.ps1 -Do shot -Out x.png
param([string]$Do = 'dump', [string]$Name = '', [string]$Window = '', [string]$Out = '', [int]$Wait = 1)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class U {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern bool IsHungAppWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
}
"@
[U]::SetProcessDPIAware() | Out-Null
$A = [System.Windows.Automation.AutomationElement]
$root = $A::RootElement
function Find($scope, $name) {
  $c = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $name
  $scope.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $c)
}
$scope = $root
if ($Window) {
  $c = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $Window
  $scope = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $c)
  if (-not $scope) { "window '$Window' not found"; exit 2 }
}
switch ($Do) {
  'click' {
    $e = Find $scope $Name
    if (-not $e) { "'$Name' not found"; exit 3 }
    $r = $e.Current.BoundingRectangle
    [U]::SetCursorPos([int]($r.X + $r.Width/2), [int]($r.Y + $r.Height/2)) | Out-Null
    Start-Sleep -Milliseconds 200
    [U]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [U]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    "clicked '$Name' at $r"
  }
  'dump' {
    $scope.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
      ? { $_.Current.Name } | % { "{0}: {1}" -f $_.Current.ControlType.ProgrammaticName.Replace('ControlType.',''), $_.Current.Name }
  }
  'windows' {
    $root.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition) |
      % { "{0} | pid {1} | {2} | hung={3}" -f $_.Current.Name, $_.Current.ProcessId, $_.Current.BoundingRectangle, [U]::IsHungAppWindow([IntPtr]$_.Current.NativeWindowHandle) }
    $sb = New-Object System.Text.StringBuilder 256; [U]::GetWindowText([U]::GetForegroundWindow(), $sb, 256) | Out-Null; "foreground: " + $sb
  }
  'front' {
    $h = [IntPtr]$scope.Current.NativeWindowHandle
    [U]::keybd_event(0x12,0,0,[UIntPtr]::Zero); [U]::keybd_event(0x12,0,2,[UIntPtr]::Zero)
    [U]::ShowWindow($h, 9) | Out-Null; [U]::SetForegroundWindow($h) | Out-Null
    "front $Window"
  }
  'hung' {
    Get-Process OneZip -ErrorAction SilentlyContinue | % { "pid {0} responding={1} title='{2}'" -f $_.Id, $_.Responding, $_.MainWindowTitle }
  }
  'shot' {}
}
if ($Out) {
  Start-Sleep -Seconds $Wait
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
  $bmp.Save($Out); "saved $Out"
}
