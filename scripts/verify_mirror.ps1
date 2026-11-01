[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1, 300)][int]$TimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$verificationPath = Join-Path $workspacePath 'verification'
$reportPath = Join-Path $verificationPath 'mirror-report.json'
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $Executable = Join-Path $workspacePath 'target/debug/sketch3d.exe'
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$null = New-Item -ItemType Directory -Path $verificationPath -Force
if (Test-Path -LiteralPath $reportPath -PathType Leaf) {
    Remove-Item -LiteralPath $reportPath
}

$process = $null
try {
    $process = Start-Process `
        -FilePath $Executable `
        -ArgumentList '--verify-mirror' `
        -WorkingDirectory $workspacePath `
        -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $verificationPath 'mirror.stdout.log') `
        -RedirectStandardError (Join-Path $verificationPath 'mirror.stderr.log') `
        -PassThru

    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        throw "Mirror verification did not finish within $TimeoutSeconds seconds."
    }
    $process.Refresh()
    if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) {
        throw 'Mirror verification failed. See verification/mirror.stderr.log.'
    }
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        throw 'Mirror verification did not produce its report.'
    }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $requiredChecks = @(
        'synthetic pointer-handler samples draw on an active saved cylinder guide',
        'curved-guide brush points remain on the guide surface',
        'mirror preview geometry does not mutate the document',
        'committed mirrored geometry matches its preview exactly',
        'source and mirrored run commit as one undo edit',
        'undo/redo preserve the exact mirrored pair',
        'cancel discards an in-progress source without history',
        'whole-drawing GLB includes the mirrored pair',
        'mouse canvas events preview, cancel, and commit a straight line on a rotated active guide',
        'active guide horizontal constraint and mirrored points remain on the rotated plane',
        'line source and mirror commit as one undo edit and survive undo/redo'
    )
    foreach ($check in $requiredChecks) {
        if ($report.checks -notcontains $check) {
            throw "Mirror report is missing check: $check"
        }
    }
    if ($report.passed -ne $true -or $report.physical_tablet_tested -ne $false) {
        throw 'Mirror report must pass and disclose that physical tablet testing was deferred.'
    }
    Write-Output 'Curved-guide brush and rotated-plane line mirror verification passed.'
    Write-Output "Report: $reportPath"
} finally {
    if ($null -ne $process) {
        $process.Refresh()
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
            $null = $process.WaitForExit(5000)
        }
    }
}
