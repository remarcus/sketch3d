# Runs the release verifier through a stdio MCP client session and records the
# client-visible tool results. The server process is the same release binary
# used by scripts/run_verification.ps1.
[CmdletBinding()]
param(
    [string]$Executable = 'target/release/sketch3d.exe',
    [string]$OutputDirectory = 'verification/normalized-mcp',
    [string]$CheckId = 'glb-texture-roundtrip'
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [IO.Path]::IsPathRooted($Executable)) { $Executable = Join-Path $repoRoot $Executable }
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot $OutputDirectory }
$Executable = [IO.Path]::GetFullPath($Executable)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { throw "MCP server executable not found: $Executable" }
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
if (-not $server.Start()) { throw 'Could not start local MCP server.' }
$transcript = [Collections.Generic.List[object]]::new()
$rpcId = 0
function Invoke-McpRequest([string]$Method, [object]$Params) {
    $script:rpcId++
    $request = [ordered]@{ jsonrpc = '2.0'; id = $script:rpcId; method = $Method; params = $Params }
    $server.StandardInput.WriteLine(($request | ConvertTo-Json -Compress -Depth 40))
    $server.StandardInput.Flush()
    $line = $server.StandardOutput.ReadLine()
    if ([string]::IsNullOrWhiteSpace($line)) {
        $stderr = $server.StandardError.ReadToEnd()
        throw "MCP server closed without a response to $Method. $stderr"
    }
    $response = $line | ConvertFrom-Json -AsHashtable
    $transcript.Add([ordered]@{ request = $request; response = $response })
    return $response
}
try {
    $initialized = Invoke-McpRequest 'initialize' ([ordered]@{ protocolVersion = '2025-06-18'; capabilities = @{}; clientInfo = @{ name = 'sketch3d-release-review-pilot'; version = '1.0' } })
    if ($initialized.error) { throw "MCP initialize failed: $($initialized.error.message)" }
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $server.StandardInput.Flush()
    $listed = Invoke-McpRequest 'tools/list' @{}
    $checkTool = $listed.result.tools | Where-Object name -eq 'verification_run_check'
    if (-not $checkTool -or $checkTool.inputSchema.properties.check_id.enum -notcontains $CheckId) { throw "MCP client could not discover allowlisted check '$CheckId'." }
    $checkCatalog = Invoke-McpRequest 'tools/call' @{ name = 'verification_list_checks'; arguments = @{} }
    $catalogEntry = $checkCatalog.result.structuredContent.checks | Where-Object check_id -eq $CheckId | Select-Object -First 1
    if ($checkCatalog.result.isError -or -not $catalogEntry) { throw "MCP client check catalog did not list '$CheckId': $($checkCatalog.result.content[0].text)" }
    $runResponse = Invoke-McpRequest 'tools/call' @{ name = 'verification_run_check'; arguments = @{ check_id = $CheckId; timeout_seconds = 180 } }
    $runPayload = $runResponse.result.structuredContent
    if ($runResponse.result.isError -or $runPayload.check.outcome -ne 'passed') { throw "MCP verifier failed: $($runResponse.result.content[0].text)" }
    $runId = $runPayload.run_id
    $readResult = Invoke-McpRequest 'tools/call' @{ name = 'verification_read_result'; arguments = @{ run_id = $runId } }
    if ($readResult.result.isError -or $readResult.result.structuredContent.results[0].outcome -ne 'passed') { throw 'MCP result retrieval did not return a passed check.' }
    $artifactResponse = Invoke-McpRequest 'tools/call' @{ name = 'verification_list_artifacts'; arguments = @{ run_id = $runId; check_id = $CheckId } }
    $artifacts = $artifactResponse.result.structuredContent.artifacts
    $logArtifact = $artifacts | Where-Object { $_.path -eq "$CheckId.stdout.log" } | Select-Object -First 1
    $pngArtifact = $artifacts | Where-Object { $_.path -like '*texture-preview.png' } | Select-Object -First 1
    if (-not $logArtifact -or -not $pngArtifact) { throw 'MCP artifact list omitted the expected log or PNG preview.' }
    $logResponse = Invoke-McpRequest 'tools/call' @{ name = 'verification_read_artifact'; arguments = @{ run_id = $runId; check_id = $CheckId; path = $logArtifact.path } }
    if ($logResponse.result.isError -or $logResponse.result.content[0].text -notmatch 'GLB texture round-trip passed') { throw 'MCP log retrieval did not include the verifier success line.' }
    $pngResponse = Invoke-McpRequest 'tools/call' @{ name = 'verification_read_artifact'; arguments = @{ run_id = $runId; check_id = $CheckId; path = $pngArtifact.path } }
    $image = $pngResponse.result.content | Where-Object type -eq 'image' | Select-Object -First 1
    if ($pngResponse.result.isError -or -not $image -or $image.mimeType -ne 'image/png') { throw 'MCP client did not receive a displayable PNG image.' }
    $runPath = Join-Path $OutputDirectory $runId
    [IO.File]::WriteAllBytes((Join-Path $runPath 'mcp-client-displayed-preview.png'), [Convert]::FromBase64String($image.data))

    $failureResponse = Invoke-McpRequest 'tools/call' @{ name = 'verification_run_check'; arguments = @{ check_id = 'intentional-invalid-check'; timeout_seconds = 5 } }
    $failureMessage = [string]$failureResponse.result.content[0].text
    if (-not $failureResponse.result.isError -or $failureMessage -notmatch 'Unsupported check ID') { throw 'MCP client did not surface the intentional invalid-check error clearly.' }
    $transcriptPath = Join-Path $runPath 'mcp-client-transcript.json'
    $pilot = [ordered]@{
        client = 'sketch3d-release-review-pilot'
        transport = 'stdio'
        server_executable = $Executable
        server_sha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
        check_id = $CheckId
        run_id = $runId
        discovered_check = $true
        executed_outcome = $runPayload.check.outcome
        retrieved_result = $true
        retrieved_log = $true
        retrieved_png = $true
        displayed_preview_artifact = 'mcp-client-displayed-preview.png'
        intentional_failure = @{ surfaced = $true; message = $failureMessage }
        transcript = $transcript
    }
    $pilot | ConvertTo-Json -Depth 60 | Set-Content -LiteralPath $transcriptPath -Encoding utf8
    Write-Output "MCP pilot passed: $runId"
    Write-Output "MCP client transcript: $transcriptPath"
} finally {
    if ($server -and -not $server.HasExited) { $server.StandardInput.Close(); $server.WaitForExit(5000) | Out-Null }
    if ($server) { $server.Dispose() }
}
