# Exercise bounded CI ZIP import, platform grouping and original MCP artifacts.
[CmdletBinding()]
param(
    [string]$Executable = 'target/release/sketch3d.exe',
    [string]$VerificationRoot = 'verification',
    [string]$SourceRunId = '20261105T181238Z-ee0d05b0'
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [IO.Path]::IsPathRooted($Executable)) { $Executable = Join-Path $repoRoot $Executable }
if (-not [IO.Path]::IsPathRooted($VerificationRoot)) { $VerificationRoot = Join-Path $repoRoot $VerificationRoot }
$Executable = [IO.Path]::GetFullPath($Executable)
$VerificationRoot = [IO.Path]::GetFullPath($VerificationRoot)
$source = Join-Path (Join-Path $VerificationRoot 'verification-tooling-suite') $SourceRunId
$sourceResultPath = Join-Path $source 'result.json'
if (-not (Test-Path -LiteralPath $sourceResultPath -PathType Leaf)) { throw "Source result not found: $SourceRunId" }
$baseline = Get-Content -LiteralPath $sourceResultPath -Raw | ConvertFrom-Json -AsHashtable
$check = $baseline.results | Where-Object check_id -eq 'export' | Select-Object -First 1
if (-not $check) { throw 'Source run has no export check.' }
$revision = [string]$baseline.source_revision.value
$fixture = [string]$check.fixture.id
$pilotRoot = Join-Path $VerificationRoot "ci-import-pilot-$([guid]::NewGuid().ToString('N').Substring(0,8))"
[IO.Directory]::CreateDirectory($pilotRoot) | Out-Null
$importRoot = Join-Path $pilotRoot 'imported'
[IO.Directory]::CreateDirectory($importRoot) | Out-Null
$platformRuns = @{}
foreach ($platform in @('linux', 'macos', 'windows')) {
    $stage = Join-Path $pilotRoot "stage-$platform"
    Copy-Item -LiteralPath $source -Destination $stage -Recurse
    $resultPath = Join-Path $stage 'result.json'
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json -AsHashtable
    $export = $result.results | Where-Object check_id -eq 'export' | Select-Object -First 1
    $log = Join-Path $stage 'export.stderr.log'
    if ($platform -eq 'windows') { $export.outcome = 'passed'; $export.failure_reason = $null; Set-Content -LiteralPath $log -Value '' -NoNewline }
    else { $export.outcome = 'failed'; $export.failure_reason = "Synthetic $platform export failure"; Set-Content -LiteralPath $log -Value "Synthetic original $platform export error log." }
    $result | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath $resultPath -Encoding utf8
    $archive = Join-Path $pilotRoot "$platform.zip"
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -CompressionLevel Optimal
    $ciId = "pilot-$platform"
    & pwsh -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'import_ci_verification.ps1') -ArchivePath $archive -OutputDirectory $importRoot -Platform $platform -CiRunId $ciId -Attempt 1 | Out-Null
    $importLine = Get-ChildItem -LiteralPath $importRoot -Directory | Where-Object Name -like "ci-$ciId-1-$platform-*" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $importLine) { throw "Importer did not create a visible normalized run for $platform." }
    $platformRuns[$platform] = $importLine.Name
}
$unsafeZip = Join-Path $pilotRoot 'unsafe-path.zip'
$badArchive = [IO.Compression.ZipFile]::Open($unsafeZip, [IO.Compression.ZipArchiveMode]::Create)
try { $badEntry = $badArchive.CreateEntry('../escape.txt'); $writer = [IO.StreamWriter]::new($badEntry.Open()); try { $writer.Write('unsafe') } finally { $writer.Dispose() } } finally { $badArchive.Dispose() }
$unsafeOutput = & pwsh -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'import_ci_verification.ps1') -ArchivePath $unsafeZip -OutputDirectory (Join-Path $pilotRoot 'unsafe-output') -Platform linux -CiRunId unsafe 2>&1
if ($LASTEXITCODE -eq 0 -or (Test-Path -LiteralPath (Join-Path $pilotRoot 'unsafe-output') -PathType Container)) { throw 'Importer accepted an unsafe ZIP path or created output before validation.' }

$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $Executable; $start.WorkingDirectory = $repoRoot; $start.UseShellExecute = $false; $start.CreateNoWindow = $true
$start.RedirectStandardInput = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
$start.ArgumentList.Add('--mcp-verification-server'); $start.ArgumentList.Add($importRoot)
$server = [Diagnostics.Process]::new(); $server.StartInfo = $start
if (-not $server.Start()) { throw 'Could not start the report-only MCP server.' }
$rpcId = 0
function Invoke-Mcp([string]$Method, $Params) {
    $script:rpcId++
    $server.StandardInput.WriteLine((@{jsonrpc='2.0';id=$script:rpcId;method=$Method;params=$Params} | ConvertTo-Json -Compress -Depth 20))
    $server.StandardInput.Flush()
    $line = $server.StandardOutput.ReadLine()
    if ([string]::IsNullOrWhiteSpace($line)) { throw "MCP server closed before replying to $Method" }
    return $line | ConvertFrom-Json -AsHashtable
}
try {
    $null = Invoke-Mcp 'initialize' @{protocolVersion='2025-06-18';capabilities=@{};clientInfo=@{name='ci-import-pilot';version='1.0'}}
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $summary = Invoke-Mcp 'tools/call' @{name='verification_ci_summary';arguments=@{source_revision=$revision;fixture_id=$fixture;check_id='export'}}
    $groups = $summary.result.structuredContent.platform_outcomes
    if ($summary.result.isError -or $groups.linux.failed -ne 1 -or $groups.macos.failed -ne 1 -or $groups.windows.passed -ne 1) { throw "CI summary conflated outcomes: $($summary.result.content[0].text)" }
    $readLog = Invoke-Mcp 'tools/call' @{name='verification_read_artifact';arguments=@{run_id=$platformRuns.linux;check_id='export';path='export.stderr.log'}}
    if ($readLog.result.isError -or $readLog.result.structuredContent.artifact_type -ne 'text/plain' -or -not ($readLog.result.content[0].text.Contains('Synthetic original linux export error log.'))) { throw 'Original Linux error log was not readable over MCP.' }
    $readPng = Invoke-Mcp 'tools/call' @{name='verification_read_artifact';arguments=@{run_id=$platformRuns.windows;check_id='export';path='export-original/transparent-edge.png'}}
    if ($readPng.result.isError -or $readPng.result.structuredContent.artifact_type -ne 'image/png' -or -not ($readPng.result.content | Where-Object type -eq 'image')) { throw "Original Windows PNG was not readable over MCP: $($readPng.result | ConvertTo-Json -Depth 5 -Compress)" }
    $pilot = [ordered]@{source_revision=$revision;fixture_id=$fixture;check_id='export';platform_outcomes=$groups;run_ids=$platformRuns;linux_error_log_readable=$true;windows_png_readable=$true}
    $pilot | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $pilotRoot 'ci-import-pilot.json') -Encoding utf8
    Write-Output "CI importer pilot passed: Linux/macOS failures and Windows pass remain distinct; log and PNG retrieved through MCP."
} finally {
    if ($server -and -not $server.HasExited) { $server.StandardInput.Close(); $server.WaitForExit(5000) | Out-Null }
    if ($server) { $server.Dispose() }
}
