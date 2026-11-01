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
try {
    [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $runProfile, 'Process')
    foreach ($theme in @('dark', 'light')) {
        foreach ($zoom in @(0.8, 1.5, 2.0)) {
            $percent = [int][Math]::Round($zoom * 100)
            $reportPath = Join-Path $outputPath "$theme-$percent-percent.json"
            $imagePath = Join-Path $outputPath "$theme-$percent-percent.png"
            $stdoutPath = Join-Path $outputPath ".$theme-$percent.stdout.log"
            $stderrPath = Join-Path $outputPath ".$theme-$percent.stderr.log"
            foreach ($previousOutput in @($reportPath, $imagePath, $stdoutPath, $stderrPath)) {
                if (Test-Path -LiteralPath $previousOutput -PathType Leaf) {
                    Remove-Item -LiteralPath $previousOutput
                }
            }
            $process = Start-Process `
                -FilePath $Executable `
                -ArgumentList @('--verify-appearance-layout', $theme, $zoom.ToString([Globalization.CultureInfo]::InvariantCulture)) `
                -WorkingDirectory $workspacePath `
                -WindowStyle Hidden `
                -RedirectStandardOutput $stdoutPath `
                -RedirectStandardError $stderrPath `
                -PassThru
            try {
                if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
                    throw "Appearance capture timed out for $theme at $percent percent."
                }
                $process.Refresh()
                if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) {
                    throw "Appearance capture failed for $theme at $percent percent; see $stderrPath."
                }
            } finally {
                $process.Refresh()
                if (-not $process.HasExited) {
                    Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
                    $null = $process.WaitForExit(5000)
                }
            }
            if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf) -or
                -not (Test-Path -LiteralPath $imagePath -PathType Leaf)) {
                throw "Appearance capture did not produce both artifacts for $theme at $percent percent."
            }
            $capture = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
            if ($capture.passed -ne $true -or
                $capture.theme -ne $theme -or
                $capture.egui_zoom_percent -ne $percent -or
                $capture.screenshot_size_pixels.Count -ne 2 -or
                $capture.screenshot_size_pixels[0] -ne 1440 -or
                $capture.screenshot_size_pixels[1] -ne 940) {
                throw "Appearance capture report is invalid for $theme at $percent percent."
            }
            $captures.Add([pscustomobject]@{
                theme = $theme
                egui_zoom_percent = $percent
                screenshot = [IO.Path]::GetFileName($imagePath)
                report = [IO.Path]::GetFileName($reportPath)
                screenshot_size_pixels = $capture.screenshot_size_pixels
            })
        }
    }
} finally {
    [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $previousLocalAppData, 'Process')
}

$summary = [pscustomobject]@{
    passed = $true
    checks = @(
        'the full editing workspace rendered in Dark and Light themes at 80%, 150%, and 200% egui zoom',
        'all six hidden app processes used settings separate from the user profile and saved 1440x940 screenshots',
        'the Color control remains visible at 150% and wraps to a second row at 200%',
        'the empty-canvas message remains legible within the reduced canvas width at 200%'
    )
    limitations = @(
        'egui zoom checks do not replace operating-system DPI scaling walkthroughs',
        'Windows Narrator announcements still require direct assistive-technology testing'
    )
    physical_tablet_tested = $false
    screenshots = @($captures)
}
$summaryPath = Join-Path $outputPath 'appearance-layout-report.json'
[IO.File]::WriteAllText($summaryPath, ($summary | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
Write-Output 'Appearance layout captures passed at 80%, 150%, and 200% in Dark and Light themes.'
Write-Output "Report: $summaryPath"
