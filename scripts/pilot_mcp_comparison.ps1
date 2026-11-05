# Exercise baseline comparison, changed metrics, MCP image return, and the
# unsupported-fixture path using completed export reports.
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
if (-not (Test-Path -LiteralPath (Join-Path $source 'result.json') -PathType Leaf)) { throw "Source result not found: $SourceRunId" }
$suffix = [guid]::NewGuid().ToString('N').Substring(0, 8)
$baselineId = "compare-base-$suffix"
$currentId = "compare-current-$suffix"
$unsupportedId = "compare-unsupported-$suffix"

function Copy-ComparisonRun([string]$RunId) {
    $destination = Join-Path $VerificationRoot $RunId
    Copy-Item -LiteralPath $source -Destination $destination -Recurse
    $resultPath = Join-Path $destination 'result.json'
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json -AsHashtable
    $result.run_id = $RunId
    $result | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $resultPath -Encoding utf8
    return $destination
}

$null = Copy-ComparisonRun $baselineId
$current = Copy-ComparisonRun $currentId
$currentResultPath = Join-Path $current 'result.json'
$currentResult = Get-Content -LiteralPath $currentResultPath -Raw | ConvertFrom-Json -AsHashtable
$export = $currentResult.results | Where-Object check_id -eq 'export' | Select-Object -First 1
$export.outcome = 'failed'
$export.failure_reason = 'Synthetic changed-outcome comparison pilot.'
$currentResult | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $currentResultPath -Encoding utf8
$reportPath = Join-Path $current 'export-original/still-export-report.json'
$report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json -AsHashtable
$report.transparent_png.maximum_straight_color_channel_error = [int]$report.transparent_png.maximum_straight_color_channel_error + 1
$report | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $reportPath -Encoding utf8
$pngPath = Join-Path $current 'export-original/transparent-edge.png'
Add-Type -AssemblyName System.Drawing
$bitmap = [Drawing.Bitmap]::new($pngPath)
$memory = [IO.MemoryStream]::new()
$bitmap.SetPixel(0, 0, [Drawing.Color]::FromArgb(255, 255, 0, 255))
$bitmap.Save($memory, [Drawing.Imaging.ImageFormat]::Png)
$bitmap.Dispose()
[IO.File]::WriteAllBytes($pngPath, $memory.ToArray())
$memory.Dispose()

$unsupported = Copy-ComparisonRun $unsupportedId
$unsupportedResultPath = Join-Path $unsupported 'result.json'
$unsupportedResult = Get-Content -LiteralPath $unsupportedResultPath -Raw | ConvertFrom-Json -AsHashtable
($unsupportedResult.results | Where-Object check_id -eq 'export' | Select-Object -First 1).fixture.fingerprint = ('a' * 64)
$unsupportedResult | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $unsupportedResultPath -Encoding utf8

$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $Executable
$start.WorkingDirectory = $repoRoot
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardInput = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.ArgumentList.Add('--mcp-verification-server')
$start.ArgumentList.Add($VerificationRoot)
$server = [Diagnostics.Process]::new()
$server.StartInfo = $start
if (-not $server.Start()) { throw 'Could not start report-only MCP server.' }
$rpcId = 0
function Invoke-Mcp([string]$Method, $Params) {
    $script:rpcId++
    $request = @{ jsonrpc = '2.0'; id = $script:rpcId; method = $Method; params = $Params }
    $server.StandardInput.WriteLine(($request | ConvertTo-Json -Compress -Depth 20))
    $server.StandardInput.Flush()
    $line = $server.StandardOutput.ReadLine()
    if ([string]::IsNullOrWhiteSpace($line)) { throw "MCP server closed before replying to $Method" }
    return $line | ConvertFrom-Json -AsHashtable
}
try {
    $null = Invoke-Mcp 'initialize' @{ protocolVersion = '2025-06-18'; capabilities = @{}; clientInfo = @{ name = 'baseline-comparison-pilot'; version = '1.0' } }
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $tools = Invoke-Mcp 'tools/list' @{}
    if (@($tools.result.tools | ForEach-Object name) -notcontains 'verification_compare_runs') { throw 'MCP did not expose verification_compare_runs.' }
    $response = Invoke-Mcp 'tools/call' @{ name = 'verification_compare_runs'; arguments = @{ baseline_run_id = $baselineId; current_run_id = $currentId; check_id = 'export' } }
    $comparison = $response.result.structuredContent
    $image = $response.result.content | Where-Object type -eq 'image' | Select-Object -First 1
    if ($response.result.isError -or -not $comparison.regression -or $comparison.regression_status -ne 'compared' -or $comparison.outcomes.changed -ne $true -or $comparison.metrics.status -ne 'compared' -or $comparison.metrics.changed.Count -eq 0 -or $comparison.visual.status -ne 'compared' -or $comparison.visual.changed_pixels -lt 1 -or -not $image -or $image.mimeType -ne 'image/png') {
        throw "Regression/metric/PNG comparison failed: $($response.result.content[0].text)"
    }
    $unsupportedResponse = Invoke-Mcp 'tools/call' @{ name = 'verification_compare_runs'; arguments = @{ baseline_run_id = $baselineId; current_run_id = $unsupportedId; check_id = 'export' } }
    $unsupportedComparison = $unsupportedResponse.result.structuredContent
    if ($unsupportedResponse.result.isError -or $unsupportedComparison.comparison_status -ne 'unsupported' -or $unsupportedComparison.regression_status -ne 'unsupported' -or $unsupportedComparison.regression -ne $null -or $unsupportedComparison.visual.status -ne 'unsupported' -or @($unsupportedResponse.result.content | Where-Object type -eq 'image').Count -ne 0) {
        throw 'Mismatched fixture identity was not reported as an unsupported comparison.'
    }
    [IO.File]::WriteAllBytes((Join-Path $VerificationRoot "$currentId-diff.png"), [Convert]::FromBase64String($image.data))
    $pilot = @{ baseline_run_id = $baselineId; current_run_id = $currentId; regression = $comparison.regression; changed_metrics = $comparison.metrics.changed; visual = $comparison.visual; unsupported_fixture_run_id = $unsupportedId; unsupported_status = $unsupportedComparison.comparison_status; diff_image = "$currentId-diff.png" }
    $pilot | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath (Join-Path $VerificationRoot "$currentId-comparison-pilot.json") -Encoding utf8
    Write-Output "MCP comparison pilot passed: $baselineId -> $currentId"
} finally {
    if ($server -and -not $server.HasExited) { $server.StandardInput.Close(); $server.WaitForExit(5000) | Out-Null }
    if ($server) { $server.Dispose() }
}
