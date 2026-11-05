# Safely import one normalized result bundle from a CI artifact ZIP.
# No files are extracted until every ZIP path, result and declared artifact validates.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ArchivePath,
    [string]$OutputDirectory = 'verification/normalized',
    [Parameter(Mandatory = $true)][ValidateSet('windows', 'linux', 'macos')][string]$Platform,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Za-z0-9_-]{1,24}$')][string]$CiRunId,
    [ValidateRange(1, 999)][int]$Attempt = 1,
    [string]$SourceLabel = 'github-actions'
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [IO.Path]::IsPathRooted($ArchivePath)) { $ArchivePath = Join-Path $repoRoot $ArchivePath }
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot $OutputDirectory }
$ArchivePath = [IO.Path]::GetFullPath($ArchivePath)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not (Test-Path -LiteralPath $ArchivePath -PathType Leaf)) { throw "Archive not found: $ArchivePath" }
if ((Get-Item -LiteralPath $ArchivePath).Length -gt 128MB) { throw 'Archive exceeds the 128 MiB compressed-size limit.' }
if ($SourceLabel.Length -gt 64 -or $SourceLabel -notmatch '^[A-Za-z0-9_.-]+$') { throw 'SourceLabel must contain at most 64 ASCII letters, digits, dots, underscores, or hyphens.' }
$maxEntries = 1000
$maxEntryBytes = 20MB
$maxExpandedBytes = 100MB
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
try {
    if ($archive.Entries.Count -gt $maxEntries) { throw "Archive exceeds $maxEntries entries." }
    $entryByName = [Collections.Generic.Dictionary[string,object]]::new([StringComparer]::OrdinalIgnoreCase)
    $total = [int64]0
    foreach ($entry in $archive.Entries) {
        if ([string]::IsNullOrEmpty($entry.FullName)) { throw 'Archive contains an empty entry name.' }
        $name = $entry.FullName
        if ($name.Length -gt 512 -or $name.Contains('\') -or $name.StartsWith('/') -or $name -match '^[A-Za-z]:' -or $name -match '(^|/)\.\.?(/|$)' -or $name.ToCharArray().Where({ [char]::IsControl($_) }).Count -gt 0) { throw "Unsafe ZIP entry path: $name" }
        $parts = @($name.TrimEnd('/').Split('/'))
        if ($parts.Count -eq 0 -or $parts.Where({ [string]::IsNullOrEmpty($_) -or $_.Contains(':') -or $_.Length -gt 128 -or $_.EndsWith('.') -or $_.EndsWith(' ') -or $_ -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\..*)?$' }).Count -gt 0) { throw "Unsafe ZIP entry path: $name" }
        if ($entryByName.ContainsKey($name)) { throw "ZIP entry collides by case or is duplicated: $name" }
        $entryByName.Add($name, $entry)
        if ($entry.Length -gt $maxEntryBytes) { throw "ZIP entry exceeds the $maxEntryBytes byte limit: $name" }
        $total += $entry.Length
        if ($total -gt $maxExpandedBytes) { throw "Archive exceeds the $maxExpandedBytes byte expanded-size limit." }
        $unixType = (($entry.ExternalAttributes -shr 16) -band 0xF000)
        if ($unixType -eq 0xA000) { throw "Symbolic links are not accepted: $name" }
    }
    $resultNames = @($entryByName.Keys | Where-Object { [IO.Path]::GetFileName($_) -ceq 'result.json' -and -not $_.EndsWith('/') })
    if ($resultNames.Count -ne 1) { throw "Archive must contain exactly one result.json; found $($resultNames.Count)." }
    $resultName = $resultNames[0]
    $resultEntry = $entryByName[$resultName]
    if ($resultEntry.Length -gt 512KB) { throw 'result.json exceeds the 512 KiB limit.' }
    $reader = [IO.StreamReader]::new($resultEntry.Open(), [Text.Encoding]::UTF8, $true, 4096, $false)
    try { $result = $reader.ReadToEnd() | ConvertFrom-Json -AsHashtable -ErrorAction Stop } finally { $reader.Dispose() }
    if ($result.schema_version -ne 1 -or $result.run_id -notmatch '^[A-Za-z0-9_-]{1,64}$' -or $result.results -isnot [array]) { throw 'result.json is not a valid normalized schema-version-1 result.' }
    $sourceRunId = [string]$result.run_id
    $resultParent = [IO.Path]::GetDirectoryName($resultName.Replace('/', [IO.Path]::DirectorySeparatorChar))
    if ($resultParent -eq '.') { $resultParent = '' }
    $declared = [Collections.Generic.List[string]]::new()
    $declaredSet = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($check in $result.results) {
        if ($check.outcome -notin @('passed', 'failed', 'unsupported', 'timed_out') -or $check.artifacts -isnot [array]) { throw 'A normalized result contains an invalid outcome or artifact list.' }
        foreach ($relative in $check.artifacts) {
            if ($relative -isnot [string] -or [string]::IsNullOrWhiteSpace($relative) -or $relative.Contains('\') -or $relative.StartsWith('/') -or $relative -match '^[A-Za-z]:' -or $relative -match '(^|/)\.\.?(/|$)' -or $relative.Contains(':')) { throw 'A normalized result contains an unsafe artifact path.' }
            $joined = if ($resultParent) { "$resultParent/$relative" } else { $relative }
            if (-not $entryByName.ContainsKey($joined)) { throw "Declared artifact is missing from ZIP: $relative" }
            if (-not $declaredSet.Add($joined)) { continue }
            $declared.Add($joined)
        }
    }
    $safeRunId = "ci-$CiRunId-$Attempt-$Platform-$([guid]::NewGuid().ToString('N').Substring(0,8))"
    $destination = Join-Path $OutputDirectory $safeRunId
    [IO.Directory]::CreateDirectory($destination) | Out-Null
    try {
        $actualExpanded = [int64]0
        foreach ($relativeZipPath in @($resultName) + @($declared)) {
            $entry = $entryByName[$relativeZipPath]
            $relative = if ($resultParent -and $relativeZipPath.StartsWith("$resultParent/", [StringComparison]::OrdinalIgnoreCase)) { $relativeZipPath.Substring($resultParent.Length + 1) } else { $relativeZipPath }
            $target = Join-Path $destination ($relative.Replace('/', [IO.Path]::DirectorySeparatorChar))
            $fullTarget = [IO.Path]::GetFullPath($target)
            if (-not $fullTarget.StartsWith($destination + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Resolved artifact escaped the import directory.' }
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($fullTarget)) | Out-Null
            $inputStream = $entry.Open()
            $outputStream = [IO.File]::Create($fullTarget)
            try {
                $buffer = [byte[]]::new(64KB)
                while (($read = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $actualExpanded += $read
                    if ($outputStream.Length + $read -gt $maxEntryBytes -or $actualExpanded -gt $maxExpandedBytes) { throw 'Decompressed ZIP data exceeded its size limit.' }
                    $outputStream.Write($buffer, 0, $read)
                }
            } finally { $outputStream.Dispose(); $inputStream.Dispose() }
        }
        $imported = Get-Content -LiteralPath (Join-Path $destination 'result.json') -Raw | ConvertFrom-Json -AsHashtable
        $imported.run_id = $safeRunId
        $imported.ci = [ordered]@{ source=$SourceLabel; run_id=$CiRunId; attempt=$Attempt; platform=$Platform; source_run_id=$sourceRunId; imported_utc=[DateTime]::UtcNow.ToString('o') }
        $imported | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $destination 'result.json') -Encoding utf8
    } catch {
        if ([IO.Directory]::Exists($destination)) { [IO.Directory]::Delete($destination, $true) }
        throw
    }
    Write-Output "Imported CI run: $safeRunId"
    Write-Output "Result: $(Join-Path $destination 'result.json')"
} finally { $archive.Dispose() }
