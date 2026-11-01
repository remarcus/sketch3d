param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-z0-9][a-z0-9_-]{0,63}$')]
    [string]$Name,

    [Parameter(Mandatory = $true)]
    [ValidateRange(1, 90)]
    [int]$DurationSeconds,

    [ValidateSet('Sketch3D')]
    [string]$WindowTitle = 'Sketch3D',
    [ValidateRange(10, 30)]
    [int]$CaptureFps = 30,
    [ValidateRange(10, 15)]
    [int]$PreviewFps = 12,
    [string]$Fixture = 'house',
    [string]$FixtureVersion = 'unversioned',
    [string]$AppCommit = 'working-tree',
    [ValidateSet('Dark', 'Light', 'System', 'unknown')]
    [string]$Theme = 'unknown',
    [ValidateRange(50, 300)]
    [int]$DpiPercent = 100,
    [string]$Viewport = '1920x1080',
    [string[]]$ShotStep = @(),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\verification\capability-demos')
)

$ErrorActionPreference = 'Stop'
$maximumGifBytes = 5MB
$maximumMp4Bytes = 512MB
$maximumFrames = 2700

function Invoke-Ffmpeg {
    param([string[]]$Arguments)

    $processInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $processInfo.FileName = 'ffmpeg'
    $processInfo.UseShellExecute = $false
    $processInfo.RedirectStandardError = $true
    $processInfo.RedirectStandardOutput = $true
    foreach ($argument in $Arguments) {
        [void]$processInfo.ArgumentList.Add($argument)
    }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $processInfo
    if (-not $process.Start()) {
        throw 'Could not start FFmpeg.'
    }
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $process.WaitForExit()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) {
        throw "FFmpeg failed (exit $($process.ExitCode)): $stderr $stdout"
    }
}

if ($DurationSeconds * $CaptureFps -gt $maximumFrames) {
    throw "The recording exceeds the $maximumFrames-frame cap. Reduce duration or frame rate."
}
if ($Viewport -notmatch '^\d{3,5}x\d{3,5}$') {
    throw 'Viewport must use WIDTHxHEIGHT notation, for example 1920x1080.'
}
$dimensions = $Viewport -split 'x'
if ([int]$dimensions[0] -gt 7680 -or [int]$dimensions[1] -gt 4320) {
    throw 'The recorded viewport exceeds the 7680x4320 metadata limit.'
}

$ffmpeg = Get-Command ffmpeg -ErrorAction SilentlyContinue
if (-not $ffmpeg) {
    throw 'FFmpeg is required for capture encoding. Install it and make it available on PATH.'
}

$appWindows = @(Get-Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowTitle -ceq $WindowTitle -and $_.MainWindowHandle -ne 0 })
if ($appWindows.Count -eq 0) {
    throw "Could not find an open application window with the exact title '$WindowTitle'. This recorder never captures the desktop."
}
if ($appWindows.Count -gt 1) {
    throw "Found $($appWindows.Count) windows titled '$WindowTitle'. Close duplicate Sketch3D windows so the capture target is unambiguous."
}

$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
[System.IO.Directory]::CreateDirectory($outputRoot) | Out-Null
$masterPath = Join-Path $outputRoot "$Name.mp4"
$gifPath = Join-Path $outputRoot "$Name.gif"
$posterPath = Join-Path $outputRoot "$Name.png"
$manifestPath = Join-Path $outputRoot "$Name.recording.json"
$tempMasterPath = Join-Path $outputRoot ".$Name.capture.mp4"
$tempGifPath = Join-Path $outputRoot ".$Name.preview.gif"
$tempPosterPath = Join-Path $outputRoot ".$Name.poster.png"
$publishedPaths = [System.Collections.Generic.List[string]]::new()

foreach ($destination in @($masterPath, $gifPath, $posterPath, $manifestPath)) {
    if (Test-Path -LiteralPath $destination) {
        throw "Refusing to overwrite existing recording output: $destination"
    }
}
$temporaryPaths = @($tempMasterPath, $tempGifPath, $tempPosterPath, (Join-Path $outputRoot ".$Name.recording.json.tmp"))
foreach ($temporary in $temporaryPaths) {
    if (Test-Path -LiteralPath $temporary) {
        throw "Refusing to overwrite an unfinished capture file: $temporary"
    }
}

Write-Host "Recording only the '$WindowTitle' window. Close app dialogs and notifications first."
Write-Host 'Input is manual. This script does not synthesize pointer events or tablet pressure.'
$ready = Read-Host 'Type RECORD to begin after staging the shot and hiding private information'
if ($ready -cne 'RECORD') {
    Write-Host 'Capture cancelled.'
    exit 2
}

$startedAt = [DateTimeOffset]::UtcNow
try {
    Invoke-Ffmpeg @(
        '-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
        '-f', 'gdigrab', '-framerate', "$CaptureFps", '-draw_mouse', '1',
        '-i', "title=$WindowTitle", '-t', "$DurationSeconds", '-an',
        '-vf', 'pad=ceil(iw/2)*2:ceil(ih/2)*2', '-c:v', 'libx264', '-preset', 'slow',
        '-crf', '18', '-pix_fmt', 'yuv420p', '-movflags', '+faststart',
        $tempMasterPath
    )
    $masterBytes = (Get-Item -LiteralPath $tempMasterPath).Length
    if ($masterBytes -eq 0 -or $masterBytes -gt $maximumMp4Bytes) {
        throw "MP4 output is empty or exceeds the $maximumMp4Bytes-byte cap."
    }

    $posterSecond = [Math]::Max(0, [Math]::Min($DurationSeconds - 1, [Math]::Floor($DurationSeconds * 0.35)))
    Invoke-Ffmpeg @(
        '-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
        '-ss', "$posterSecond", '-i', $tempMasterPath, '-frames:v', '1',
        '-vf', 'scale=960:-2:flags=lanczos', '-fs', "$maximumGifBytes", $tempPosterPath
    )
    if ((Get-Item -LiteralPath $tempPosterPath).Length -ge $maximumGifBytes) {
        throw 'PNG poster reached the 5 MiB output cap; reduce the capture resolution.'
    }

    $gifBytes = $null
    if ($DurationSeconds -le 25) {
        $paletteFilter = "fps=$PreviewFps,scale=960:-2:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=64:stats_mode=diff[p];[s1][p]paletteuse=dither=sierra2_4a"
        Invoke-Ffmpeg @(
            '-hide_banner', '-loglevel', 'error', '-nostdin', '-y', '-i', $tempMasterPath,
            '-vf', $paletteFilter, '-loop', '0', '-fs', "$maximumGifBytes", $tempGifPath
        )
        $gifBytes = (Get-Item -LiteralPath $tempGifPath).Length
        if ($gifBytes -ge $maximumGifBytes) {
            Remove-Item -LiteralPath $tempGifPath -Force
            throw "GIF preview is $gifBytes bytes, over the $maximumGifBytes-byte budget. Shorten or simplify the shot and record again."
        }
    }

    $manifest = [ordered]@{
        format = 'spatial-sketch-capability-demo'
        manifest_version = 1
        clip = $Name
        recorded_utc = $startedAt.ToString('o')
        duration_seconds = $DurationSeconds
        app_commit = $AppCommit
        fixture = $Fixture
        fixture_version = $FixtureVersion
        viewport = $Viewport
        dpi_percent = $DpiPercent
        theme = $Theme
        capture_mode = 'manual user operation; app-window-only screen capture'
        input_note = 'No input automation, pressure synthesis, or latency measurement.'
        window_title = $WindowTitle
        shot_steps = @($ShotStep)
        license_or_provenance = 'Sketch3D original sample artwork or individually recorded fixture license'
        encoding = [ordered]@{
            mp4 = "FFmpeg gdigrab, window-title input, native window resolution, $CaptureFps fps capture, libx264 CRF 18"
            gif = if ($DurationSeconds -le 25) { "FFmpeg palettegen/paletteuse at $PreviewFps fps, 960px wide, <=5 MiB" } else { $null }
        }
        outputs = [ordered]@{
            mp4 = [ordered]@{ file = "$Name.mp4"; bytes = $masterBytes; sha256 = (Get-FileHash -LiteralPath $tempMasterPath -Algorithm SHA256).Hash.ToLowerInvariant() }
            gif = if ($gifBytes -ne $null) { [ordered]@{ file = "$Name.gif"; bytes = $gifBytes; sha256 = (Get-FileHash -LiteralPath $tempGifPath -Algorithm SHA256).Hash.ToLowerInvariant() } } else { $null }
            poster = [ordered]@{ file = "$Name.png"; bytes = (Get-Item -LiteralPath $tempPosterPath).Length; sha256 = (Get-FileHash -LiteralPath $tempPosterPath -Algorithm SHA256).Hash.ToLowerInvariant(); alt = 'Reviewed poster frame for the capability clip.' }
        }
        review = [ordered]@{
            reviewed = $false
            loop_boundary_checked = $false
            intended_readme_surface_checked = $false
            private_content_checked = $false
            reviewer = $null
            notes = 'Review and complete before publishing this clip.'
        }
    }
    $manifestTemp = Join-Path $outputRoot ".$Name.recording.json.tmp"
    $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestTemp -Encoding utf8

    Move-Item -LiteralPath $tempMasterPath -Destination $masterPath
    $publishedPaths.Add($masterPath)
    Move-Item -LiteralPath $tempPosterPath -Destination $posterPath
    $publishedPaths.Add($posterPath)
    if (Test-Path -LiteralPath $tempGifPath) {
        Move-Item -LiteralPath $tempGifPath -Destination $gifPath
        $publishedPaths.Add($gifPath)
    }
    Move-Item -LiteralPath $manifestTemp -Destination $manifestPath
    $publishedPaths.Add($manifestPath)
    Write-Host "Saved reviewed-candidate outputs under $outputRoot"
    Write-Host 'Review the MP4, GIF loop, poster, and manifest before adding any asset to README or docs.'
}
catch {
    foreach ($published in $publishedPaths) {
        if (Test-Path -LiteralPath $published) {
            Remove-Item -LiteralPath $published -Force -ErrorAction SilentlyContinue
        }
    }
    foreach ($temporary in @($tempMasterPath, $tempGifPath, $tempPosterPath, (Join-Path $outputRoot ".$Name.recording.json.tmp"))) {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        }
    }
    throw
}
