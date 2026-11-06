# Exercise the live Sketch3D MCP bridge with a stdio client. This pilot does
# not request or save viewport screenshots.
[CmdletBinding()]
param(
    [string]$Executable = 'target/debug/sketch3d.exe'
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not [IO.Path]::IsPathRooted($Executable)) { $Executable = Join-Path $repoRoot $Executable }
$Executable = [IO.Path]::GetFullPath($Executable)
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { throw "Sketch3D executable not found: $Executable" }

$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $Executable
$start.WorkingDirectory = $repoRoot
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardInput = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.ArgumentList.Add('--mcp-live-app-server')
$server = [Diagnostics.Process]::new()
$server.StartInfo = $start
if (-not $server.Start()) { throw 'Could not start Sketch3D live MCP server.' }
$rpcId = 0
function Invoke-Mcp([string]$Method, $Params) {
    $script:rpcId++
    $request = @{ jsonrpc = '2.0'; id = $script:rpcId; method = $Method; params = $Params }
    $server.StandardInput.WriteLine(($request | ConvertTo-Json -Compress -Depth 20))
    $server.StandardInput.Flush()
    $line = $server.StandardOutput.ReadLine()
    if ([string]::IsNullOrWhiteSpace($line)) { throw "Live MCP server closed before replying to $Method" }
    $response = $line | ConvertFrom-Json -AsHashtable
    if ($response.error) { throw "MCP $Method failed: $($response.error.message)" }
    return $response.result
}
function Invoke-LiveTool([string]$Name, $Arguments) {
    $response = Invoke-Mcp 'tools/call' @{ name = $Name; arguments = $Arguments }
    if ($response.isError) { return @{ error = $true; message = $response.content[0].text; data = $response.structuredContent } }
    return @{ error = $false; message = ''; data = $response.structuredContent }
}
try {
    $null = Invoke-Mcp 'initialize' @{ protocolVersion = '2025-06-18'; capabilities = @{}; clientInfo = @{ name = 'live-mcp-pilot'; version = '1.0' } }
    $server.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
    $tools = Invoke-Mcp 'tools/list' @{}
    $required = @('live_document_snapshot', 'live_scene_outline', 'live_stroke_index', 'live_preview_selection', 'live_apply_edit', 'live_cancel_edit')
    foreach ($name in $required) { if ($tools.tools.name -notcontains $name) { throw "Live MCP did not expose $name" } }

    $snapshot = (Invoke-LiveTool 'live_document_snapshot' @{}).data
    $outline = @()
    $page = Invoke-LiveTool 'live_scene_outline' @{ page_size = 1 }
    if ($page.error) { throw $page.message }
    $outlineRevision = $page.data.revision
    while ($true) {
        $outline += $page.data.items
        $encoded = $page.data | ConvertTo-Json -Depth 20 -Compress
        if ($encoded -match 'geometry|file_path|absolute_path|samples') { throw 'Scene outline leaked geometry or a file path.' }
        if (-not $page.data.next_cursor) { break }
        $page = Invoke-LiveTool 'live_scene_outline' @{ cursor = $page.data.next_cursor; page_size = 1 }
        if ($page.error) { throw "Scene outline pagination failed: $($page.message)" }
        if ($page.data.revision -ne $outlineRevision) { throw 'Scene outline revision changed during pagination.' }
    }
    if ($outline.Count -lt 2) { throw 'Live scene did not provide enough records to exercise pagination.' }

    $index = $null
    foreach ($layer in ($outline | Where-Object kind -eq 'layer')) {
        $candidate = Invoke-LiveTool 'live_stroke_index' @{ layer_id = $layer.id; page_size = 1 }
        if ($candidate.error) { throw "Stroke index failed for layer $($layer.id): $($candidate.message)" }
        if ($candidate.data.items.Count -gt 0) { $index = $candidate; break }
    }
    if (-not $index) { throw 'No layer with a stroke was available for the live selection smoke.' }
    $firstStroke = $index.data.items[0]
    if (-not $firstStroke.can_edit_color) { throw 'The starter scene has no safely selectable stroke on its first populated layer.' }
    $strokeCursor = $index.data.next_cursor
    if ($strokeCursor) {
        $next = Invoke-LiveTool 'live_stroke_index' @{ layer_id = $index.data.layer_id; cursor = $strokeCursor; page_size = 1 }
        if ($next.error) { throw "Stroke index pagination failed: $($next.message)" }
        if ($next.data.items.Count -gt 0 -and [UInt64]$next.data.items[0].id -le [UInt64]$firstStroke.id) { throw 'Stroke index pages are not ordered by increasing ID.' }
    }

    $beforeSelection = @($snapshot.selection.stroke_ids | Sort-Object)
    $selectionPreview = Invoke-LiveTool 'live_preview_selection' @{ revision = $index.data.revision; stroke_ids = @([UInt64]$firstStroke.id) }
    if ($selectionPreview.error) { throw "Selection preview failed: $($selectionPreview.message)" }
    $cancel = Invoke-LiveTool 'live_cancel_edit' @{ preview_id = $selectionPreview.data.preview_id }
    if ($cancel.error) { throw "Selection cancel failed: $($cancel.message)" }
    $afterCancel = (Invoke-LiveTool 'live_document_snapshot' @{}).data
    if ((@($afterCancel.selection.stroke_ids | Sort-Object) -join ',') -ne ($beforeSelection -join ',')) { throw 'Cancelling the selection preview changed the selection.' }

    $selectionPreview = Invoke-LiveTool 'live_preview_selection' @{ revision = $index.data.revision; stroke_ids = @([UInt64]$firstStroke.id) }
    if ($selectionPreview.error) { throw "Second selection preview failed: $($selectionPreview.message)" }
    $apply = Invoke-LiveTool 'live_apply_edit' @{ preview_id = $selectionPreview.data.preview_id; revision = $selectionPreview.data.revision }
    if ($apply.error -or -not $apply.data.applied -or $apply.data.undo_steps -ne 0) { throw "Applying selection did not complete without artwork history: $($apply.message)" }
    $selected = (Invoke-LiveTool 'live_document_snapshot' @{}).data.selection.stroke_ids
    if (@($selected).Count -ne 1 -or [UInt64]$selected[0] -ne [UInt64]$firstStroke.id) { throw 'Applied selection did not select the requested stroke.' }

    $staleOutline = Invoke-LiveTool 'live_scene_outline' @{ cursor = "$outlineRevision/1"; page_size = 1 }
    if (-not $staleOutline.error -or $staleOutline.message -notmatch 'stale') { throw 'A scene outline cursor survived a selection revision change.' }
    if ($strokeCursor) {
        $staleIndex = Invoke-LiveTool 'live_stroke_index' @{ layer_id = $index.data.layer_id; cursor = $strokeCursor; page_size = 1 }
        if (-not $staleIndex.error -or $staleIndex.message -notmatch 'stale') { throw 'A stroke index cursor survived a selection revision change.' }
    }
    Write-Output "Live MCP pilot passed: $($outline.Count) outline records, stroke pagination=$([bool]$strokeCursor), selection preview/cancel/apply, stale cursor rejection. No viewport capture requested."
} finally {
    if ($server -and -not $server.HasExited) {
        $server.StandardInput.Close()
        if (-not $server.WaitForExit(2000)) { $server.Kill($true); $server.WaitForExit() }
    }
    if ($server) { $server.Dispose() }
}
