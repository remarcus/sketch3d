# Extract the release app and verification support into a path with spaces,
# then exercise success and missing-prerequisite behavior through MCP stdio.
[CmdletBinding()]
param(
    [string]$Executable = 'target/release/sketch3d.exe',
    [string]$InstallParent = $env:TEMP
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [IO.Path]::IsPathRooted($Executable)) { $Executable = Join-Path $repoRoot $Executable }
$Executable = [IO.Path]::GetFullPath($Executable)
[IO.Directory]::CreateDirectory($InstallParent) | Out-Null
$installRoot = Join-Path $InstallParent ('Sketch3D installed package with spaces-' + [guid]::NewGuid().ToString('N'))
$scripts = Join-Path $installRoot 'scripts'
$source = Join-Path $installRoot 'src'
$results = Join-Path $installRoot 'verification reports'
[IO.Directory]::CreateDirectory($scripts) | Out-Null
[IO.Directory]::CreateDirectory($source) | Out-Null
[IO.Directory]::CreateDirectory($results) | Out-Null
$installedExe = Join-Path $installRoot ([IO.Path]::GetFileName($Executable))
Copy-Item -LiteralPath $Executable -Destination $installedExe
Copy-Item -LiteralPath (Join-Path $repoRoot 'scripts/run_verification.ps1') -Destination $scripts
Copy-Item -LiteralPath (Join-Path $repoRoot 'src/animation_check.rs'), (Join-Path $repoRoot 'src/still_export_check.rs'), (Join-Path $repoRoot 'src/glb_texture_roundtrip_check.rs') -Destination $source

function Start-Server([string]$Binary, [string]$ReportRoot, [string]$PathValue) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Binary
    $info.WorkingDirectory = Split-Path -Parent $Binary
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    if ($PathValue) { $info.Environment['PATH'] = $PathValue }
    $info.ArgumentList.Add('--mcp-verification-execution-server')
    $info.ArgumentList.Add($ReportRoot)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    if (-not $process.Start()) { throw 'Could not start the installed MCP server.' }
    return $process
}

function Send-Request($Process, [int]$Id, [string]$Method, $Params) {
    $request = @{ jsonrpc = '2.0'; id = $Id; method = $Method; params = $Params }
    $Process.StandardInput.WriteLine(($request | ConvertTo-Json -Compress -Depth 30))
    $Process.StandardInput.Flush()
    $line = $Process.StandardOutput.ReadLine()
    if ([string]::IsNullOrWhiteSpace($line)) { throw "MCP server closed before replying to $Method" }
    return $line | ConvertFrom-Json -AsHashtable
}

$server = Start-Server $installedExe $results $env:PATH
try {
    $init = Send-Request $server 1 'initialize' @{ protocolVersion = '2025-06-18'; capabilities = @{}; clientInfo = @{ name = 'installed-build-pilot'; version = '1.0' } }
    if ($init.error) { throw "Installed MCP initialize failed: $($init.error.message)" }
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $tools = Send-Request $server 2 'tools/list' @{}
    if (@($tools.result.tools | ForEach-Object name) -notcontains 'verification_run_check') { throw 'Installed package did not expose verification execution.' }
    $run = Send-Request $server 3 'tools/call' @{ name = 'verification_run_check'; arguments = @{ check_id = 'startup'; timeout_seconds = 60 } }
    $runData = $run.result.structuredContent
    if ($run.result.isError -or $runData.check.outcome -ne 'passed') {
        $log = Send-Request $server 4 'tools/call' @{ name = 'verification_read_artifact'; arguments = @{ run_id = $runData.run_id; check_id = 'startup'; path = 'startup.stderr.log' } }
        throw "Installed runner failed at $installRoot : $($log.result.content[0].text)"
    }
    $read = Send-Request $server 4 'tools/call' @{ name = 'verification_read_result'; arguments = @{ run_id = $runData.run_id } }
    if ($read.result.isError -or $read.result.structuredContent.results[0].outcome -ne 'passed') { throw 'Installed result could not be read back.' }
    $server.StandardInput.Close()
    $server.WaitForExit(5000) | Out-Null
} finally {
    if (-not $server.HasExited) { $server.Kill($true) }
    $server.Dispose()
}

# An installed binary with its support files present but pwsh removed from PATH
# returns an explicit unsupported result, rather than a failed check.
$missingPrereqReports = Join-Path $installRoot 'missing PowerShell reports'
[IO.Directory]::CreateDirectory($missingPrereqReports) | Out-Null
$server = Start-Server $installedExe $missingPrereqReports (Join-Path $env:SystemRoot 'System32')
try {
    $null = Send-Request $server 1 'initialize' @{ protocolVersion = '2025-06-18'; capabilities = @{}; clientInfo = @{ name = 'missing-prerequisite-pilot'; version = '1.0' } }
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $unsupported = Send-Request $server 2 'tools/call' @{ name = 'verification_run_check'; arguments = @{ check_id = 'startup'; timeout_seconds = 10 } }
    if ($unsupported.result.structuredContent.check.outcome -ne 'unsupported' -or $unsupported.result.structuredContent.check.failure_reason -notmatch 'PowerShell 7') { throw 'Missing PowerShell prerequisite was not reported as unsupported.' }
    $manifest = @{ installed_root = $installRoot; path_contains_spaces = $installRoot.Contains(' '); successful_run_id = $runData.run_id; successful_outcome = $runData.check.outcome; missing_pwsh_outcome = $unsupported.result.structuredContent.check.outcome; missing_pwsh_reason = $unsupported.result.structuredContent.check.failure_reason }
    $manifestPath = Join-Path $installRoot 'installed-mcp-pilot.json'
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $manifestPath -Encoding utf8
    Write-Output "Installed MCP pilot passed: $manifestPath"
} finally {
    if (-not $server.HasExited) { $server.StandardInput.Close(); $server.WaitForExit(5000) | Out-Null }
    $server.Dispose()
}
