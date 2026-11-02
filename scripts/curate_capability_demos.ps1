param(
    [switch]$Refresh
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$draftRoot = Join-Path $repoRoot 'verification/capability-demos/drafts'
$mediaRoot = Join-Path $repoRoot 'docs/capability-demos/media'
$clipNames = @(
    'draw-orbit', 'curved-guides', 'mirror-shapes', 'edit-erase', 'reference-export',
    'overview', 'house-tutorial', 'curved-tutorial'
)

if (-not (Test-Path -LiteralPath $draftRoot -PathType Container)) {
    throw "Missing local draft folder: $draftRoot"
}
New-Item -ItemType Directory -Path $mediaRoot -Force | Out-Null

foreach ($name in $clipNames) {
    $extensions = @('mp4', 'gif', 'png', 'en.vtt', 'recording.json')
    if ($name -in @('house-tutorial', 'curved-tutorial')) {
        $extensions += 'saved.sketch.json'
    }
    if ($name -eq 'reference-export') {
        $extensions += 'scene.glb'
    }
    $sourceFiles = @{}
    foreach ($extension in $extensions) {
        $fileName = if ($extension -eq 'saved.sketch.json') {
            "$name-saved.sketch.json"
        } elseif ($extension -eq 'scene.glb') {
            'reference-export/captured-reference-export.glb'
        } else {
            "$name.$extension"
        }
        $source = Join-Path $draftRoot $fileName
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Missing required capture output for ${name}: $source"
        }
        $sourceFiles[$extension] = (Resolve-Path -LiteralPath $source).Path
    }

    $manifestPath = Join-Path $draftRoot "$name.recording.json"
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json -AsHashtable
    if ($manifest.source_tree_dirty -ne $false) {
        throw "Refusing to curate ${name}: source_tree_dirty is not false."
    }
    $gifPath = $sourceFiles['gif']
    if ((Get-Item -LiteralPath $gifPath).Length -gt 5MB) {
        throw "Refusing to curate ${name}: GIF exceeds 5 MiB."
    }
    if ((Get-Item -LiteralPath $sourceFiles['mp4']).Length -gt 512MB) {
        throw "Refusing to curate ${name}: MP4 exceeds 512 MiB."
    }

    $hashes = @{}
    foreach ($extension in $extensions | Where-Object { $_ -ne 'recording.json' }) {
        $source = $sourceFiles[$extension]
        $hashes[$extension] = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        $destinationName = if ($extension -eq 'saved.sketch.json') {
            "$name-saved.sketch.json"
        } elseif ($extension -eq 'scene.glb') {
            'reference-export-scene.glb'
        } else {
            "$name.$extension"
        }
        $destination = Join-Path $mediaRoot $destinationName
        if ((Test-Path -LiteralPath $destination) -and -not $Refresh) {
            throw "Refusing to overwrite curated media without -Refresh: $destination"
        }
        Copy-Item -LiteralPath $source -Destination $destination -Force:$Refresh
    }
    $manifest.outputs.sha256 = $hashes
    $manifest.outputs.gif_preview = if ($name -in @('overview', 'house-tutorial', 'curved-tutorial')) {
        'Accelerated 6-second preview; use the MP4 and timed VTT for normal-speed playback.'
    } else {
        '6-second single-play preview.'
    }
    $manifestDestination = Join-Path $mediaRoot "$name.recording.json"
    if ((Test-Path -LiteralPath $manifestDestination) -and -not $Refresh) {
        throw "Refusing to overwrite curated manifest without -Refresh: $manifestDestination"
    }
    $manifest | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath $manifestDestination -Encoding utf8
    Write-Output "Curated $name with SHA-256 hashes; review flags remain as captured."
}
