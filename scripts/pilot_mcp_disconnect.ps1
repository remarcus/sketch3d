# Confirm that closing an MCP stdio client cancels an active verification child.
[CmdletBinding()]
param(
    [string]$Executable = 'target/release/sketch3d.exe',
    [string]$OutputDirectory = 'verification/mcp-disconnect-pilot'
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
try {
    $initialize = @{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = @{ protocolVersion = '2025-06-18'; capabilities = @{}; clientInfo = @{ name = 'disconnect-pilot'; version = '1.0' } } }
    $server.StandardInput.WriteLine(($initialize | ConvertTo-Json -Compress -Depth 8))
    $server.StandardInput.Flush()
    $response = $server.StandardOutput.ReadLine() | ConvertFrom-Json
    if ($response.error) { throw "MCP initialization failed: $($response.error.message)" }
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $request = @{ jsonrpc = '2.0'; id = 2; method = 'tools/call'; params = @{ name = 'verification_run_check'; arguments = @{ check_id = 'startup'; timeout_seconds = 600 } } }
    $server.StandardInput.WriteLine(($request | ConvertTo-Json -Compress -Depth 12))
    $server.StandardInput.Flush()
    $server.StandardInput.Close()
    if (-not $server.WaitForExit(15000)) {
        $server.Kill($true)
        throw 'MCP server did not stop promptly after the client disconnected.'
    }
    $staleOutput = @(Get-ChildItem -LiteralPath $OutputDirectory -Force -ErrorAction SilentlyContinue)
    if ($staleOutput.Count -gt 0) { throw 'Disconnect pilot left a partial verification run directory behind.' }
    Write-Output 'MCP disconnect cancellation passed: server and verifier process tree stopped.'
} finally {
    if ($server -and -not $server.HasExited) { $server.Kill($true) }
    if ($server) { $server.Dispose() }
}
