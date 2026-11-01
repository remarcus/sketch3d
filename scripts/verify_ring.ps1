[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1, 300)][int]$TimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$verificationPath = Join-Path $workspacePath 'verification'
$reportPath = Join-Path $verificationPath 'ring-report.json'
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
        -ArgumentList '--verify-ring' `
        -WorkingDirectory $workspacePath `
        -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $verificationPath 'ring.stdout.log') `
        -RedirectStandardError (Join-Path $verificationPath 'ring.stderr.log') `
        -PassThru

    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        throw "Ring verification did not finish within $TimeoutSeconds seconds."
    }
    $process.Refresh()
    if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) {
        throw 'Ring verification failed. See verification/ring.stderr.log.'
    }
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        throw 'Ring verification did not produce its report.'
    }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $requiredChecks = @(
        'Add ring action creates and activates a saved torus at the camera target',
        'active ring camera alignment faces the torus hole',
        'synthetic pointer samples draw on the saved ring surface',
        'a center-hole miss splits the drawing into two surface strokes',
        'ring strokes remain on the torus instead of cutting through its hole',
        'cancel removes an in-progress ring stroke without history',
        'undo and redo preserve split ring strokes',
        'ring strokes export to GLB'
    )
    foreach ($check in $requiredChecks) {
        if ($report.checks -notcontains $check) {
            throw "Ring report is missing check: $check"
        }
    }
    if ($report.passed -ne $true -or $report.physical_tablet_tested -ne $false) {
        throw 'Ring report must pass and disclose that physical tablet testing was deferred.'
    }
    Write-Output 'Saved-ring interaction verification passed.'
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
