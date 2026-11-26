# UI 自动化测试小工具：按名字点控件、列出窗口内容、截屏。
# 用法：ui.ps1 -Do click -Name '右键菜单…' [-Window OneZip]
#       ui.ps1 -Do dump [-Window OneZip]
#       ui.ps1 -Do shot -Out x.png
param([string]$Do = 'dump', [string]$Name = '', [string]$Window = '', [string]$Out = '', [int]$Wait = 1, [string]$Text = '', [string]$Type = '')
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class U {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [StructLayout(LayoutKind.Sequential)] public struct PT { public int X; public int Y; }
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(PT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll", EntryPoint="GetWindowThreadProcessId")] public static extern uint PidOf(IntPtr h, out int p);
  public static string ProcAt(int x, int y) {
    var pt = new PT(); pt.X = x; pt.Y = y;
    int pid; PidOf(GetAncestor(WindowFromPoint(pt), 2), out pid);
    try { return System.Diagnostics.Process.GetProcessById(pid).ProcessName; } catch { return ""; }
  }
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,int x,int y,uint d,UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out int p);
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
  if (-not $scope) {
    # 归属于别的窗口的对话框挂在主窗口下面
    $cw = New-Object System.Windows.Automation.PropertyCondition ($A::ControlTypeProperty), ([System.Windows.Automation.ControlType]::Window)
    $scope = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.AndCondition $c, $cw))
  }
  if (-not $scope) { "window '$Window' not found"; exit 2 }
}
switch ($Do) {
  'click' {
    $e = Find $scope $Name
    if (-not $e) { "'$Name' not found"; exit 3 }
    $r = $e.Current.BoundingRectangle
    if ([U]::ProcAt([int]($r.X + $r.Width/2), [int]($r.Y + $r.Height/2)) -eq 'Cage') { "refuse click: Cage is under the target"; exit 7 }; [U]::SetCursorPos([int]($r.X + $r.Width/2), [int]($r.Y + $r.Height/2)) | Out-Null
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
      % { "{0} | pid {1} | {2} | hung={3}" -f $_.Current.Name, $_.Current.ProcessId, $_.Current.BoundingRectangle, [U]::IsHungAppWindow([IntPtr][int]$_.Current.NativeWindowHandle) }
    $sb = New-Object System.Text.StringBuilder 256; [U]::GetWindowText([U]::GetForegroundWindow(), $sb, 256) | Out-Null; "foreground: " + $sb
  }
  'front' {
    $h = [IntPtr]$scope.Current.NativeWindowHandle
    [U]::keybd_event(0x12,0,0,[UIntPtr]::Zero); [U]::keybd_event(0x12,0,2,[UIntPtr]::Zero)
    [U]::ShowWindow($h, 9) | Out-Null; [U]::SetForegroundWindow($h) | Out-Null
    "front $Window"
  }
  'max' {
    $h = [IntPtr]$scope.Current.NativeWindowHandle
    [U]::ShowWindow($h, 3) | Out-Null; [U]::SetForegroundWindow($h) | Out-Null
    "max $Window"
  }
  'restore' {
    $h = [IntPtr]$scope.Current.NativeWindowHandle
    [U]::ShowWindow($h, 9) | Out-Null
    "restore $Window"
  }
  'clicktype' {
    # 点窗口里第 N 个（从 0 开始，-Name 给序号）某类控件，例如 Edit
    $ct = [System.Windows.Automation.ControlType]::$Type
    $cc = New-Object System.Windows.Automation.PropertyCondition ($A::ControlTypeProperty), $ct
    $all = $scope.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cc)
    $e = $all[[int]$Name]
    if (-not $e) { "no $Type #$Name (found $($all.Count))"; exit 3 }
    $r = $e.Current.BoundingRectangle
    if ([U]::ProcAt([int]($r.X + $r.Width/2), [int]($r.Y + $r.Height/2)) -eq 'Cage') { "refuse click: Cage is under the target"; exit 7 }; [U]::SetCursorPos([int]($r.X + $r.Width/2), [int]($r.Y + $r.Height/2)) | Out-Null
    Start-Sleep -Milliseconds 200
    [U]::mouse_event(2,0,0,0,[UIntPtr]::Zero); [U]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    "clicked $Type #$Name at $r"
  }
  'type' {
    # 最前面是 Claude 应用（Cage）时不按：在那里按 Esc 会停掉当前会话。
    $procId = 0; [void][U]::GetWindowThreadProcessId([U]::GetForegroundWindow(), [ref]$procId)
    $fp = try { (Get-Process -Id $procId).ProcessName } catch { '' }
    if ($fp -eq 'Cage') { "skip typing (Cage is foreground)"; exit 4 }
    [System.Windows.Forms.SendKeys]::SendWait($Text); "typed"
  }
  'setvalue' {
    # 给第 N 个（-Name 序号）Edit 直接赋值，不经过键盘（避开输入法和自动补全）
    $cc = New-Object System.Windows.Automation.PropertyCondition ($A::ControlTypeProperty), ([System.Windows.Automation.ControlType]::Edit)
    if ($Name -match '^\d+$') { $e = $scope.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cc)[[int]$Name] }
    else {
      # 按名字找（可能是组合框），再取它里面的编辑框
      $cn = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $Name
      $e = $scope.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cn)
      if ($e -and $e.Current.ControlType -ne [System.Windows.Automation.ControlType]::Edit) {
        $inner = $e.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cc)
        if ($inner) { $e = $inner }
      }
    }
    if (-not $e) { "no edit #$Name"; exit 3 }
    $e.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($Text)
    "set edit #$Name"
  }
  'invoke' {
    $c1 = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $Name
    $c2 = New-Object System.Windows.Automation.PropertyCondition ($A::ControlTypeProperty), ([System.Windows.Automation.ControlType]::Button)
    $e = $scope.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.AndCondition $c1, $c2))
    if (-not $e) { "button '$Name' not found"; exit 3 }
    $e.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    "invoked '$Name'"
  }
  'wheel' {
    # 在窗口中部向下滚动
    $r = $scope.Current.BoundingRectangle
    if ([U]::ProcAt([int]($r.X + $r.Width/2), [int]($r.Y + $r.Height*0.7)) -eq 'Cage') { "refuse click: Cage is under the target"; exit 7 }; [U]::SetCursorPos([int]($r.X + $r.Width/2), [int]($r.Y + $r.Height*0.7)) | Out-Null
    Start-Sleep -Milliseconds 200
    1..10 | % { [U]::mouse_event(0x800,0,0,[uint32]4294967176,[UIntPtr]::Zero); Start-Sleep -Milliseconds 50 }
    "wheel"
  }
  'close' {
    # 像点标题栏的 × 一样关窗口（不动鼠标、不按键）
    if (-not $Window) { "need -Window"; exit 2 }
    $scope.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern).Close()
    "closed '$Window'"
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
