# 生成 7z.dll 格式的测试文件（在 Windows 上运行一次，结果提交进仓库）。
#   ISO：Windows 自带的 IMAPI2 映像生成器；CAB：Windows 自带的 makecab；
#   WIM：7-Zip 的 7z.exe（第一个参数给出路径）。
# 用法：powershell -ExecutionPolicy Bypass -File make.ps1 <7z.exe>
param([string]$SevenZip)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$work = Join-Path $env:TEMP 'onezip-fixtures'
Remove-Item $work -Recurse -Force -EA SilentlyContinue
New-Item -ItemType Directory $work | Out-Null

function Noise([int]$seed, [int]$len) {
  # 可复现的半可压缩数据：随机字节里每三个夹一个字母
  $b = New-Object byte[] $len; (New-Object Random $seed).NextBytes($b)
  for ($i = 0; $i -lt $len; $i += 3) { $b[$i] = [byte](97 + $i % 26) }
  $b
}

# 样本树
$s = "$work\sample"
New-Item -ItemType Directory "$s\docs", "$s\中文目录", "$s\sub\deep", "$s\空目录" -Force | Out-Null
Set-Content "$s\docs\readme.txt" ("OneZip fixture`r`n" * 200) -NoNewline -Encoding ascii
[IO.File]::WriteAllText("$s\中文目录\你好.txt", "你好，OneZip`n" * 50, [Text.Encoding]::UTF8)
[IO.File]::WriteAllBytes("$s\empty.txt", @())
[IO.File]::WriteAllBytes("$s\sub\deep\data.bin", (Noise 1 100000))
[IO.File]::WriteAllBytes("$s\big file.bin", (Noise 2 400000))
# 很多小文件（超过一批 512 个）
$m = "$work\many"
for ($i = 0; $i -lt 700; $i++) { $d = "$m\d$($i % 7)"; New-Item -ItemType Directory $d -Force | Out-Null; [IO.File]::WriteAllBytes("$d\f$i.bin", (Noise (100 + $i) (300 + ($i % 5) * 300))) }

# ---- ISO ----
Add-Type -TypeDefinition @"
using System; using System.IO; using System.Runtime.InteropServices; using System.Runtime.InteropServices.ComTypes;
public static class IsoWriter {
  public static void Save(object stream, string path) {
    IStream s = (IStream)stream; var buf = new byte[1 << 20]; IntPtr pRead = Marshal.AllocHGlobal(4);
    using (var f = File.Create(path)) { while (true) { s.Read(buf, buf.Length, pRead); int n = Marshal.ReadInt32(pRead); if (n == 0) break; f.Write(buf, 0, n); } }
    Marshal.FreeHGlobal(pRead);
  }
}
"@
function Iso($src, $fs, $out) {
  $img = New-Object -ComObject IMAPI2FS.MsftFileSystemImage
  $img.FileSystemsToCreate = $fs   # 1 ISO9660, 2 Joliet, 4 UDF
  $img.VolumeName = 'ONEZIP'
  $img.Root.AddTree($src, $false)
  [IsoWriter]::Save($img.CreateResultImage().ImageStream, $out)
}
New-Item -ItemType Directory "$here\iso" -Force | Out-Null
Iso $s 3 "$here\iso\joliet.iso"
Iso $s 4 "$here\iso\udf.iso"
Iso $m 7 "$here\iso\many.iso"

# ---- CAB ----
New-Item -ItemType Directory "$here\cab" -Force | Out-Null
function Cab($src, $type, $out, $threshold) {
  $ddf = "$work\cab.ddf"
  $lines = @(".OPTION EXPLICIT", ".Set CabinetNameTemplate=$(Split-Path $out -Leaf)", ".Set DiskDirectoryTemplate=$(Split-Path $out)",
    ".Set CompressionType=$type", ".Set Cabinet=on", ".Set Compress=on", ".Set MaxDiskSize=0", ".Set FolderSizeThreshold=$threshold", ".Set UniqueFiles=off")
  foreach ($f in Get-ChildItem $src -Recurse -File) {
    $rel = $f.FullName.Substring($src.Length + 1)
    $lines += "`"$($f.FullName)`" `"$rel`""
  }
  $lines | Set-Content $ddf -Encoding Default
  & makecab /F $ddf /L $work | Out-Null
  Remove-Item "$work\setup.inf", "$work\setup.rpt" -EA SilentlyContinue
}
Cab $s 'MSZIP' "$here\cab\mszip.cab" 200000
Cab $s 'LZX' "$here\cab\lzx.cab" 200000
Cab $m 'MSZIP' "$here\cab\many.cab" 300000

# ---- WIM ----
New-Item -ItemType Directory "$here\wim" -Force | Out-Null
Push-Location $s; & $SevenZip a -twim "$here\wim\sample.wim" * | Out-Null; Pop-Location

Get-ChildItem $here -Recurse -File | ? Extension -ne '.ps1' | % { "{0,10:n0}  {1}" -f $_.Length, $_.FullName.Substring($here.Length + 1) }
Remove-Item $work -Recurse -Force
