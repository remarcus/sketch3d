[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1, 300)][int]$TimeoutSeconds = 90
)

$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$verificationPath = [IO.Path]::GetFullPath((Join-Path $workspacePath 'verification'))
$reportPath = Join-Path $verificationPath 'camera-bookmark-restart-report.json'
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $Executable = Join-Path $workspacePath 'target/debug/sketch3d.exe'
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$runRoot = Join-Path $verificationPath ("camera-bookmark-restart-{0}" -f [guid]::NewGuid().ToString('N'))
$profilePath = Join-Path $runRoot 'profile'
$seedStdout = Join-Path $runRoot 'seed.stdout.log'
$seedStderr = Join-Path $runRoot 'seed.stderr.log'
$reopenStdout = Join-Path $runRoot 'reopen.stdout.log'
$reopenStderr = Join-Path $runRoot 'reopen.stderr.log'

$resolvedRunRoot = [IO.Path]::GetFullPath($runRoot)
$verificationPrefix = $verificationPath.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $resolvedRunRoot.StartsWith($verificationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing to create the restart-verification workspace outside verification/.'
}
$null = New-Item -ItemType Directory -Path $profilePath -Force
if (Test-Path -LiteralPath $reportPath -PathType Leaf) {
    Remove-Item -LiteralPath $reportPath
}

$previousLocalAppData = [Environment]::GetEnvironmentVariable('LOCALAPPDATA', 'Process')
$process = $null
$passed = $false
try {
    [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $profilePath, 'Process')
    $process = Start-Process `
        -FilePath $Executable `
        -ArgumentList @('--verify-camera-bookmarks-seed') `
        -WorkingDirectory $workspacePath `
        -WindowStyle Hidden `
        -RedirectStandardOutput $seedStdout `
        -RedirectStandardError $seedStderr `
        -PassThru
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        throw 'The first camera-bookmark process did not exit within the timeout.'
    }
    $process.Refresh()
    if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) {
        throw 'The first camera-bookmark process failed. See its logs in the verification run folder.'
    }
    $bookmarkPath = Join-Path $profilePath 'SpatialSketch/camera-bookmarks.json'
    if (-not (Test-Path -LiteralPath $bookmarkPath -PathType Leaf)) {
        throw 'The first process did not create the production workspace camera-bookmark file.'
    }

    $process = Start-Process `
        -FilePath $Executable `
        -ArgumentList @('--verify-camera-bookmarks', $reportPath) `
        -WorkingDirectory $workspacePath `
        -WindowStyle Hidden `
        -RedirectStandardOutput $reopenStdout `
        -RedirectStandardError $reopenStderr `
        -PassThru
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        throw 'The second camera-bookmark application process did not exit within the timeout.'
    }
    $process.Refresh()
    if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) {
        throw 'The second camera-bookmark application process failed. See its logs in the verification run folder.'
    }
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        throw 'The second application process did not produce its verification report.'
    }

    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $requiredChecks = @(
        'first process saved two full camera views and a timed ordered sequence through CameraBookmarkStore::save',
        'second application process loaded the exact saved library through SketchApp startup',
        'bookmark projection, framing, target, sequence order, and transition duration survived restart'
    )
    foreach ($check in $requiredChecks) {
        if ($report.checks -notcontains $check) {
            throw "Restart report is missing check: $check"
        }
    }
    if ($report.passed -ne $true -or $report.bookmark_count -ne 2 -or $report.sequence_count -ne 1) {
        throw 'Restart report did not confirm the expected saved camera library.'
    }
    $passed = $true
    Write-Output 'Camera-bookmark process restart verification passed.'
    Write-Output "Report: $reportPath"
} finally {
    if ($null -ne $process) {
        $process.Refresh()
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
            $null = $process.WaitForExit(5000)
        }
    }
    [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $previousLocalAppData, 'Process')
    if ($passed -and (Test-Path -LiteralPath $resolvedRunRoot -PathType Container)) {
        $resolvedRun = (Resolve-Path -LiteralPath $resolvedRunRoot).Path
        if (-not $resolvedRun.StartsWith($verificationPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Refusing to remove a restart-verification folder outside verification/.'
        }
        Remove-Item -LiteralPath $resolvedRun -Recurse -Force
    }
}
