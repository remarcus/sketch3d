[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1, 300)][int]$TimeoutSeconds = 90
)

$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$verificationPath = [IO.Path]::GetFullPath((Join-Path $workspacePath 'verification'))
$outputPath = Join-Path $verificationPath 'appearance-layout'
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $Executable = Join-Path $workspacePath 'target/debug/sketch3d.exe'
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$runProfile = Join-Path $outputPath 'profile'
$runRoot = [IO.Path]::GetFullPath($runProfile)
$verificationPrefix = $verificationPath.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $runRoot.StartsWith($verificationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing to create appearance verification settings outside verification/.'
}
$null = New-Item -ItemType Directory -Path $runProfile -Force

$previousLocalAppData = [Environment]::GetEnvironmentVariable('LOCALAPPDATA', 'Process')
$captures = [System.Collections.Generic.List[object]]::new()
$captureRequests = [System.Collections.Generic.List[object]]::new()
foreach ($theme in @('dark', 'light')) {
    foreach ($zoom in @(0.8, 1.5, 2.0)) {
        $captureRequests.Add([pscustomobject]@{
            theme = $theme
            kind = 'zoom'
            zoom = $zoom
            width = 1440
            height = 940
        })
    }
    foreach ($size in @(@(1280, 720), @(1920, 1080))) {
        $captureRequests.Add([pscustomobject]@{
            theme = $theme
            kind = 'viewport'
            zoom = 1.0
            width = $size[0]
            height = $size[1]
        })
    }
    foreach ($state in @('brush', 'select', 'layers', 'color-picker', 'library', 'recovery-error', 'export', 'scene-dark-background', 'scene-light-background', 'scene-custom-background', 'scene-image-background', 'scene-mirror-preview-dark', 'scene-mirror-preview-light', 'scene-mirror-preview-custom', 'scene-mirror-preview-image', 'scene-brush-selection-drag-light', 'scene-brush-selection-drag-image', 'scene-mixed-resource-disabled', 'scene-liquify-cursor-light', 'scene-liquify-cursor-image')) {
        foreach ($zoom in @(1.0, 1.5, 2.0)) {
            foreach ($size in @(@(1280, 720), @(1920, 1080))) {
                $captureRequests.Add([pscustomobject]@{
                    theme = $theme
                    kind = 'state'
                    state = $state
                    zoom = $zoom
                    width = $size[0]
                    height = $size[1]
                })
            }
        }
    }
}
try {
    [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $runProfile, 'Process')
    foreach ($request in $captureRequests) {
        $theme = $request.theme
        $zoom = [double]$request.zoom
        $state = if ($request.PSObject.Properties.Name -contains 'state') { [string]$request.state } else { 'workspace' }
        $width = [int]$request.width
        $height = [int]$request.height
        $percent = [int][Math]::Round($zoom * 100)
        $suffix = if ($request.kind -eq 'zoom') { "$state-$percent-percent" } else { "$state-viewport-${width}x${height}-$percent-percent" }
        $reportPath = Join-Path $outputPath "$theme-$suffix.json"
        $imagePath = Join-Path $outputPath "$theme-$suffix.png"
        $stdoutPath = Join-Path $outputPath ".$theme-$suffix.stdout.log"
        $stderrPath = Join-Path $outputPath ".$theme-$suffix.stderr.log"
        $artifactsProduced = $false
        for ($attempt = 1; $attempt -le 2 -and -not $artifactsProduced; $attempt++) {
            foreach ($previousOutput in @($reportPath, $imagePath, $stdoutPath, $stderrPath)) {
                if (Test-Path -LiteralPath $previousOutput -PathType Leaf) {
                    Remove-Item -LiteralPath $previousOutput
                }
            }
            $process = Start-Process `
                -FilePath $Executable `
                -ArgumentList @(
                    '--verify-appearance-layout',
                    $theme,
                    $zoom.ToString([Globalization.CultureInfo]::InvariantCulture),
                    $width.ToString([Globalization.CultureInfo]::InvariantCulture),
                    $height.ToString([Globalization.CultureInfo]::InvariantCulture),
                    $state
                ) `
                -WorkingDirectory $workspacePath `
                -WindowStyle Hidden `
                -RedirectStandardOutput $stdoutPath `
                -RedirectStandardError $stderrPath `
                -PassThru
            $captureTimedOut = $false
            $captureExitCode = $null
            try {
                if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
                    $captureTimedOut = $true
                } else {
                    $process.Refresh()
                    $captureExitCode = $process.ExitCode
                }
            } finally {
                $process.Refresh()
                if (-not $process.HasExited) {
                    Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
                    $null = $process.WaitForExit(5000)
                }
            }
            if ($captureTimedOut) {
                if ($attempt -eq 2) {
                    throw "Appearance capture timed out twice for $theme $state at $width`x$height and $percent percent."
                }
                continue
            }
            if ($null -ne $captureExitCode -and $captureExitCode -ne 0) {
                if ($attempt -eq 2) {
                    throw "Appearance capture failed twice for $theme $state at $width`x$height and $percent percent (exit $captureExitCode); see $stderrPath."
                }
                continue
            }
            $artifactsProduced = (Test-Path -LiteralPath $reportPath -PathType Leaf) -and
                (Test-Path -LiteralPath $imagePath -PathType Leaf)
        }
        if (-not $artifactsProduced) {
            throw "Appearance capture did not produce both artifacts after one retry for $theme $state at $width`x$height and $percent percent."
        }
        $capture = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
        if ($capture.passed -ne $true -or
            $capture.theme -ne $theme -or
            $capture.state -ne $state -or
            $capture.egui_zoom_percent -ne $percent -or
            $capture.requested_viewport_points.Count -ne 2 -or
            $capture.requested_viewport_points[0] -ne $width -or
            $capture.requested_viewport_points[1] -ne $height -or
            $capture.screenshot_size_pixels.Count -ne 2 -or
            $capture.screenshot_size_pixels[0] -ne $width -or
            $capture.screenshot_size_pixels[1] -ne $height) {
            throw "Appearance capture report is invalid for $theme at $width`x$height and $percent percent."
        }
        $captures.Add([pscustomobject]@{
            theme = $theme
            capture_kind = $request.kind
            state = $state
            egui_zoom_percent = $percent
            requested_viewport_points = $capture.requested_viewport_points
            screenshot = [IO.Path]::GetFileName($imagePath)
            report = [IO.Path]::GetFileName($reportPath)
            screenshot_size_pixels = $capture.screenshot_size_pixels
        })
    }
} finally {
    [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $previousLocalAppData, 'Process')
}

$stateCaptures = @($captures | Where-Object { $_.capture_kind -eq 'state' })
$expectedStateCaptureCount = 20 * 2 * 2 * 3
if ($stateCaptures.Count -ne $expectedStateCaptureCount) {
    throw "Expected $expectedStateCaptureCount named UI-state captures; found $($stateCaptures.Count)."
}
$expectedCaptureCount = $expectedStateCaptureCount + 2 * (3 + 2)
if ($captures.Count -ne $expectedCaptureCount) {
    throw "Expected $expectedCaptureCount total captures; found $($captures.Count)."
}
$summary = [pscustomobject]@{
    passed = $true
    checks = @(
        'the full editing workspace rendered in Dark and Light themes at 80%, 150%, and 200% egui zoom',
        'all hidden app processes used settings separate from the user profile and saved fresh screenshots at their requested dimensions',
        'the workspace rendered in Dark and Light at 1280x720 and 1920x1080 viewport sizes',
        'Brush, Select, Layers, color picker, library, recovery error, PNG export, and light, dark, custom, and image scene backgrounds rendered in both themes at 1280x720 and 1920x1080 with 100%, 150%, and 200% egui zoom',
        'production live line and mirror preview overlays rendered against dark, light, custom, and mixed image backgrounds',
        'production brush-selection drag paths and radius cursors rendered against light and mixed image backgrounds after crossing a real fixture stroke',
        'mixed-resource translation inspector rendered with Rotate, Scale, and Apply disabled before preview movement',
        'the painted Liquify radius cursor rendered at a synthetic in-canvas pointer location on light and image backgrounds',
        'the Color control remains visible at 150% and wraps to a second row at 200%',
        'the empty-canvas message remains legible within the reduced canvas width at 200%'
    )
    limitations = @(
        'viewport and egui zoom checks do not replace operating-system DPI scaling walkthroughs',
        'Windows Narrator announcements still require direct assistive-technology testing',
        'OS-drawn Brush crosshair visibility is not present in screenshots and needs desktop review'
    )
    physical_tablet_tested = $false
    state_capture_count = $stateCaptures.Count
    screenshots = @($captures)
}
$summaryPath = Join-Path $outputPath 'appearance-layout-report.json'
[IO.File]::WriteAllText($summaryPath, ($summary | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
Write-Output "Appearance layout captures passed ($($captures.Count) total; $($stateCaptures.Count) named UI-state captures)."
Write-Output "Report: $summaryPath"
