$ErrorActionPreference = 'Stop'
$workspacePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$recorderPath = Join-Path $PSScriptRoot 'record_capability_demo.ps1'
$testRoot = Join-Path $workspacePath 'verification' ("caption-validation-{0}" -f [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot -Force

function Write-Track {
    param([string]$Name, [string]$Text)
    $path = Join-Path $testRoot $Name
    [IO.File]::WriteAllText($path, $Text, [Text.UTF8Encoding]::new($false))
    return $path
}

function Assert-Rejected {
    param(
        [string]$Path,
        [int]$DurationSeconds,
        [string]$ExpectedMessage
    )
    $rejected = $false
    $message = ''
    try {
        & $recorderPath -Name caption-test -DurationSeconds $DurationSeconds -CaptionFile $Path -ValidateOnly
    }
    catch {
        $rejected = $true
        $message = $_.Exception.Message
    }
    if (-not $rejected -or $message -notmatch $ExpectedMessage) {
        throw "Expected caption validation to reject '$Path' with '$ExpectedMessage'; got '$message'."
    }
}

try {
    $valid = Write-Track 'valid.vtt' @'
WEBVTT

00:00:00.000 --> 00:00:10.000
First cue.

00:00:10.000 --> 00:01:00.000
Final cue.
'@
    & $recorderPath -Name caption-test -DurationSeconds 60 -CaptionFile $valid -ValidateOnly

    $shortTimestamps = Write-Track 'short-timestamps.vtt' @'
WEBVTT

00:00.000 --> 00:10.000
Short-form cue.

00:10.000 --> 00:00:20.000
Transition to the hour form.

00:00:20.000 --> 01:00.000
Short-form final cue.
'@
    & $recorderPath -Name caption-test -DurationSeconds 60 -CaptionFile $shortTimestamps -ValidateOnly

    $overlap = Write-Track 'overlap.vtt' @'
WEBVTT

00:00:00.000 --> 00:00:12.000
First cue.

00:00:11.999 --> 00:00:20.000
Overlapping cue.
'@
    Assert-Rejected -Path $overlap -DurationSeconds 60 -ExpectedMessage 'overlaps the previous cue'

    $tooLong = Write-Track 'too-long.vtt' @'
WEBVTT

00:00:59.000 --> 00:01:01.000
This cue exceeds the clip.
'@
    Assert-Rejected -Path $tooLong -DurationSeconds 60 -ExpectedMessage 'extends beyond the 60-second clip'

    $largeHour = Write-Track 'large-hour.vtt' @'
WEBVTT

100:00:00.000 --> 100:00:01.000
Valid hour-form syntax outside the clip duration.
'@
    Assert-Rejected -Path $largeHour -DurationSeconds 60 -ExpectedMessage 'extends beyond the 60-second clip'

    $missingCaptionsRejected = $false
    try {
        & $recorderPath -Name long-clip -DurationSeconds 60 -ValidateOnly
    }
    catch {
        $missingCaptionsRejected = $_.Exception.Message -match 'require a timed WebVTT sidecar'
    }
    if (-not $missingCaptionsRejected) {
        throw 'Long tutorials must require a timed WebVTT sidecar.'
    }

    Write-Output 'Capability-demo caption validation passed.'
}
finally {
    $resolvedVerification = [IO.Path]::GetFullPath((Join-Path $workspacePath 'verification')).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    $resolvedTestRoot = (Resolve-Path -LiteralPath $testRoot).Path
    if (-not $resolvedTestRoot.StartsWith($resolvedVerification, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing to remove caption test files outside verification/.'
    }
    Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
}
