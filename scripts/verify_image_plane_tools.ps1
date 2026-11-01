[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1, 300)][int]$TimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$verificationPath = Join-Path $workspacePath 'verification'
$reportPath = Join-Path $verificationPath 'image-plane-report.json'
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $Executable = Join-Path $workspacePath 'target/release/sketch3d.exe'
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
        -ArgumentList '--verify-image-planes' `
        -WorkingDirectory $workspacePath `
        -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $verificationPath 'image-plane-tools.stdout.log') `
        -RedirectStandardError (Join-Path $verificationPath 'image-plane-tools.stderr.log') `
        -PassThru

    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        throw "Image-plane tool verification did not finish within $TimeoutSeconds seconds."
    }
    $process.Refresh()
    if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) {
        throw "Image-plane tool verification failed (exit $($process.ExitCode)). See verification/image-plane-tools.stderr.log."
    }
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        throw 'Image-plane tool verification did not produce its report.'
    }
    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $requiredChecks = @(
        'transformed finite image-plane line drag and undo/redo',
        'image-plane fill click-to-close, clipping, and undo/redo',
        'rotated image-plane circle drag, clipping, and undo/redo',
        'rotated image-plane ellipse drag, clipping, and undo/redo',
        'image-plane curve anchor clicks, clipping, and undo/redo',
        'line, fill, circle, and curve cancel without document edits',
        'hidden and locked image planes reject all planar tools'
    )
    foreach ($check in $requiredChecks) {
        if ($report.checks -notcontains $check) {
            throw "Image-plane tool report is missing check: $check"
        }
    }
    if ($report.passed -ne $true -or $report.physical_tablet_tested -ne $false) {
        throw 'Image-plane tool report must pass and must disclose that physical tablet testing was deferred.'
    }
    Write-Output 'Image-plane planar-tool verification passed.'
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
