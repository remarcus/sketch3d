#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$Executable,
    [string]$OutputDirectory = "verification/normalized",
    [ValidateSet('startup', 'save-reopen', 'export', 'glb-texture-roundtrip')]
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

$buildVersion = $null
$buildFileVersion = $null
$buildProductVersion = $null
$buildFingerprint = $null
$buildFingerprintStatus = 'unavailable'
try {
    $fileVersionInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($Executable)
    $buildFileVersion = if ([string]::IsNullOrWhiteSpace($fileVersionInfo.FileVersion)) { $null } else { $fileVersionInfo.FileVersion }
    $buildProductVersion = if ([string]::IsNullOrWhiteSpace($fileVersionInfo.ProductVersion)) { $null } else { $fileVersionInfo.ProductVersion }
    $buildVersion = if ($buildProductVersion) { $buildProductVersion } else { $buildFileVersion }
} catch {}
try {
    $buildFingerprint = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
    $buildFingerprintStatus = 'available'
} catch {}
$buildIdentity = [ordered]@{
    version = $buildVersion
    version_status = if ($buildVersion) { 'available' } else { 'unavailable' }
    file_version = $buildFileVersion
    product_version = $buildProductVersion
    fingerprint_algorithm = 'sha256'
    fingerprint = $buildFingerprint
    fingerprint_status = $buildFingerprintStatus
}

$sourceRevision = $null
$sourceRevisionStatus = 'unavailable'
$gitCommand = Get-Command 'git' -ErrorAction SilentlyContinue
if ($gitCommand) {
    try {
        $revisionOutput = & $gitCommand.Source -C $repoRoot rev-parse --verify HEAD 2>$null
        $revision = ([string]$revisionOutput).Trim()
        if ($LASTEXITCODE -eq 0 -and $revision -match '^[0-9a-fA-F]{40,64}$') {
            $sourceRevision = $revision.ToLowerInvariant()
            $sourceRevisionStatus = 'available'
        }
    } catch {}
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

function Get-FixtureIdentity([string]$Id) {
    $fixtureId = $null
    $sourceRelativePath = $null
    switch ($Id) {
        'startup' { $fixtureId = 'startup-smoke-v1' }
        'save-reopen' {
            $fixtureId = 'artwork-animation-roundtrip-v1'
            $sourceRelativePath = 'src/animation_check.rs'
        }
        'export' {
            $fixtureId = 'still-export-visual-v1'
            $sourceRelativePath = 'src/still_export_check.rs'
        }
        'glb-texture-roundtrip' {
            $fixtureId = 'textured-glb-managed-native-roundtrip-v1'
            $sourceRelativePath = 'src/glb_texture_roundtrip_check.rs'
        }
    }
    $fingerprint = $null
    $fingerprintStatus = 'not_applicable'
    if ($sourceRelativePath) {
        $fingerprintStatus = 'unavailable'
        $fixtureSource = Join-Path $repoRoot ($sourceRelativePath.Replace('/', [IO.Path]::DirectorySeparatorChar))
        if (Test-Path -LiteralPath $fixtureSource -PathType Leaf) {
            try {
                $fingerprint = (Get-FileHash -LiteralPath $fixtureSource -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
                $fingerprintStatus = 'available'
            } catch {}
        }
    }
    return [ordered]@{
        id = $fixtureId
        fingerprint_algorithm = 'sha256'
        fingerprint = $fingerprint
        fingerprint_status = $fingerprintStatus
    }
}

function Get-RenderingMetadata([string]$Id, [string[]]$Reports) {
    if ($Id -eq 'glb-texture-roundtrip') {
        return [ordered]@{ status = 'not_run'; reason = 'Interactive viewport rendering is tracked separately from this structural check.' }
    }
    if ($Id -ne 'export') { return $null }
    $reportPath = $Reports | Where-Object { $_.EndsWith('still-export-report.json', [StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1
    if (-not $reportPath -or -not (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
        return [ordered]@{ status = 'unavailable' }
    }
    try {
        $report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json -ErrorAction Stop
        $size = @($report.size)
        if ($size.Count -eq 2 -and [int]$size[0] -gt 0 -and [int]$size[1] -gt 0) {
            return [ordered]@{ status = 'available'; resolution_px = @([int]$size[0], [int]$size[1]) }
        }
    } catch {}
    return [ordered]@{ status = 'unavailable' }
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
                if ($combined -match '(?i)(no (suitable )?(gpu|graphics|vulkan|wgpu).*adapter|adapter.*(not found|unsupported)|no suitable adapter found|NoSuitableAdapterFound|FailedToCreateSurfaceForAnyBackend|CreateSurfaceError|could not create headless gpu renderer|unsupported environment|feature .* not supported)') {
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
        fixture = Get-FixtureIdentity $Id
        rendering = Get-RenderingMetadata $Id $Reports
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
        'glb-texture-roundtrip' {
            $report = Join-Path $reportDirectory 'glb-texture-roundtrip-report.json'
            $results.Add((Invoke-VerificationCheck 'glb-texture-roundtrip' @('--verify-glb-texture-roundtrip', $reportDirectory) @($report, (Join-Path $reportDirectory 'texture-preview.png'), (Join-Path $reportDirectory 'roundtrip.sketch.json'), (Join-Path $reportDirectory 'whole-scene.glb'))))
        }
    }
}

$summary = [ordered]@{
    schema_version = 1
    run_id = $runId
    generated_utc = [DateTime]::UtcNow.ToString('o')
    build = $buildIdentity
    source_revision = [ordered]@{ value = $sourceRevision; status = $sourceRevisionStatus }
    results = @($results)
}
$summaryPath = Join-Path $runDirectory 'result.json'
$summary | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $summaryPath -Encoding utf8
Write-Output "Normalized result: $summaryPath"
$results | ForEach-Object { Write-Output ("{0}: {1}" -f $_.check_id, $_.outcome) }
if ($results.Where({ $_.outcome -in @('failed', 'timed_out') }).Count -gt 0) { exit 1 }
