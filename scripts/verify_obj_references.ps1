[CmdletBinding()]
param(
    [string]$Executable,
    [ValidateRange(1, 300)][int]$TimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$verificationPath = Join-Path $workspacePath 'verification'
$reportPath = Join-Path $verificationPath 'obj-reference-report.json'
$previewPath = Join-Path $verificationPath 'obj-reference-preview.png'
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $Executable = Join-Path $workspacePath 'target/debug/sketch3d.exe'
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$null = New-Item -ItemType Directory -Path $verificationPath -Force
foreach ($output in @($reportPath, $previewPath)) {
    if (Test-Path -LiteralPath $output -PathType Leaf) {
        Remove-Item -LiteralPath $output
    }
}

$process = $null
try {
    $process = Start-Process `
        -FilePath $Executable `
        -ArgumentList '--verify-obj-references' `
        -WorkingDirectory $workspacePath `
        -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $verificationPath 'obj-reference.stdout.log') `
        -RedirectStandardError (Join-Path $verificationPath 'obj-reference.stderr.log') `
        -PassThru

    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        throw "OBJ reference verification did not finish within $TimeoutSeconds seconds."
    }
    $process.Refresh()
    if ($null -ne $process.ExitCode -and $process.ExitCode -ne 0) {
        throw 'OBJ reference verification failed. See verification/obj-reference.stderr.log.'
    }
    if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $previewPath -PathType Leaf)) {
        throw 'OBJ reference verification did not produce its report and preview.'
    }

    $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
    $requiredChecks = @(
        'OBJ References window displays its import control',
        "synthetic pointer move, press, and release clicks the window's import control",
        'canceling the picker leaves the document, history, and dirty state unchanged and keeps the source fixture available',
        'the clicked control invokes the production import handler with an injected file-picker result',
        'the imported drawing-local bundle reloads',
        'imported model is registered with the scene renderer',
        'synthetic viewport selection picks the imported model',
        'selected-model bounds match the normalized OBJ bounds',
        'production Preview button creates detached line art',
        'Update button recomputes line art for the current camera',
        'Bake button creates ordinary strokes in one undo edit',
        'undo and redo remove and restore the baked batch',
        'removing the source model leaves baked strokes intact'
    )
    foreach ($check in $requiredChecks) {
        if ($report.checks -notcontains $check) {
            throw "OBJ reference report is missing check: $check"
        }
    }
    if ($report.passed -ne $true -or
        $report.native_file_picker_tested -ne $false -or
        $report.physical_tablet_tested -ne $false -or
        $report.model_count -ne 1 -or
        $report.triangle_count -ne 1 -or
        $report.stroke_count -le 0) {
        throw 'OBJ reference report has an invalid result or overstates native picker/tablet coverage.'
    }
    Write-Output 'OBJ reference window click and production import verification passed.'
    Write-Output 'Native file picker and physical tablet testing remain unverified.'
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
