# Exercise UI-workflow discovery and normalized reporting through a real MCP
# stdio client. Headless/foreground-locked desktops must remain unsupported.
[CmdletBinding()]
param(
    [string]$Executable = 'target/release/sketch3d.exe',
    [string]$OutputDirectory = 'verification/ui-workflow-mcp-pilot'
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [IO.Path]::IsPathRooted($Executable)) { $Executable = Join-Path $repoRoot $Executable }
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot $OutputDirectory }
$Executable = [IO.Path]::GetFullPath($Executable)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $Executable
$start.WorkingDirectory = $repoRoot
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardInput = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.ArgumentList.Add('--mcp-verification-execution-server')
$start.ArgumentList.Add($OutputDirectory)
$server = [Diagnostics.Process]::new()
$server.StartInfo = $start
if (-not $server.Start()) { throw 'Could not start MCP execution server.' }
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
    $null = Invoke-Mcp 'initialize' @{ protocolVersion = '2025-06-18'; capabilities = @{}; clientInfo = @{ name = 'ui-workflow-pilot'; version = '1.0' } }
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $tools = Invoke-Mcp 'tools/list' @{}
    $runTool = $tools.result.tools | Where-Object name -eq 'verification_run_check' | Select-Object -First 1
    if (-not $runTool -or $runTool.inputSchema.properties.check_id.enum -notcontains 'ui-workflow') { throw 'MCP did not expose the UI workflow check.' }
    $run = Invoke-Mcp 'tools/call' @{ name = 'verification_run_check'; arguments = @{ check_id = 'ui-workflow'; timeout_seconds = 90 } }
    $runData = $run.result.structuredContent
    if ($run.result.isError -or $runData.check.outcome -notin @('passed', 'unsupported')) { throw "MCP returned an invalid UI workflow outcome: $($run.result.content[0].text)" }
    $read = Invoke-Mcp 'tools/call' @{ name = 'verification_read_result'; arguments = @{ run_id = $runData.run_id } }
    if ($read.result.isError -or $read.result.structuredContent.results[0].outcome -ne $runData.check.outcome) { throw 'MCP normalized result disagrees with the UI workflow outcome.' }
    $artifacts = Invoke-Mcp 'tools/call' @{ name = 'verification_list_artifacts'; arguments = @{ run_id = $runData.run_id; check_id = 'ui-workflow' } }
    if ($artifacts.result.isError -or -not ($artifacts.result.structuredContent.artifacts.path -contains 'ui-workflow-original/ui-workflow-report.json')) { throw 'MCP did not list the UI workflow report artifact.' }
    $pilot = @{ run_id = $runData.run_id; outcome = $runData.check.outcome; failure_reason = $runData.check.failure_reason; artifacts = $artifacts.result.structuredContent.artifacts; client = 'stdio'; screenshot_available = ($runData.check.artifacts -contains 'ui-workflow-original/ui-workflow.png') }
    $pilot | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'ui-workflow-mcp-pilot.json') -Encoding utf8
    Write-Output "MCP UI workflow pilot recorded: $($runData.check.outcome) ($($runData.run_id))"
} finally {
    if ($server -and -not $server.HasExited) { $server.StandardInput.Close(); $server.WaitForExit(5000) | Out-Null }
    if ($server) { $server.Dispose() }
}
