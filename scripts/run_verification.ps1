#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$Executable,
    [string]$OutputDirectory = "verification/normalized",
    [ValidateSet('startup', 'save-reopen', 'export')]
    [string[]]$Check = @('startup', 'save-reopen', 'export'),
    [ValidateRange(5, 3600)]
    [int]$TimeoutSeconds = 180
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($Executable)) {
    $fileName = if ($IsWindows) { 'sketch3d.exe' } else { 'sketch3d' }
    $Executable = Join-Path $repoRoot "target/release/$fileName"
}
if (-not [IO.Path]::IsPathRooted($Executable)) {
    $Executable = Join-Path $repoRoot $Executable
}
$Executable = [IO.Path]::GetFullPath($Executable)
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw "Executable not found: $Executable. Build the release app first or pass -Executable."
}
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot $OutputDirectory
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$runDirectory = Join-Path $OutputDirectory $runId
[IO.Directory]::CreateDirectory($runDirectory) | Out-Null

function Get-RelativeArtifact([string]$Path) {
    $relative = [IO.Path]::GetRelativePath($runDirectory, [IO.Path]::GetFullPath($Path))
    return $relative.Replace('\', '/')
}

function Invoke-VerificationCheck([string]$Id, [string[]]$Arguments, [string[]]$Reports) {
    $stdoutPath = Join-Path $runDirectory "$Id.stdout.log"
    $stderrPath = Join-Path $runDirectory "$Id.stderr.log"
    $failureReason = $null
    $outcome = 'failed'
    $exitCode = $null
    $process = $null
    $stdout = ''
    $stderr = ''
    try {
        $command = $Executable
        $commandArguments = $Arguments
        if ($Id -eq 'startup' -and $IsLinux) {
            $xvfb = Get-Command 'xvfb-run' -ErrorAction SilentlyContinue
            if ($xvfb) {
                $command = $xvfb.Source
                $commandArguments = @('-a', $Executable) + $Arguments
            } elseif ([string]::IsNullOrWhiteSpace($env:DISPLAY) -and [string]::IsNullOrWhiteSpace($env:WAYLAND_DISPLAY)) {
                $outcome = 'unsupported'
                $failureReason = 'Linux startup verification needs a display or xvfb-run.'
                throw [System.PlatformNotSupportedException]::new($failureReason)
            }
        }
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $command
        $startInfo.WorkingDirectory = $repoRoot
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        foreach ($argument in $commandArguments) { $startInfo.ArgumentList.Add($argument) }
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        if (-not $process.Start()) { throw 'Process did not start.' }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $outcome = 'timed_out'
            $failureReason = "Timed out after $TimeoutSeconds seconds."
            try { $process.Kill($true) } catch { try { $process.Kill() } catch {} }
            $process.WaitForExit()
        } else {
            $process.WaitForExit()
            $exitCode = $process.ExitCode
            if ($exitCode -eq 0) {
                $outcome = 'passed'
            } else {
                $stderr = $stderrTask.GetAwaiter().GetResult()
                $stdout = $stdoutTask.GetAwaiter().GetResult()
                $combined = "$stderr`n$stdout"
                if ($combined -match '(?i)(no (suitable )?(gpu|graphics|vulkan|wgpu).*adapter|adapter.*(not found|unsupported)|could not create headless gpu renderer|unsupported environment|feature .* not supported)') {
                    $outcome = 'unsupported'
                    $failureReason = ($combined.Trim() -split "`r?`n" | Select-Object -Last 1)
                } else {
                    $outcome = 'failed'
                    $detail = if ([string]::IsNullOrWhiteSpace($stderr)) { $stdout } else { $stderr }
                    $failureReason = if ([string]::IsNullOrWhiteSpace($detail)) { "Verifier exited with code $exitCode." } else { ($detail.Trim() -split "`r?`n" | Select-Object -Last 1) }
                }
            }
        }
        if ($process -and $process.HasExited) {
            if ([string]::IsNullOrEmpty($stdout)) { $stdout = $stdoutTask.GetAwaiter().GetResult() }
            if ([string]::IsNullOrEmpty($stderr)) { $stderr = $stderrTask.GetAwaiter().GetResult() }
        }
    } catch {
        if ($outcome -ne 'unsupported') {
            $failureReason = $_.Exception.Message
            $outcome = 'failed'
        }
    } finally {
        if ($process) { $process.Dispose() }
        [IO.File]::WriteAllText($stdoutPath, $stdout, [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($stderrPath, $stderr, [Text.UTF8Encoding]::new($false))
    }
    if ($outcome -eq 'passed' -and $Reports.Count -gt 0 -and -not (Test-Path -LiteralPath $Reports[0] -PathType Leaf)) {
        $outcome = 'failed'
        $failureReason = 'Verifier exited successfully but did not produce its original report.'
    }
    $artifacts = [Collections.Generic.List[string]]::new()
    foreach ($path in @($stdoutPath, $stderrPath) + $Reports) {
        if (Test-Path -LiteralPath $path -PathType Leaf) { $artifacts.Add((Get-RelativeArtifact $path)) }
    }
    return [ordered]@{
        check_id = $Id
        outcome = $outcome
        failure_reason = $failureReason
        exit_code = $exitCode
        artifacts = @($artifacts)
    }
}

$results = [Collections.Generic.List[object]]::new()
foreach ($checkId in $Check) {
    $reportDirectory = Join-Path $runDirectory "$checkId-original"
    [IO.Directory]::CreateDirectory($reportDirectory) | Out-Null
    switch ($checkId) {
        'startup' {
            $results.Add((Invoke-VerificationCheck 'startup' @('--verify') @()))
        }
        'save-reopen' {
            $report = Join-Path $reportDirectory 'artwork-animation-report.json'
            $results.Add((Invoke-VerificationCheck 'save-reopen' @('--verify-artwork-animation', $report) @($report)))
        }
        'export' {
            $results.Add((Invoke-VerificationCheck 'export' @('--verify-still-export', $reportDirectory) @((Join-Path $reportDirectory 'still-export-report.json'), (Join-Path $reportDirectory 'transparent-edge.png'), (Join-Path $reportDirectory 'jpeg-canvas.jpg'))))
        }
    }
}

$summary = [ordered]@{
    schema_version = 1
    run_id = $runId
    generated_utc = [DateTime]::UtcNow.ToString('o')
    results = @($results)
}
$summaryPath = Join-Path $runDirectory 'result.json'
$summary | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $summaryPath -Encoding utf8
Write-Output "Normalized result: $summaryPath"
$results | ForEach-Object { Write-Output ("{0}: {1}" -f $_.check_id, $_.outcome) }
if ($results.Where({ $_.outcome -in @('failed', 'timed_out') }).Count -gt 0) { exit 1 }
