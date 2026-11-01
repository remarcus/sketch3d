[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1, 300)][int]$TimeoutSeconds = 90
)

$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$verificationPath = [IO.Path]::GetFullPath((Join-Path $workspacePath 'verification'))
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $Executable = Join-Path $workspacePath 'target/debug/sketch3d.exe'
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$runRoot = Join-Path $verificationPath ("drawing-backup-interruption-{0}" -f [guid]::NewGuid().ToString('N'))
$profilePath = Join-Path $runRoot 'profile'
$markerPath = Join-Path $runRoot 'copy-paused.marker'
$sourcePath = Join-Path $runRoot 'interrupted.sketch.json'
$destinationPath = Join-Path $runRoot 'restored/restored.sketch.json'
$reportPath = Join-Path $verificationPath 'drawing-backup-interruption-report.json'
$childStdout = Join-Path $runRoot 'child.stdout.log'
$childStderr = Join-Path $runRoot 'child.stderr.log'
$reopenStdout = Join-Path $runRoot 'reopen.stdout.log'
$reopenStderr = Join-Path $runRoot 'reopen.stderr.log'

$resolvedRunRoot = [IO.Path]::GetFullPath($runRoot)
$verificationPrefix = $verificationPath.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $resolvedRunRoot.StartsWith($verificationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing to create the interruption-verification workspace outside verification/.'
}
$null = New-Item -ItemType Directory -Path $profilePath -Force
if (Test-Path -LiteralPath $reportPath -PathType Leaf) {
    Remove-Item -LiteralPath $reportPath
}

$previousLocalAppData = [Environment]::GetEnvironmentVariable('LOCALAPPDATA', 'Process')
$previousPauseMarker = [Environment]::GetEnvironmentVariable('SKETCH3D_VERIFY_COPY_PAUSE_MARKER', 'Process')
$process = $null
$passed = $false
try {
    [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $profilePath, 'Process')
    [Environment]::SetEnvironmentVariable('SKETCH3D_VERIFY_COPY_PAUSE_MARKER', $markerPath, 'Process')
    $process = Start-Process `
        -FilePath $Executable `
        -ArgumentList @('--verify-library-backup-interrupt-child', $sourcePath) `
        -WorkingDirectory $workspacePath `
        -WindowStyle Hidden `
        -RedirectStandardOutput $childStdout `
        -RedirectStandardError $childStderr `
        -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while (-not (Test-Path -LiteralPath $markerPath -PathType Leaf) -and [DateTime]::UtcNow -lt $deadline) {
        $process.Refresh()
        if ($process.HasExited) {
            throw 'The child copy process exited before reaching the partial-write marker.'
        }
        Start-Sleep -Milliseconds 25
    }
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        throw 'The child copy process did not reach its partial-write marker before timeout.'
    }
    $process.Refresh()
    if ($process.HasExited) {
        throw 'The child copy process exited instead of pausing after a partial asset write.'
    }

    $trashRoot = Join-Path $profilePath 'SpatialSketch/brushes.json/drawing-trash'
    $stages = @(Get-ChildItem -LiteralPath $trashRoot -Directory -Force -Filter '.sketch-backup-*')
    if ($stages.Count -ne 1) {
        throw "Expected one real staging directory before killing the child; found $($stages.Count)."
    }
    $partialAsset = Join-Path $stages[0].FullName 'drawing.sketch.json.assets/interrupt-probe.bin'
    if ((Get-Item -LiteralPath $partialAsset).Length -ne 8) {
        throw 'The child had not written exactly eight bytes into the staging asset file.'
    }
    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        throw 'The source drawing disappeared before the backup was installed.'
    }

    Stop-Process -Id $process.Id -Force
    $process.WaitForExit()
    $process.Refresh()
    if (-not $process.HasExited) {
        throw 'The interrupted copy process remained alive after termination.'
    }
    $process = $null
    [Environment]::SetEnvironmentVariable('SKETCH3D_VERIFY_COPY_PAUSE_MARKER', $null, 'Process')

    $process = Start-Process `
        -FilePath $Executable `
        -ArgumentList @('--verify-library-backup-interrupt-reopen', $sourcePath, $destinationPath, $reportPath) `
        -WorkingDirectory $workspacePath `
        -WindowStyle Hidden `
        -RedirectStandardOutput $reopenStdout `
        -RedirectStandardError $reopenStderr `
        -PassThru
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        throw 'The fresh-process retry/restore verifier did not exit within the timeout.'
    }
    $process.Refresh()
    if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) {
        throw 'The fresh-process retry/restore verifier failed. See its logs in the verification run folder.'
    }
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        throw 'The fresh process did not produce its interruption verification report.'
    }

    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    if ($report.passed -ne $true -or $report.partial_asset_bytes_before_termination -ne 8 -or $report.restored_asset_bytes -ne 1048576) {
        throw 'The interruption report did not confirm partial-copy termination and full retry/restore.'
    }
    $passed = $true
    Write-Output 'Drawing backup active-copy interruption verification passed.'
    Write-Output "Report: $reportPath"
} finally {
    if ($null -ne $process) {
        $process.Refresh()
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            $null = $process.WaitForExit(5000)
        }
    }
    [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $previousLocalAppData, 'Process')
    [Environment]::SetEnvironmentVariable('SKETCH3D_VERIFY_COPY_PAUSE_MARKER', $previousPauseMarker, 'Process')
    if ($passed -and (Test-Path -LiteralPath $resolvedRunRoot -PathType Container)) {
        $resolvedRun = (Resolve-Path -LiteralPath $resolvedRunRoot).Path
        if (-not $resolvedRun.StartsWith($verificationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Refusing to remove an interruption-verification folder outside verification/.'
        }
        Remove-Item -LiteralPath $resolvedRun -Recurse -Force
    }
}
