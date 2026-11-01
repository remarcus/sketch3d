[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1, 300)][int]$ReadyTimeoutSeconds = 30,
    [ValidateRange(1, 300)][int]$RestartTimeoutSeconds = 45
)

$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$workspacePrefix = $workspacePath.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
$verificationPath = Join-Path $workspacePath 'verification'
$readyPath = Join-Path $verificationPath 'recovery-crash-ready.json'
$reportPath = Join-Path $verificationPath 'recovery-report.json'
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $Executable = Join-Path $workspacePath 'target/release/sketch3d.exe'
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$crashProcess = $null
$restartProcess = $null

function Assert-WorkspacePath([string]$Path) {
    $absolutePath = [IO.Path]::GetFullPath($Path)
    # Rust canonicalize() returns extended Windows paths. Compare the same
    # normalized form rather than rejecting a valid workspace snapshot.
    if ($absolutePath.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        $absolutePath = '\\' + $absolutePath.Substring(8)
    } elseif ($absolutePath.StartsWith('\\?\', [StringComparison]::Ordinal)) {
        $absolutePath = $absolutePath.Substring(4)
    }
    if (-not $absolutePath.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Verification artifact is outside the workspace: $absolutePath"
    }
    return $absolutePath
}

function Read-Marker([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    try {
        return (Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json)
    } catch {
        # A writer can briefly expose an incomplete marker. Keep polling until
        # valid JSON appears or the bounded timeout reports the failure.
        return $null
    }
}

function Resolve-MarkerPath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw 'Recovery ready marker must contain snapshotPath and sourcePath.'
    }
    if (-not [IO.Path]::IsPathRooted($Path)) {
        $Path = Join-Path $workspacePath $Path
    }
    return (Assert-WorkspacePath $Path)
}

function Stop-OwnedProcess($Process) {
    if ($null -ne $Process) {
        $Process.Refresh()
        if (-not $Process.HasExited) {
            Stop-Process -Id $Process.Id -ErrorAction SilentlyContinue
            $null = $Process.WaitForExit(5000)
        }
    }
}

try {
    $null = New-Item -ItemType Directory -Path $verificationPath -Force
    # Delete only these exact stale test artifacts, after checking their
    # resolved absolute paths. Recovery snapshots themselves are never deleted.
    foreach ($artifactPath in @($readyPath, $reportPath)) {
        $checkedPath = Assert-WorkspacePath $artifactPath
        if (Test-Path -LiteralPath $checkedPath -PathType Leaf) {
            Remove-Item -LiteralPath $checkedPath
        }
    }

    $crashProcess = Start-Process -FilePath $Executable -ArgumentList '--verify-recovery-crash' -WorkingDirectory $workspacePath -WindowStyle Hidden -RedirectStandardOutput (Join-Path $workspacePath 'recovery-crash.stdout.log') -RedirectStandardError (Join-Path $workspacePath 'recovery-crash.stderr.log') -PassThru
    $readyDeadline = [DateTime]::UtcNow.AddSeconds($ReadyTimeoutSeconds)
    $marker = $null
    while ([DateTime]::UtcNow -lt $readyDeadline) {
        $crashProcess.Refresh()
        if ($crashProcess.HasExited) {
            throw "Crash fixture exited before it could be interrupted (exit $($crashProcess.ExitCode)). See recovery-crash.stderr.log."
        }
        $marker = Read-Marker $readyPath
        if ($null -ne $marker -and $marker.snapshotPath -and $marker.sourcePath) {
            break
        }
        Start-Sleep -Milliseconds 200
    }
    if ($null -eq $marker -or -not $marker.snapshotPath -or -not $marker.sourcePath) {
        throw "Recovery snapshot was not ready within $ReadyTimeoutSeconds seconds. See recovery-crash.stderr.log."
    }
    $snapshotPath = Resolve-MarkerPath $marker.snapshotPath
    $sourcePath = Resolve-MarkerPath $marker.sourcePath
    if (-not (Test-Path -LiteralPath $snapshotPath -PathType Leaf)) {
        throw "Confirmed recovery snapshot is missing: $snapshotPath"
    }
    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        throw "Original native drawing is missing: $sourcePath"
    }
    $sourceHash = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash
    $crashProcess.Refresh()
    if ($crashProcess.HasExited) {
        throw 'Crash fixture exited before the forced interruption.'
    }
    # This is the unexpected termination being tested. Never target processes
    # by image name: another user-owned drawing window may be running.
    Stop-Process -Id $crashProcess.Id
    if (-not $crashProcess.WaitForExit(5000)) {
        throw 'The crash fixture did not stop after the forced interruption.'
    }

    $restartStartedUtc = [DateTime]::UtcNow
    $restartProcess = Start-Process -FilePath $Executable -ArgumentList '--verify-recovery' -WorkingDirectory $workspacePath -WindowStyle Hidden -RedirectStandardOutput (Join-Path $workspacePath 'recovery-restart.stdout.log') -RedirectStandardError (Join-Path $workspacePath 'recovery-restart.stderr.log') -PassThru
    $restartDeadline = [DateTime]::UtcNow.AddSeconds($RestartTimeoutSeconds)
    do {
        $restartProcess.Refresh()
        if ($restartProcess.HasExited) { break }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $restartDeadline)
    $restartProcess.Refresh()
    if (-not $restartProcess.HasExited) {
        throw "Recovery verification did not finish within $RestartTimeoutSeconds seconds. See recovery-restart.stderr.log."
    }
    $restartProcess.WaitForExit()
    $restartExitCode = $restartProcess.ExitCode
    if ($null -ne $restartExitCode -and $restartExitCode -ne 0) {
        throw "Recovery verification failed (exit $restartExitCode). See recovery-restart.stderr.log."
    }
    if ($null -eq $restartExitCode) {
        Write-Warning 'The GUI process exit code was unavailable; requiring the fresh passed report and unchanged source checks below.'
    }
    $report = Read-Marker $reportPath
    if ($null -eq $report -or $report.passed -ne $true) {
        throw 'Recovery verification did not produce a successful report.'
    }
    if ((Get-Item -LiteralPath $reportPath).LastWriteTimeUtc -lt $restartStartedUtc.AddSeconds(-2)) {
        throw 'Recovery report predates this restart.'
    }
    if ((Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash -ne $sourceHash) {
        throw 'Recovery changed the original native drawing.'
    }
    Write-Output 'Forced-crash recovery verification passed.'
    Write-Output "Report: $reportPath"
} finally {
    Stop-OwnedProcess $restartProcess
    Stop-OwnedProcess $crashProcess
}
