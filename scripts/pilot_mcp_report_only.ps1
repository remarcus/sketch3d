# Exercise report-only MCP discovery, result, and artifact reads with a stdio client.
[CmdletBinding()]
param(
    [string]$Executable = 'target/release/sketch3d.exe',
    [string]$ResultsDirectory = 'verification/verification-tooling-suite',
    [string]$RunId = '20261105T181238Z-ee0d05b0'
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [IO.Path]::IsPathRooted($Executable)) { $Executable = Join-Path $repoRoot $Executable }
if (-not [IO.Path]::IsPathRooted($ResultsDirectory)) { $ResultsDirectory = Join-Path $repoRoot $ResultsDirectory }
$Executable = [IO.Path]::GetFullPath($Executable)
$ResultsDirectory = [IO.Path]::GetFullPath($ResultsDirectory)
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { throw "MCP server executable not found: $Executable" }
if (-not (Test-Path -LiteralPath (Join-Path $ResultsDirectory $RunId) -PathType Container)) { throw "Verification run not found: $RunId" }

$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $Executable
$start.WorkingDirectory = $repoRoot
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardInput = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.ArgumentList.Add('--mcp-verification-server')
$start.ArgumentList.Add($ResultsDirectory)
$server = [Diagnostics.Process]::new()
$server.StartInfo = $start
if (-not $server.Start()) { throw 'Could not start report-only MCP server.' }
$transcript = [Collections.Generic.List[object]]::new()
$rpcId = 0
function Invoke-McpRequest([string]$Method, [object]$Params) {
    $script:rpcId++
    $request = [ordered]@{ jsonrpc = '2.0'; id = $script:rpcId; method = $Method; params = $Params }
    $server.StandardInput.WriteLine(($request | ConvertTo-Json -Compress -Depth 50))
    $server.StandardInput.Flush()
    $line = $server.StandardOutput.ReadLine()
    if ([string]::IsNullOrWhiteSpace($line)) {
        if ($server.HasExited) { $server.WaitForExit(); $stderr = $server.StandardError.ReadToEnd() } else { $stderr = 'No response before server exit.' }
        throw "MCP server closed without a response to $Method. $stderr"
    }
    $response = $line | ConvertFrom-Json -AsHashtable
    $transcript.Add([ordered]@{ request = $request; response = $response })
    return $response
}
try {
    $initialized = Invoke-McpRequest 'initialize' ([ordered]@{ protocolVersion = '2025-06-18'; capabilities = @{}; clientInfo = @{ name = 'sketch3d-report-only-pilot'; version = '1.0' } })
    if ($initialized.error) { throw "MCP initialize failed: $($initialized.error.message)" }
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $server.StandardInput.Flush()
    $listed = Invoke-McpRequest 'tools/list' @{}
    $toolNames = @($listed.result.tools | ForEach-Object name)
    if ($listed.error -or $toolNames -notcontains 'verification_list_checks' -or $toolNames -contains 'verification_run_check') {
        throw "Report-only tool set was incorrect: $($toolNames -join ', ')"
    }
    $catalog = Invoke-McpRequest 'tools/call' @{ name = 'verification_list_checks'; arguments = @{} }
    if ($catalog.result.isError -or -not ($catalog.result.structuredContent.completed_runs -contains $RunId)) {
        throw 'Report-only catalog omitted the selected completed run.'
    }
    $result = Invoke-McpRequest 'tools/call' @{ name = 'verification_read_result'; arguments = @{ run_id = $RunId } }
    if ($result.result.isError -or $result.result.structuredContent.results.Count -ne 3) {
        throw 'Report-only result read did not return the three completed checks.'
    }
    $artifacts = Invoke-McpRequest 'tools/call' @{ name = 'verification_list_artifacts'; arguments = @{ run_id = $RunId; check_id = 'export' } }
    $log = $artifacts.result.structuredContent.artifacts | Where-Object { $_.path -eq 'export.stdout.log' } | Select-Object -First 1
    if ($artifacts.result.isError -or -not $log) { throw 'Report-only artifact list omitted the export log.' }
    $readLog = Invoke-McpRequest 'tools/call' @{ name = 'verification_read_artifact'; arguments = @{ run_id = $RunId; check_id = 'export'; path = $log.path } }
    if ($readLog.result.isError -or $readLog.result.content[0].type -ne 'text') { throw 'Report-only artifact read did not return text.' }
    $disabled = Invoke-McpRequest 'tools/call' @{ name = 'verification_run_check'; arguments = @{ check_id = 'startup'; timeout_seconds = 5 } }
    if (-not $disabled.result.isError -or $disabled.result.content[0].text -notmatch 'execution is disabled') {
        throw 'Report-only mode did not reject process execution.'
    }
    $manifest = [ordered]@{
        pilot = 'report-only-stdio-client'
        protocol_version = $initialized.result.protocolVersion
        tools = $toolNames
        listed_run = $RunId
        result_checks = @($result.result.structuredContent.results | ForEach-Object { @{ check_id = $_.check_id; outcome = $_.outcome } })
        artifact_read = $log.path
        execution_tool_absent = $true
        direct_execution_rejected = $true
        transcript = $transcript
    }
    $path = Join-Path (Join-Path $ResultsDirectory $RunId) 'report-only-mcp-client-transcript.json'
    $manifest | ConvertTo-Json -Depth 60 | Set-Content -LiteralPath $path -Encoding utf8
    Write-Output "Report-only MCP pilot passed: $RunId"
    Write-Output "Transcript: $path"
} finally {
    if ($server -and -not $server.HasExited) { $server.StandardInput.Close(); $server.WaitForExit(5000) | Out-Null }
    if ($server) { $server.Dispose() }
}
