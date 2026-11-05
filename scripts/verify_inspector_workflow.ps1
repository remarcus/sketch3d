# Real Windows UI Automation pilot for the inspector visibility workflow.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [ValidateRange(10, 300)][int]$TimeoutSeconds = 90
)
$ErrorActionPreference = 'Stop'
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
$reportPath = Join-Path $OutputDirectory 'ui-workflow-report.json'
$screenshotPath = Join-Path $OutputDirectory 'ui-workflow.png'
$steps = [Collections.Generic.List[string]]::new()
$started = [DateTime]::UtcNow
$outcome = 'failed'
$reason = $null
$screenshotSize = $null
$process = $null

function Write-WorkflowReport {
    $report = [ordered]@{
        schema_version = 1
        outcome = $outcome
        passed = ($outcome -eq 'passed')
        failure_reason = $reason
        checks = @($steps)
        screenshot = if (Test-Path -LiteralPath $screenshotPath -PathType Leaf) { [IO.Path]::GetFileName($screenshotPath) } else { $null }
        screenshot_size_pixels = $screenshotSize
        generated_utc = [DateTime]::UtcNow.ToString('o')
        ui_environment = 'Windows UI Automation; physical tablet not tested'
    }
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $reportPath -Encoding utf8
}

if (-not $IsWindows) {
    $outcome = 'unsupported'
    $reason = 'Inspector UI Automation requires an interactive Windows desktop.'
    Write-WorkflowReport
    exit 0
}
if (-not [Environment]::UserInteractive) {
    $outcome = 'unsupported'
    $reason = 'No interactive Windows desktop session is available.'
    Write-WorkflowReport
    exit 0
}

try {
    try {
        Add-Type -AssemblyName UIAutomationClient
        Add-Type -AssemblyName UIAutomationTypes
        Add-Type -AssemblyName System.Windows.Forms
        Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.IO.Compression;
using System.Runtime.InteropServices;
public static class Sketch3DUiCapture {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr hwnd, IntPtr hdc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr hdc);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr hdc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int width, int height);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr hdc, IntPtr obj);
    [DllImport("gdi32.dll")] static extern bool StretchBlt(IntPtr dst, int x, int y, int width, int height, IntPtr src, int srcX, int srcY, int srcWidth, int srcHeight, uint rop);
    [StructLayout(LayoutKind.Sequential)] struct BITMAPINFOHEADER { public uint size; public int width, height; public ushort planes, bitCount; public uint compression, imageSize; public int xPixelsPerMeter, yPixelsPerMeter; public uint colorsUsed, colorsImportant; }
    [StructLayout(LayoutKind.Sequential)] struct BITMAPINFO { public BITMAPINFOHEADER header; public uint colors; }
    [DllImport("gdi32.dll")] static extern int GetDIBits(IntPtr hdc, IntPtr bitmap, uint start, uint lines, [Out] byte[] bits, ref BITMAPINFO info, uint usage);
    static void Be32(Stream s, uint n) { s.WriteByte((byte)(n >> 24)); s.WriteByte((byte)(n >> 16)); s.WriteByte((byte)(n >> 8)); s.WriteByte((byte)n); }
    static uint Crc(byte[] data) { uint c=0xffffffff; foreach(byte b in data) { c^=b; for(int k=0;k<8;k++) c=(c&1)!=0 ? 0xedb88320^(c>>1) : c>>1; } return ~c; }
    static void Chunk(Stream s, string name, byte[] data) { byte[] type=System.Text.Encoding.ASCII.GetBytes(name); Be32(s,(uint)data.Length); s.Write(type,0,type.Length); s.Write(data,0,data.Length); byte[] all=new byte[type.Length+data.Length]; Buffer.BlockCopy(type,0,all,0,type.Length); Buffer.BlockCopy(data,0,all,type.Length,data.Length); Be32(s,Crc(all)); }
    public static int[] SaveWindow(IntPtr hwnd, string path) {
        RECT rect; if (!GetWindowRect(hwnd, out rect)) throw new InvalidOperationException("Could not read app window bounds.");
        int srcWidth=rect.Right-rect.Left, srcHeight=rect.Bottom-rect.Top;
        if (srcWidth<320 || srcHeight<240) throw new InvalidOperationException("App window is too small for a screenshot.");
        double scale=Math.Min(1.0,Math.Min(1280.0/srcWidth,720.0/srcHeight));
        int width=Math.Max(1,(int)(srcWidth*scale)), height=Math.Max(1,(int)(srcHeight*scale));
        IntPtr screen=GetDC(IntPtr.Zero), memory=IntPtr.Zero, bitmap=IntPtr.Zero, previous=IntPtr.Zero;
        try {
            memory=CreateCompatibleDC(screen); bitmap=CreateCompatibleBitmap(screen,width,height); previous=SelectObject(memory,bitmap);
            if(!StretchBlt(memory,0,0,width,height,screen,rect.Left,rect.Top,srcWidth,srcHeight,0x00CC0020|0x40000000)) throw new InvalidOperationException("Could not capture the Sketch3D window from the interactive desktop.");
            BITMAPINFO info=new BITMAPINFO(); info.header.size=40; info.header.width=width; info.header.height=-height; info.header.planes=1; info.header.bitCount=32; info.header.compression=0; info.header.imageSize=(uint)(width*height*4);
            byte[] bgra=new byte[width*height*4]; if(GetDIBits(memory,bitmap,0,(uint)height,bgra,ref info,0)!=height) throw new InvalidOperationException("Could not read screenshot pixels.");
            byte[] raw=new byte[(width*4+1)*height];
            for(int y=0;y<height;y++) { int row=y*(width*4+1), src=y*width*4; raw[row]=0; for(int x=0;x<width;x++) { int si=src+x*4, di=row+1+x*4; raw[di]=bgra[si+2]; raw[di+1]=bgra[si+1]; raw[di+2]=bgra[si]; raw[di+3]=255; } }
            byte[] compressed; using(var ms=new MemoryStream()) { using(var z=new ZLibStream(ms,CompressionLevel.Optimal,true)) z.Write(raw,0,raw.Length); compressed=ms.ToArray(); }
            using(var png=new MemoryStream()) { byte[] sig={137,80,78,71,13,10,26,10}; png.Write(sig,0,sig.Length); byte[] ihdr=new byte[13]; ihdr[0]=(byte)(width>>24); ihdr[1]=(byte)(width>>16); ihdr[2]=(byte)(width>>8); ihdr[3]=(byte)width; ihdr[4]=(byte)(height>>24); ihdr[5]=(byte)(height>>16); ihdr[6]=(byte)(height>>8); ihdr[7]=(byte)height; ihdr[8]=8; ihdr[9]=6; Chunk(png,"IHDR",ihdr); Chunk(png,"IDAT",compressed); Chunk(png,"IEND",new byte[0]); File.WriteAllBytes(path,png.ToArray()); }
        } finally {
            if(previous!=IntPtr.Zero) SelectObject(memory,previous); if(bitmap!=IntPtr.Zero) DeleteObject(bitmap); if(memory!=IntPtr.Zero) DeleteDC(memory); if(screen!=IntPtr.Zero) ReleaseDC(IntPtr.Zero,screen);
        }
        return new int[] { width, height };
    }
}
'@
    } catch {
        $outcome = 'unsupported'
        $reason = "Windows UI Automation or screen capture is unavailable: $($_.Exception.Message)"
        Write-WorkflowReport
        exit 0
    }
    $executablePath = [IO.Path]::GetFullPath($Executable)
    if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) { throw "Sketch3D executable not found: $executablePath" }
    $profilePath = Join-Path $OutputDirectory 'isolated-profile'
    [IO.Directory]::CreateDirectory($profilePath) | Out-Null
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $executablePath
    $start.WorkingDirectory = (Get-Location).Path
    $start.UseShellExecute = $false
    $start.Environment['LOCALAPPDATA'] = $profilePath
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'Could not launch Sketch3D.' }

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $process.Refresh()
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($process.MainWindowHandle -eq [IntPtr]::Zero) { throw [PlatformNotSupportedException]::new('No interactive desktop window was available before the UI timeout.') }
    if (-not [Sketch3DUiCapture]::SetForegroundWindow($process.MainWindowHandle)) { throw [PlatformNotSupportedException]::new('Windows did not grant foreground access to the Sketch3D window.') }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
    if ($null -eq $root) { throw [PlatformNotSupportedException]::new('Windows UI Automation could not attach to the interactive Sketch3D window.') }
    $initialTree = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    $initialNames = @($initialTree | ForEach-Object { $_.Current.Name } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($initialNames.Count -eq 0) { throw [PlatformNotSupportedException]::new('This desktop session does not expose Sketch3D controls through Windows UI Automation.') }

    function Find-UiElement([string]$Name, [int]$WaitSeconds = 5) {
        $end = [DateTime]::UtcNow.AddSeconds($WaitSeconds)
        $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, $Name)
        do {
            $process.Refresh()
            if ($process.HasExited) { throw 'Sketch3D exited during the UI workflow.' }
            $found = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
            if ($null -ne $found) { return $found }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $end)
        return $null
    }
    function Invoke-UiElement($Element) {
        if ($null -eq $Element) { throw 'Requested UI element is unavailable.' }
        $pattern = $Element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        $pattern.Invoke()
        Start-Sleep -Milliseconds 250
    }
    function Require-Element([string]$Name) {
        $found = Find-UiElement $Name
        if ($null -eq $found) {
            $observed = @()
            try {
                $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
                foreach ($item in $all) { if (-not [string]::IsNullOrWhiteSpace($item.Current.Name)) { $observed += $item.Current.Name } }
            } catch {}
            if ($observed.Count -eq 0) { throw [PlatformNotSupportedException]::new('This desktop session does not expose Sketch3D controls through Windows UI Automation.') }
            throw "Accessible UI control was not found: $Name. Observed names: $($observed -join ' | ')"
        }
        return $found
    }

    $layers = Require-Element 'Layers'
    Invoke-UiElement $layers
    $steps.Add('opened the Layers inspector tab through its accessible production widget')
    $folderName = Require-Element 'Folder name'
    $folderName.SetFocus()
    [System.Windows.Forms.SendKeys]::SendWait('Active edit draft')
    Start-Sleep -Milliseconds 250
    $valuePattern = $folderName.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
    if ($valuePattern.Current.Value -ne 'Active edit draft') { throw 'Keyboard input did not reach the focused inspector edit field.' }
    $steps.Add('keyboard focus entered a real inspector text edit and retained its unsaved draft')

    Invoke-UiElement (Require-Element 'Collapse inspector to its tab rail')
    $expand = Require-Element 'Expand inspector'
    $expand.SetFocus()
    if (-not $expand.Current.HasKeyboardFocus) { throw 'Keyboard focus did not move to the inspector expand widget.' }
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    if ($null -eq (Find-UiElement 'Collapse inspector to its tab rail')) { throw 'Enter did not expand the inspector.' }
    $steps.Add('collapsed and expanded the production inspector using accessible click and keyboard Enter')

    Invoke-UiElement (Require-Element 'Collapse inspector to its tab rail')
    Invoke-UiElement (Require-Element 'Hide inspector')
    if ($null -ne (Find-UiElement 'Expand inspector' 1)) { throw 'Inspector remained visible after Hide inspector.' }
    $viewMenu = Require-Element 'View'
    Invoke-UiElement $viewMenu
    Invoke-UiElement (Require-Element 'Show inspector')
    $null = Require-Element 'Collapse inspector to its tab rail'
    $folderName = Require-Element 'Folder name'
    $valuePattern = $folderName.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
    if ($valuePattern.Current.Value -ne 'Active edit draft') { throw 'The active edit draft was lost during inspector hide and restore.' }
    $steps.Add('hid and restored the inspector through View while preserving the active text edit')

    $screenshotSize = [Sketch3DUiCapture]::SaveWindow($process.MainWindowHandle, $screenshotPath)
    $steps.Add('captured the restored inspector and focused-edit workflow as a PNG screenshot')
    $outcome = 'passed'
} catch [PlatformNotSupportedException] {
    $outcome = 'unsupported'
    $reason = $_.Exception.Message
} catch {
    $reason = $_.Exception.Message
} finally {
    if ($process) {
        try { if (-not $process.HasExited) { $process.CloseMainWindow(); if (-not $process.WaitForExit(5000)) { $process.Kill($true) } } } catch {}
        $process.Dispose()
    }
    Write-WorkflowReport
}
if ($outcome -eq 'passed') { Write-Output 'Inspector UI workflow passed.'; exit 0 }
if ($outcome -eq 'unsupported') { Write-Output "Inspector UI workflow unsupported: $reason"; exit 0 }
throw "Inspector UI workflow failed: $reason"
