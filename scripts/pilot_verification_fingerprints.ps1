# Verify same-version build fingerprints and legacy result readability.
[CmdletBinding()]
param(
    [string]$Executable = 'target/release/sketch3d.exe',
    [string]$VerificationRoot = 'verification'
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [IO.Path]::IsPathRooted($Executable)) { $Executable = Join-Path $repoRoot $Executable }
if (-not [IO.Path]::IsPathRooted($VerificationRoot)) { $VerificationRoot = Join-Path $repoRoot $VerificationRoot }
$Executable = [IO.Path]::GetFullPath($Executable)
$VerificationRoot = [IO.Path]::GetFullPath($VerificationRoot)
$suffix = [guid]::NewGuid().ToString('N').Substring(0, 8)
$copyRoot = Join-Path $VerificationRoot "fingerprint-pilot-$suffix"
[IO.Directory]::CreateDirectory($copyRoot) | Out-Null
$exeA = Join-Path $copyRoot 'build-a.exe'
$exeB = Join-Path $copyRoot 'build-b.exe'
Copy-Item -LiteralPath $Executable -Destination $exeA
Copy-Item -LiteralPath $Executable -Destination $exeB
$stream = [IO.File]::Open($exeB, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::Read)
try { $overlay = [byte[]](0x53, 0x4B, 0x45, 0x54, 0x43, 0x48, 0x33, 0x44); $stream.Write($overlay, 0, $overlay.Length) } finally { $stream.Dispose() }
$versionA = [Diagnostics.FileVersionInfo]::GetVersionInfo($exeA).ProductVersion
$versionB = [Diagnostics.FileVersionInfo]::GetVersionInfo($exeB).ProductVersion
$hashA = (Get-FileHash -LiteralPath $exeA -Algorithm SHA256).Hash.ToLowerInvariant()
$hashB = (Get-FileHash -LiteralPath $exeB -Algorithm SHA256).Hash.ToLowerInvariant()
if ($versionA -ne $versionB -or $hashA -eq $hashB) { throw 'Same-version build fixtures did not have distinct SHA-256 fingerprints.' }

function Invoke-Runner([string]$Path) {
    $output = & pwsh -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'run_verification.ps1') -Executable $Path -Check startup -OutputDirectory $VerificationRoot -TimeoutSeconds 90 2>&1
    $resultLine = @($output | ForEach-Object { [string]$_ } | Where-Object { $_.StartsWith('Normalized result: ') } | Select-Object -Last 1)
    if ($resultLine.Count -eq 0) { throw "Runner did not report a normalized result: $($output -join "`n")" }
    return (Get-Content -LiteralPath $resultLine[0].Substring('Normalized result: '.Length) -Raw | ConvertFrom-Json -AsHashtable)
}
$resultA = Invoke-Runner $exeA
$resultB = Invoke-Runner $exeB
$buildA = $resultA.build
$buildB = $resultB.build
if ($buildA.version -ne $buildB.version -or $buildA.fingerprint -ne $hashA -or $buildB.fingerprint -ne $hashB -or $buildA.fingerprint -eq $buildB.fingerprint) { throw 'Normalized results did not preserve the distinct build identities.' }

$legacyId = "legacy-metadata-$suffix"
$legacyDirectory = Join-Path $VerificationRoot $legacyId
[IO.Directory]::CreateDirectory($legacyDirectory) | Out-Null
$legacy = $resultA.Clone()
$legacy.run_id = $legacyId
$legacy.Remove('build')
$legacy.Remove('source_revision')
foreach ($check in $legacy.results) { $check.Remove('fixture'); $check.Remove('rendering') }
$legacy | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath (Join-Path $legacyDirectory 'result.json') -Encoding utf8

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
$server = [Diagnostics.Process]::new(); $server.StartInfo = $start
if (-not $server.Start()) { throw 'Could not start the read-only MCP verification server.' }
$rpcId = 0
function Invoke-Mcp([string]$Method, $Params) {
    $script:rpcId++
    $server.StandardInput.WriteLine((@{jsonrpc='2.0'; id=$script:rpcId; method=$Method; params=$Params} | ConvertTo-Json -Compress -Depth 20))
    $server.StandardInput.Flush()
    $line = $server.StandardOutput.ReadLine()
    if ([string]::IsNullOrWhiteSpace($line)) { throw "MCP server closed before replying to $Method" }
    return $line | ConvertFrom-Json -AsHashtable
}
try {
    $null = Invoke-Mcp 'initialize' @{protocolVersion='2025-06-18'; capabilities=@{}; clientInfo=@{name='fingerprint-pilot';version='1.0'}}
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $legacyRead = Invoke-Mcp 'tools/call' @{name='verification_read_result';arguments=@{run_id=$legacyId}}
    if ($legacyRead.result.isError -or $legacyRead.result.structuredContent.run_id -ne $legacyId) { throw 'MCP rejected a legacy result missing optional identity metadata.' }
    $pilot = [ordered]@{ version=$versionA; fingerprint_a=$buildA.fingerprint; fingerprint_b=$buildB.fingerprint; run_a=$resultA.run_id; run_b=$resultB.run_id; legacy_run=$legacyId; legacy_metadata_absent=$true }
    $pilot | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $copyRoot 'fingerprint-pilot.json') -Encoding utf8
    Write-Output "Build fingerprint pilot passed: $($resultA.run_id), $($resultB.run_id); legacy result readable."
} finally {
    if ($server -and -not $server.HasExited) { $server.StandardInput.Close(); $server.WaitForExit(5000) | Out-Null }
    if ($server) { $server.Dispose() }
}
