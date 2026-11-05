//! MCP access to normalized verification reports with explicit opt-in execution.
//!
//! Start explicitly with `--mcp-verification-server <verification-directory>`.
//! The directory is canonicalized once, and every reported file is checked
//! against the selected run directory before it is returned.

use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RESULT_BYTES: u64 = 512 * 1024;
const MAX_ARTIFACTS: usize = 256;
const MAX_RUNS: usize = 100;
const MAX_SCANNED_ENTRIES: usize = 1000;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_RUN_TIMEOUT_SECONDS: u64 = 600;
const RUNNER_TIMEOUT_PADDING_SECONDS: u64 = 30;
const MAX_PROCESS_LOG_BYTES: usize = 64 * 1024;
const MAX_TEXT_ARTIFACT_BYTES: u64 = 256 * 1024;
const MAX_PNG_ARTIFACT_BYTES: u64 = 1_000_000;
const MAX_PNG_DIMENSION: u32 = 16_384;
const MAX_DIFF_PIXELS: u64 = 4_000_000;
const MAX_DIFF_PNG_BYTES: usize = 1_000_000;
const MAX_STRUCTURED_METRICS: usize = 64;
static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(0);
static MCP_CLIENT_DISCONNECTED: AtomicBool = AtomicBool::new(false);

pub fn serve(configured_root: &Path) -> Result<(), String> {
    serve_reports(configured_root, false)
}

pub fn serve_with_execution(configured_root: &Path) -> Result<(), String> {
    serve_reports(configured_root, true)
}

fn serve_reports(configured_root: &Path, execution_enabled: bool) -> Result<(), String> {
    let root = configured_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve verification directory: {error}"))?;
    if !root.is_dir() {
        return Err("Configured verification path is not a directory".into());
    }

    serve_config(ServerConfig::Reports {
        root,
        execution_enabled,
    })
}

pub fn serve_drawings(configured_input: &Path, configured_output: &Path) -> Result<(), String> {
    let input = configured_input
        .canonicalize()
        .map_err(|error| format!("Cannot resolve native drawing directory: {error}"))?;
    let output = configured_output
        .canonicalize()
        .map_err(|error| format!("Cannot resolve drawing export directory: {error}"))?;
    if !input.is_dir() || !output.is_dir() {
        return Err("Native drawing and export paths must be existing directories".into());
    }
    serve_config(ServerConfig::Drawings { input, output })
}

enum ServerConfig {
    Reports {
        root: PathBuf,
        execution_enabled: bool,
    },
    Drawings {
        input: PathBuf,
        output: PathBuf,
    },
}

fn serve_config(config: ServerConfig) -> Result<(), String> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    MCP_CLIENT_DISCONNECTED.store(false, Ordering::Relaxed);
    let (sender, receiver) = mpsc::sync_channel(16);
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        loop {
            let mut line = Vec::new();
            let item = match read_bounded_line(&mut input, &mut line) {
                Ok(ReadLine::Line) => InputItem::Line(line),
                Ok(ReadLine::Oversized) => InputItem::Oversized,
                Ok(ReadLine::Eof) => {
                    MCP_CLIENT_DISCONNECTED.store(true, Ordering::Relaxed);
                    let _ = sender.send(InputItem::Eof);
                    return;
                }
                Err(error) => {
                    MCP_CLIENT_DISCONNECTED.store(true, Ordering::Relaxed);
                    let _ = sender.send(InputItem::Failed(error.to_string()));
                    return;
                }
            };
            if sender.send(item).is_err() {
                return;
            }
        }
    });
    loop {
        match receiver.recv().map_err(|error| error.to_string())? {
            InputItem::Eof => return Ok(()),
            InputItem::Failed(error) => return Err(error),
            InputItem::Oversized => {
                write_message(
                    &mut output,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {"code": -32600, "message": "Request exceeds the 64 KiB limit"}
                    }),
                )
                .map_err(|error| error.to_string())?;
            }
            InputItem::Line(line) => {
                let parsed = serde_json::from_slice::<Value>(&line);
                let response = match parsed {
                    Ok(message) => handle_message(&config, message),
                    Err(error) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {"code": -32700, "message": format!("Invalid JSON: {error}")}
                    })),
                };
                if let Some(response) = response {
                    write_message(&mut output, &response).map_err(|error| error.to_string())?;
                }
            }
        }
    }
}

enum InputItem {
    Line(Vec<u8>),
    Oversized,
    Eof,
    Failed(String),
}

enum ReadLine {
    Eof,
    Oversized,
    Line,
}

fn read_bounded_line<R: BufRead>(reader: &mut R, output: &mut Vec<u8>) -> io::Result<ReadLine> {
    output.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if output.is_empty() {
                ReadLine::Eof
            } else {
                ReadLine::Line
            });
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        let oversized = output.len().saturating_add(take) > MAX_REQUEST_BYTES;
        if !oversized {
            output.extend_from_slice(&available[..take]);
        }
        let ended = available.get(take.saturating_sub(1)) == Some(&b'\n');
        reader.consume(take);
        if oversized {
            output.clear();
            if ended {
                return Ok(ReadLine::Oversized);
            }
            loop {
                let available = reader.fill_buf()?;
                if available.is_empty() {
                    return Ok(ReadLine::Oversized);
                }
                let take = available
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(available.len(), |index| index + 1);
                let ended = available.get(take.saturating_sub(1)) == Some(&b'\n');
                reader.consume(take);
                if ended {
                    return Ok(ReadLine::Oversized);
                }
            }
        }
        if ended {
            return Ok(ReadLine::Line);
        }
    }
}

fn write_message<W: Write>(writer: &mut W, message: &Value) -> io::Result<()> {
    let encoded = serde_json::to_vec(message)
        .map_err(|error| io::Error::other(format!("Cannot encode MCP response: {error}")))?;
    if encoded.len() > MAX_RESPONSE_BYTES {
        let bounded = json!({
            "jsonrpc": "2.0",
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "error": {"code": -32603, "message": "Response exceeds the 2 MiB limit"}
        });
        serde_json::to_writer(&mut *writer, &bounded)
            .map_err(|error| io::Error::other(error.to_string()))?;
    } else {
        writer.write_all(&encoded)?;
    }
    writer.write_all(b"\n")?;
    writer.flush()
}

fn handle_message(config: &ServerConfig, message: Value) -> Option<Value> {
    let id = message.get("id").cloned();
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return id.map(|id| rpc_error(id, -32600, "Expected JSON-RPC 2.0"));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return id.map(|id| rpc_error(id, -32600, "Missing JSON-RPC method"));
    };

    match method {
        "initialize" => {
            let id = id?;
            let requested = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            let protocol_version = if supported_protocol(requested) {
                requested
            } else {
                PROTOCOL_VERSION
            };
            Some(rpc_result(
                id,
                json!({
                    "protocolVersion": protocol_version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": if matches!(config, ServerConfig::Reports { .. }) { "sketch3d-verification-reports" } else { "sketch3d-drawing-library" }, "version": env!("CARGO_PKG_VERSION")},
                    "instructions": match config {
                        ServerConfig::Reports { execution_enabled: true, .. } => "Read-only access to normalized verification results and explicit access to allowlisted local verification execution beneath the configured verification directory.",
                        ServerConfig::Reports { .. } => "Read-only access to normalized verification results beneath the configured verification directory. Verification process execution is disabled in this mode.",
                        ServerConfig::Drawings { .. } => "Scoped inspection and bounded GLB export for native drawings, plus creation of new editable native copies beneath the configured output directory."
                    }
                }),
            ))
        }
        "notifications/initialized" | "notifications/cancelled" | "notifications/progress" => None,
        "ping" => id.map(|id| rpc_result(id, json!({}))),
        "tools/list" => id.map(|id| rpc_result(id, json!({ "tools": tool_definitions(config) }))),
        "tools/call" => {
            let id = id?;
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = call_configured_tool(config, name, &arguments);
            Some(rpc_result(id, result))
        }
        _ if id.is_none() => None,
        _ => id.map(|id| rpc_error(id, -32601, "Method not found")),
    }
}

fn supported_protocol(version: &str) -> bool {
    matches!(version, "2025-06-18" | "2025-11-25")
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_definitions(config: &ServerConfig) -> Value {
    let mut reports = json!([
        {
            "name": "verification_list_checks",
            "description": "List the supported verification check IDs and completed run IDs beneath the configured report directory.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
        },
        {
            "name": "verification_read_result",
            "description": "Read a completed schema-versioned result.json for one run.",
            "inputSchema": {"type": "object", "properties": {"run_id": {"type": "string", "maxLength": 64}}, "required": ["run_id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
        },
        {
            "name": "verification_ci_summary",
            "description": "Summarize imported CI outcomes by platform for one exact source revision, fixture ID, and check ID.",
            "inputSchema": {"type":"object","properties":{"source_revision":{"type":"string","minLength":40,"maxLength":64},"fixture_id":{"type":"string","minLength":1,"maxLength":128},"check_id":{"type":"string","enum":["startup","save-reopen","export","glb-texture-roundtrip","ui-workflow"]}},"required":["source_revision","fixture_id","check_id"],"additionalProperties":false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
        },
        {
            "name": "verification_list_artifacts",
            "description": "List the bounded artifact paths and sizes recorded for one check in a completed run.",
            "inputSchema": {"type": "object", "properties": {"run_id": {"type": "string", "maxLength": 64}, "check_id": {"type": "string", "enum": ["startup", "save-reopen", "export", "glb-texture-roundtrip", "ui-workflow"]}}, "required": ["run_id", "check_id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
        },
        {
            "name": "verification_read_artifact",
            "description": "Read one bounded text log or PNG preview recorded in a completed verification result.",
            "inputSchema": {"type": "object", "properties": {"run_id": {"type": "string", "maxLength": 64}, "check_id": {"type": "string", "enum": ["startup", "save-reopen", "export", "glb-texture-roundtrip", "ui-workflow"]}, "path": {"type": "string", "maxLength": 512}}, "required": ["run_id", "check_id", "path"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
        },
        {
            "name": "verification_compare_runs",
            "description": "Compare one check across two completed runs. Reports outcome and bounded structured metric changes, plus a bounded PNG diff only when fixture and rendering metadata match.",
            "inputSchema": {"type": "object", "properties": {"baseline_run_id": {"type": "string", "maxLength": 64}, "current_run_id": {"type": "string", "maxLength": 64}, "check_id": {"type": "string", "enum": ["startup", "save-reopen", "export", "glb-texture-roundtrip", "ui-workflow"]}}, "required": ["baseline_run_id", "current_run_id", "check_id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
        },
        {
            "name": "verification_run_check",
            "description": "Run one allowlisted local verifier in a unique output run directory and return its normalized result. This starts the configured Sketch3D executable and is bounded by timeout_seconds.",
            "inputSchema": {"type": "object", "properties": {"check_id": {"type": "string", "enum": ["startup", "save-reopen", "export", "glb-texture-roundtrip", "ui-workflow"]}, "timeout_seconds": {"type": "integer", "minimum": 5, "maximum": 600, "default": 180}}, "required": ["check_id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}
        }
    ]);
    let drawings = json!([
        {
            "name": "drawing_inspect_native",
            "description": "Return a bounded summary of one native drawing beneath the configured input directory, including layers, guides, world bounds, and managed external-asset requirements.",
            "inputSchema": {"type":"object","properties":{"path":{"type":"string","maxLength":512}},"required":["path"],"additionalProperties":false},
            "annotations": {"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}
        },
        {
            "name": "drawing_export_glb",
            "description": "Export one validated native drawing as a GLB file in the configured output directory. The native source is read-only.",
            "inputSchema": {"type":"object","properties":{"path":{"type":"string","maxLength":512}},"required":["path"],"additionalProperties":false},
            "annotations": {"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false}
        },
        {
            "name": "drawing_create_native_copy",
            "description": "Create a new editable native drawing from a bounded typed command batch. The output name must be new beneath the configured output directory; existing drawings are never overwritten.",
            "inputSchema": {"type":"object","properties":{"output_name":{"type":"string","maxLength":96},"commands":{"type":"array","minItems":3,"maxItems":64,"items":{"oneOf":[
                {"type":"object","properties":{"type":{"const":"create_plane"},"name":{"type":"string","maxLength":96},"origin":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"rotation":{"type":"array","items":{"type":"number"},"minItems":4,"maxItems":4}},"required":["type","name","origin","rotation"],"additionalProperties":false},
                {"type":"object","properties":{"type":{"const":"create_layer"},"name":{"type":"string","maxLength":64}},"required":["type","name"],"additionalProperties":false},
                {"type":"object","properties":{"type":{"const":"line"},"start":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"end":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"radius":{"type":"number","minimum":0.001,"maximum":1}},"required":["type","start","end"],"additionalProperties":false},
                {"type":"object","properties":{"type":{"const":"polyline"},"points":{"type":"array","minItems":2,"maxItems":512,"items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}},"color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"radius":{"type":"number","minimum":0.001,"maximum":1}},"required":["type","points"],"additionalProperties":false},
                {"type":"object","properties":{"type":{"const":"rectangle"},"min":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"max":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"radius":{"type":"number","minimum":0.001,"maximum":1}},"required":["type","min","max"],"additionalProperties":false}
            ]}}},"required":["output_name","commands"],"additionalProperties":false},
            "annotations": {"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false}
        }
    ]);
    match config {
        ServerConfig::Reports {
            execution_enabled, ..
        } => {
            if !execution_enabled {
                if let Some(tools) = reports.as_array_mut() {
                    tools.retain(|tool| {
                        tool.get("name").and_then(Value::as_str) != Some("verification_run_check")
                    });
                }
            }
            reports
        }
        ServerConfig::Drawings { .. } => drawings,
    }
}

fn call_configured_tool(config: &ServerConfig, name: &str, arguments: &Value) -> Value {
    match config {
        ServerConfig::Reports {
            root,
            execution_enabled,
        } => call_tool(root, name, arguments, *execution_enabled),
        ServerConfig::Drawings { input, output } => {
            call_drawing_tool(input, output, name, arguments)
        }
    }
}

fn call_tool(root: &Path, name: &str, arguments: &Value, execution_enabled: bool) -> Value {
    if name == "verification_run_check" && !execution_enabled {
        return json!({
            "content": [{"type": "text", "text": "Verification execution is disabled. Launch --mcp-verification-execution-server to enable the allowlisted run-check tool."}],
            "isError": true
        });
    }
    if name == "verification_read_artifact" {
        let result = required_string(arguments, "run_id").and_then(|run_id| {
            required_string(arguments, "check_id").and_then(|check_id| {
                required_string(arguments, "path")
                    .and_then(|path| read_artifact(root, run_id, check_id, path))
            })
        });
        return match result {
            Ok(ReadArtifact::Text {
                path,
                text,
                size_bytes,
            }) => json!({
                "content": [{"type": "text", "text": text}],
                "structuredContent": {"path": path, "artifact_type": "text/plain", "size_bytes": size_bytes},
                "isError": false
            }),
            Ok(ReadArtifact::Png {
                path,
                data,
                size_bytes,
            }) => json!({
                "content": [{"type": "image", "data": data, "mimeType": "image/png"}],
                "structuredContent": {"path": path, "artifact_type": "image/png", "size_bytes": size_bytes},
                "isError": false
            }),
            Err(error) => json!({
                "content": [{"type": "text", "text": error}],
                "isError": true
            }),
        };
    }
    if name == "verification_compare_runs" {
        let compared = required_string(arguments, "baseline_run_id").and_then(|baseline| {
            required_string(arguments, "current_run_id").and_then(|current| {
                required_string(arguments, "check_id")
                    .and_then(|check_id| compare_runs(root, baseline, current, check_id))
            })
        });
        return match compared {
            Ok(comparison) => {
                let mut content = vec![json!({
                    "type": "text",
                    "text": serde_json::to_string_pretty(&comparison.summary).unwrap_or_else(|_| "{}".into())
                })];
                if let Some(data) = comparison.diff_png {
                    content.push(json!({"type":"image","data":data,"mimeType":"image/png"}));
                }
                json!({"content":content,"structuredContent":comparison.summary,"isError":false})
            }
            Err(error) => json!({"content":[{"type":"text","text":error}],"isError":true}),
        };
    }
    let result = match name {
        "verification_list_checks" => list_checks(root),
        "verification_read_result" => {
            required_string(arguments, "run_id").and_then(|run_id| read_result(root, run_id))
        }
        "verification_ci_summary" => {
            required_string(arguments, "source_revision").and_then(|revision| {
                required_string(arguments, "fixture_id").and_then(|fixture| {
                    required_string(arguments, "check_id")
                        .and_then(|check| ci_summary(root, &revision, &fixture, &check))
                })
            })
        }
        "verification_list_artifacts" => required_string(arguments, "run_id").and_then(|run_id| {
            required_string(arguments, "check_id")
                .and_then(|check_id| list_artifacts(root, run_id, check_id))
        }),
        "verification_compare_runs" => Err("Comparison dispatch failed".into()),
        "verification_run_check" => required_string(arguments, "check_id")
            .and_then(|check_id| run_check(root, check_id, arguments)),
        _ => Err("Unknown tool".into()),
    };
    match result {
        Ok(payload) => {
            let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into());
            json!({
                "content": [{"type": "text", "text": text}],
                "structuredContent": payload,
                "isError": false
            })
        }
        Err(error) => json!({
            "content": [{"type": "text", "text": error}],
            "isError": true
        }),
    }
}

fn call_drawing_tool(input: &Path, output: &Path, name: &str, arguments: &Value) -> Value {
    let result = match name {
        "drawing_create_native_copy" => create_native_drawing(output, arguments),
        "drawing_inspect_native" | "drawing_export_glb" => required_string(arguments, "path")
            .and_then(|relative| match name {
                "drawing_inspect_native" => inspect_native_drawing(input, relative),
                _ => export_native_drawing(input, output, relative),
            }),
        _ => Err("Unknown drawing tool".into()),
    };
    match result {
        Ok(payload) => {
            let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into());
            json!({
                "content": [{"type":"text", "text":text}],
                "structuredContent": payload,
                "isError": false
            })
        }
        Err(error) => json!({"content":[{"type":"text","text":error}],"isError":true}),
    }
}

const MAX_NATIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DRAWING_SUMMARY_BYTES: usize = 512 * 1024;

fn native_file(input: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    if relative.len() > 512
        || relative_path.is_absolute()
        || relative_path.as_os_str().is_empty()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || relative_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("json"))
    {
        return Err("Native drawing path must be a safe relative .json path".into());
    }
    let mut candidate = input.to_path_buf();
    for component in relative_path.components() {
        candidate.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&candidate).map_err(|_| {
            "Native drawing was not found beneath the configured input directory".to_owned()
        })?;
        if metadata.file_type().is_symlink() {
            return Err("Native drawing path cannot contain symlinks".into());
        }
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let metadata = fs::metadata(&canonical).map_err(|error| error.to_string())?;
    if !canonical.starts_with(input) || !metadata.is_file() {
        return Err(
            "Native drawing is outside the configured input directory or is not a file".into(),
        );
    }
    if metadata.len() > MAX_NATIVE_BYTES {
        return Err("Native drawing exceeds the 64 MiB read limit".into());
    }
    Ok(canonical)
}

fn load_native_document(
    input: &Path,
    relative: &str,
) -> Result<(PathBuf, crate::document::Document), String> {
    let path = native_file(input, relative)?;
    let file = fs::File::open(&path).map_err(|error| error.to_string())?;
    let mut text = String::new();
    file.take(MAX_NATIVE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("Native drawing is not valid UTF-8: {error}"))?;
    if text.len() as u64 > MAX_NATIVE_BYTES {
        return Err("Native drawing grew beyond the 64 MiB read limit".into());
    }
    let document = crate::document::Document::from_json(&text)
        .map_err(|error| format!("Native drawing is invalid: {error}"))?;
    Ok((path, document))
}

fn inspect_native_drawing(input: &Path, relative: &str) -> Result<Value, String> {
    let (path, document) = load_native_document(input, relative)?;
    let asset_root = crate::image_plane_storage::asset_root(&path)?;
    let asset_root_metadata = fs::symlink_metadata(&asset_root).ok();
    let asset_root_safe = asset_root_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
    let mut min = glam::Vec3::splat(f32::INFINITY);
    let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
    let mut samples = 0usize;
    for point in document
        .strokes
        .iter()
        .flat_map(|stroke| stroke.points.iter())
    {
        if point.position.is_finite() {
            min = min.min(point.position);
            max = max.max(point.position);
            samples += 1;
        }
    }
    let bounds = if samples == 0 {
        Value::Null
    } else {
        json!({"min":[min.x,min.y,min.z],"max":[max.x,max.y,max.z],"sample_count":samples})
    };
    let image_assets: Vec<_> = document.image_planes.iter().map(|plane| {
        let candidate = asset_root.join(&plane.asset_file);
        let candidate_metadata = fs::symlink_metadata(&candidate).ok();
        let symlinked_asset = candidate_metadata.as_ref().is_some_and(|metadata| metadata.file_type().is_symlink());
        let resolved = if asset_root_safe && !symlinked_asset {
            plane.resolve_asset(&asset_root)
        } else {
            Ok(None)
        };
        let (available, size_bytes, issue) = match resolved {
            Ok(Some(asset)) => match fs::metadata(&asset) {
                Ok(metadata) => (true, Some(metadata.len()), None),
                Err(error) => (false, None, Some(error.to_string())),
            },
            Ok(None) => (false, None, Some(if symlinked_asset { "asset file is a symlink".to_owned() } else if asset_root_metadata.is_some() && !asset_root_safe { "managed asset directory is unavailable".to_owned() } else { "missing".to_owned() })),
            Err(error) => (false, None, Some(error)),
        };
        json!({"id":plane.id,"name":plane.name,"file":plane.asset_file,"available":available,"size_bytes":size_bytes,"pixel_size":plane.pixel_size,"issue":issue})
    }).collect();
    let models_root = asset_root.join("models");
    let models_metadata = fs::symlink_metadata(&models_root).ok();
    let models_root_safe = asset_root_safe
        && models_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
    let model_assets: Vec<_> = document.model_references.iter().map(|resource| {
        let bundle = models_root.join(resource.asset.id.to_string());
        let metadata = models_root_safe.then(|| fs::symlink_metadata(&bundle).ok()).flatten();
        let available = metadata.as_ref().is_some_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
        let size_bytes = if available { directory_size_bounded(&bundle, crate::obj_reference_storage::MAX_MODEL_BUNDLE_BYTES).ok() } else { None };
        json!({"id":resource.id,"name":resource.name,"bundle":resource.asset.bundle_dir,"available":available,"size_bytes":size_bytes,"issue":if available { Value::Null } else { json!("managed model bundle is missing or unsafe") }})
    }).collect();
    let summary = json!({
        "path":relative,
        "document_version":document.version,
        "counts":{"strokes":document.strokes.len(),"layers":document.layers.len(),"guides":document.guide_resources.len(),"image_planes":document.image_planes.len(),"model_references":document.model_references.len()},
        "bounds":{"coordinate_space":"world","source":"static_stroke_points","extent":bounds},
        "layers":document.layers.iter().map(|layer| json!({"id":layer.id,"name":layer.name,"visible":layer.visible,"locked":layer.locked,"folder":document.layer_folders.iter().find(|folder|folder.layer_ids.contains(&layer.id)).map(|folder| folder.name.clone())})).collect::<Vec<_>>(),
        "guides":document.guide_resources.iter().map(|guide| json!({"id":guide.id,"name":guide.name,"visible":guide.visible,"locked":guide.locked,"kind":serde_json::to_value(&guide.geometry).ok().and_then(|value|value.get("kind").and_then(Value::as_str).map(str::to_owned))})).collect::<Vec<_>>(),
        "external_assets":{"image_planes":image_assets,"model_references":model_assets}
    });
    let encoded = serde_json::to_vec(&summary).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_DRAWING_SUMMARY_BYTES {
        return Err("Native drawing summary exceeds the 512 KiB response limit".into());
    }
    Ok(summary)
}

fn directory_size_bounded(path: &Path, limit: u64) -> Result<u64, String> {
    let mut total = 0u64;
    let mut pending = vec![path.to_path_buf()];
    let mut seen = 0usize;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            seen += 1;
            if seen > 4096 {
                return Err("Model bundle has too many entries".into());
            }
            let entry = entry.map_err(|error| error.to_string())?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() {
                return Err("Model bundle contains a symlink".into());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
                if total > limit {
                    return Err("Model bundle exceeds its size limit".into());
                }
            }
        }
    }
    Ok(total)
}

fn export_native_drawing(input: &Path, output: &Path, relative: &str) -> Result<Value, String> {
    let (path, document) = load_native_document(input, relative)?;
    let filename = path.file_stem().ok_or("Native drawing has no filename")?;
    let destination = output.join(filename).with_extension("glb");
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(
            "A GLB export with that name already exists in the configured output directory".into(),
        );
    }
    let asset_root = crate::image_plane_storage::asset_root(&path)?;
    let summary = crate::export::write_glb_with_images(&document, None, &asset_root, &destination)?;
    let metadata = fs::metadata(&destination).map_err(|error| error.to_string())?;
    if metadata.len() > 128 * 1024 * 1024 {
        let _ = fs::remove_file(&destination);
        return Err("Export exceeded the 128 MiB output limit and was removed".into());
    }
    Ok(
        json!({"source":relative,"output":destination.file_name().unwrap_or_default().to_string_lossy(),"size_bytes":metadata.len(),"strokes":summary.strokes,"vertices":summary.vertices,"triangles":summary.triangles,"image_planes":summary.image_planes,"model_references":summary.model_references}),
    )
}

const MAX_CREATE_COMMANDS: usize = 64;
const MAX_CREATE_STROKES: usize = 60;
const MAX_CREATE_POINTS: usize = 2048;
const MAX_CREATED_NATIVE_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateNativeRequest {
    output_name: String,
    commands: Vec<CreateDrawingCommand>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CreateDrawingCommand {
    CreatePlane {
        name: String,
        origin: [f32; 3],
        /// Quaternion in x, y, z, w order.
        rotation: [f32; 4],
    },
    CreateLayer {
        name: String,
    },
    Line {
        start: [f32; 2],
        end: [f32; 2],
        #[serde(default = "default_stroke_color")]
        color: [f32; 3],
        #[serde(default = "default_stroke_radius")]
        radius: f32,
    },
    Polyline {
        points: Vec<[f32; 2]>,
        #[serde(default = "default_stroke_color")]
        color: [f32; 3],
        #[serde(default = "default_stroke_radius")]
        radius: f32,
    },
    Rectangle {
        min: [f32; 2],
        max: [f32; 2],
        #[serde(default = "default_stroke_color")]
        color: [f32; 3],
        #[serde(default = "default_stroke_radius")]
        radius: f32,
    },
}

fn default_stroke_color() -> [f32; 3] {
    [0.08, 0.08, 0.08]
}
fn default_stroke_radius() -> f32 {
    0.025
}

fn create_native_drawing(output: &Path, arguments: &Value) -> Result<Value, String> {
    let request: CreateNativeRequest = serde_json::from_value(arguments.clone())
        .map_err(|error| format!("Invalid typed drawing command batch: {error}"))?;
    if request.commands.len() < 3 || request.commands.len() > MAX_CREATE_COMMANDS {
        return Err(format!(
            "Command batch must contain 3–{MAX_CREATE_COMMANDS} commands"
        ));
    }
    if request.output_name.len() > 96
        || request.output_name.trim() != request.output_name
        || request.output_name.chars().any(char::is_control)
        || request
            .output_name
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
        || Path::new(&request.output_name).components().count() != 1
        || !matches!(
            Path::new(&request.output_name).components().next(),
            Some(Component::Normal(_))
        )
        || Path::new(&request.output_name)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("json"))
    {
        return Err("output_name must be a simple .json filename of at most 96 characters".into());
    }
    let destination = output.join(&request.output_name);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(
            "A native drawing with that name already exists in the configured output directory"
                .into(),
        );
    }

    let mut plane: Option<crate::document::guide_resources::GuideResource> = None;
    let mut layer_name: Option<String> = None;
    let mut marks: Vec<(Vec<[f32; 2]>, [f32; 3], f32)> = Vec::new();
    let mut total_points = 0usize;
    for command in request.commands {
        match command {
            CreateDrawingCommand::CreatePlane {
                name,
                origin,
                rotation,
            } => {
                if plane.is_some() || !marks.is_empty() {
                    return Err("Create exactly one plane before any drawing commands".into());
                }
                let guide = crate::document::guide_resources::GuideResource {
                    id: 1,
                    name,
                    visible: true,
                    locked: false,
                    geometry: crate::document::guide_resources::GuideGeometry::Plane {
                        origin: glam::Vec3::from_array(origin),
                        rotation: glam::Quat::from_xyzw(
                            rotation[0],
                            rotation[1],
                            rotation[2],
                            rotation[3],
                        ),
                    },
                };
                guide.validate()?;
                plane = Some(guide);
            }
            CreateDrawingCommand::CreateLayer { name } => {
                if layer_name.is_some() || !marks.is_empty() {
                    return Err("Create exactly one layer before any drawing commands".into());
                }
                let layer = crate::document::Layer {
                    id: 1,
                    name: name.clone(),
                    visible: true,
                    locked: false,
                    opacity: 1.0,
                    blend_mode: crate::document::LayerBlendMode::Normal,
                    hide_onion_skin: false,
                    mask_inverted: false,
                    mask_source: None,
                    clip_to_below: false,
                };
                let validation_doc = crate::document::Document {
                    layers: vec![layer],
                    ..crate::document::Document::default()
                };
                let encoded =
                    serde_json::to_string(&validation_doc).map_err(|error| error.to_string())?;
                crate::document::Document::from_json(&encoded)?;
                layer_name = Some(name);
            }
            CreateDrawingCommand::Line {
                start,
                end,
                color,
                radius,
            } => {
                if plane.is_none() || layer_name.is_none() {
                    return Err("Create a plane and layer before adding strokes".into());
                }
                validate_mark_style(color, radius)?;
                total_points = total_points.saturating_add(2);
                marks.push((vec![start, end], color, radius));
            }
            CreateDrawingCommand::Polyline {
                points,
                color,
                radius,
            } => {
                if plane.is_none() || layer_name.is_none() {
                    return Err("Create a plane and layer before adding strokes".into());
                }
                validate_mark_style(color, radius)?;
                if !(2..=512).contains(&points.len()) {
                    return Err("Polyline commands require 2–512 points".into());
                }
                total_points = total_points.saturating_add(points.len());
                marks.push((points, color, radius));
            }
            CreateDrawingCommand::Rectangle {
                min,
                max,
                color,
                radius,
            } => {
                if plane.is_none() || layer_name.is_none() {
                    return Err("Create a plane and layer before adding strokes".into());
                }
                validate_mark_style(color, radius)?;
                let (min_x, min_y) = (min[0], min[1]);
                let (max_x, max_y) = (max[0], max[1]);
                if !min_x.is_finite()
                    || !min_y.is_finite()
                    || !max_x.is_finite()
                    || !max_y.is_finite()
                    || min_x >= max_x
                    || min_y >= max_y
                {
                    return Err("Rectangle bounds must be finite and ordered".into());
                }
                total_points = total_points.saturating_add(5);
                marks.push((
                    vec![min, [max_x, min_y], max, [min_x, max_y], min],
                    color,
                    radius,
                ));
            }
        }
        if marks.len() > MAX_CREATE_STROKES || total_points > MAX_CREATE_POINTS {
            return Err("Command batch exceeds the 60 stroke or 2,048 point limit".into());
        }
    }
    let plane = plane.ok_or("Command batch must create one planar guide")?;
    let layer_name = layer_name.ok_or("Command batch must create one layer")?;
    if marks.is_empty() {
        return Err("Command batch must contain at least one supported stroke".into());
    }

    let (origin, rotation) = match &plane.geometry {
        crate::document::guide_resources::GuideGeometry::Plane { origin, rotation } => {
            (*origin, *rotation)
        }
        _ => return Err("Only a planar drawing guide is supported".into()),
    };
    let mut document = crate::document::Document::default();
    let created_layer_name = layer_name.clone();
    document.layers[0].name = layer_name;
    document.guide_resources.push(plane);
    document.active_guide = Some(1);
    let axis_u = rotation * glam::Vec3::X;
    let normal = rotation * glam::Vec3::Z;
    for (index, (coordinates, color, radius)) in marks.into_iter().enumerate() {
        let stroke_id = index as u64 + 1;
        document.strokes.push(crate::document::Stroke {
            guide_attachment: None,
            model_surface: None,
            curve: None,
            material: crate::document::StrokeMaterial::Unlit,
            point_thickness: None,
            point_colors: None,
            modifiers: None,
            fill: None,
            group_id: None,
            layer_id: 1,
            id: stroke_id,
            color,
            gradient: None,
            radius,
            brush: crate::document::Brush {
                kind: crate::document::BrushKind::Round,
                normal,
                tangent: axis_u,
                ..crate::document::Brush::default()
            },
            points: coordinates
                .into_iter()
                .enumerate()
                .map(|(point_index, point)| {
                    let local = glam::Vec3::new(point[0], point[1], 0.0);
                    crate::document::Point {
                        position: origin + rotation * local,
                        pressure: 1.0,
                        tilt: None,
                        time_ms: point_index as u32,
                    }
                })
                .collect(),
        });
    }
    let encoded = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_CREATED_NATIVE_BYTES {
        return Err("Created native document exceeds the 1 MiB output limit".into());
    }
    let validated = crate::document::Document::from_json(
        std::str::from_utf8(&encoded).map_err(|error| error.to_string())?,
    )?;
    let encoded = serde_json::to_vec(&validated).map_err(|error| error.to_string())?;
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(
            "A native drawing with that name already exists in the configured output directory"
                .into(),
        );
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(output).map_err(|error| error.to_string())?;
    temporary
        .write_all(&encoded)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist_noclobber(&destination)
        .map_err(|error| format!("Could not atomically create native copy: {}", error.error))?;
    Ok(
        json!({"output": request.output_name,"bytes":encoded.len(),"layer":created_layer_name,"plane":"Planar guide","strokes":document.strokes.len(),"points":total_points,"editable":true}),
    )
}

fn validate_mark_style(color: [f32; 3], radius: f32) -> Result<(), String> {
    if color
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err("Stroke colors must be finite linear RGB values from 0 to 1".into());
    }
    if !radius.is_finite() || !(0.001..=1.0).contains(&radius) {
        return Err("Stroke radius must be from 0.001 to 1 world unit".into());
    }
    Ok(())
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Missing string argument: {key}"))
}

fn list_checks(root: &Path) -> Result<Value, String> {
    let mut runs = Vec::new();
    let mut scanned = 0usize;
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        scanned += 1;
        if scanned > MAX_SCANNED_ENTRIES {
            break;
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !valid_run_id(&name) {
            continue;
        }
        let Ok(run_directory) = run_directory(root, &name) else {
            continue;
        };
        let result_path = run_directory.join("result.json");
        let Ok(result_path) = result_path.canonicalize() else {
            continue;
        };
        if result_path.parent() != Some(run_directory.as_path())
            || fs::metadata(&result_path)
                .map(|metadata| metadata.len() > MAX_RESULT_BYTES)
                .unwrap_or(true)
        {
            continue;
        }
        runs.push((
            entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok()),
            name,
        ));
    }
    runs.sort_by_key(|left| std::cmp::Reverse(left.0));
    let completed = runs
        .into_iter()
        .take(MAX_RUNS)
        .map(|(_, run_id)| run_id)
        .collect::<Vec<_>>();
    Ok(json!({
        "checks": [
            {"check_id":"startup", "description":"Application startup smoke check"},
            {"check_id":"save-reopen", "description":"Animation native save/reopen check"},
            {"check_id":"export", "description":"Still image export check"},
            {"check_id":"glb-texture-roundtrip", "description":"Textured GLB import, native save/reopen, and whole-scene export check"},
            {"check_id":"ui-workflow", "description":"Interactive inspector collapse, keyboard expansion, hide and restore workflow"}
        ],
        "completed_runs": completed,
        "run_limit": MAX_RUNS,
        "scan_limit": MAX_SCANNED_ENTRIES
    }))
}

fn read_result(root: &Path, run_id: &str) -> Result<Value, String> {
    let (_, _, bytes) = result_file(root, run_id)?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("Invalid result JSON: {error}"))?;
    if value.get("schema_version").and_then(Value::as_u64) != Some(1)
        || !value.get("results").is_some_and(Value::is_array)
    {
        return Err("Run result does not use schema version 1".into());
    }
    Ok(value)
}

fn ci_summary(
    root: &Path,
    source_revision: &str,
    fixture_id: &str,
    check_id: &str,
) -> Result<Value, String> {
    if !matches!(
        check_id,
        "startup" | "save-reopen" | "export" | "glb-texture-roundtrip" | "ui-workflow"
    ) {
        return Err("Unsupported check ID".into());
    }
    if !matches!(source_revision.len(), 40 | 64)
        || !source_revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        || fixture_id.is_empty()
        || fixture_id.len() > 128
        || !fixture_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("Source revision or fixture ID is invalid".into());
    }
    let entries = fs::read_dir(root).map_err(|error| error.to_string())?;
    let mut outcomes: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut matched = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= MAX_SCANNED_ENTRIES {
            break;
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let run_id = entry.file_name().to_string_lossy().into_owned();
        if !file_type.is_dir() || file_type.is_symlink() || !valid_run_id(&run_id) {
            continue;
        }
        let Ok(result) = read_result(root, &run_id) else {
            continue;
        };
        let revision = result
            .pointer("/source_revision/value")
            .and_then(Value::as_str);
        if revision != Some(source_revision) {
            continue;
        }
        let Some(platform) = result
            .pointer("/ci/platform")
            .and_then(Value::as_str)
            .filter(|platform| matches!(*platform, "windows" | "linux" | "macos"))
        else {
            continue;
        };
        let Some(checks) = result.get("results").and_then(Value::as_array) else {
            continue;
        };
        let Some(check) = checks.iter().find(|check| {
            check.get("check_id").and_then(Value::as_str) == Some(check_id)
                && check.pointer("/fixture/id").and_then(Value::as_str) == Some(fixture_id)
        }) else {
            continue;
        };
        let Some(outcome) = check
            .get("outcome")
            .and_then(Value::as_str)
            .filter(|outcome| {
                matches!(*outcome, "passed" | "failed" | "unsupported" | "timed_out")
            })
        else {
            continue;
        };
        *outcomes
            .entry(platform.to_owned())
            .or_default()
            .entry(outcome.to_owned())
            .or_default() += 1;
        matched.push(json!({"run_id":run_id,"platform":platform,"outcome":outcome}));
    }
    Ok(json!({
        "source_revision": source_revision,
        "fixture_id": fixture_id,
        "check_id": check_id,
        "matching_runs": matched,
        "platform_outcomes": outcomes,
        "scan_limit": MAX_SCANNED_ENTRIES
    }))
}

fn result_file(root: &Path, run_id: &str) -> Result<(PathBuf, PathBuf, Vec<u8>), String> {
    let run_directory = run_directory(root, run_id)?;
    let result_path = run_directory
        .join("result.json")
        .canonicalize()
        .map_err(|_| "Completed result.json was not found".to_owned())?;
    if result_path.parent() != Some(run_directory.as_path()) {
        return Err("Result path is outside its run directory".into());
    }
    let metadata = fs::metadata(&result_path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_RESULT_BYTES {
        return Err("Result exceeds the 512 KiB read limit".into());
    }
    let bytes = fs::read(&result_path).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_RESULT_BYTES {
        return Err("Result grew beyond the 512 KiB read limit".into());
    }
    Ok((result_path, run_directory, bytes))
}

fn run_directory(root: &Path, run_id: &str) -> Result<PathBuf, String> {
    if !valid_run_id(run_id) {
        return Err("Invalid run ID".into());
    }
    let run_directory = root
        .join(run_id)
        .canonicalize()
        .map_err(|_| "Completed run was not found".to_owned())?;
    if run_directory.parent() != Some(root) || !run_directory.is_dir() {
        return Err("Run directory is outside the configured verification root".into());
    }
    Ok(run_directory)
}

fn list_artifacts(root: &Path, run_id: &str, check_id: &str) -> Result<Value, String> {
    if !matches!(
        check_id,
        "startup" | "save-reopen" | "export" | "glb-texture-roundtrip" | "ui-workflow"
    ) {
        return Err("Unsupported check ID".into());
    }
    let (_, run_directory, bytes) = result_file(root, run_id)?;
    let result: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("Invalid result JSON: {error}"))?;
    if result.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err("Run result does not use schema version 1".into());
    }
    let check = result
        .get("results")
        .and_then(Value::as_array)
        .and_then(|checks| {
            checks
                .iter()
                .find(|item| item.get("check_id").and_then(Value::as_str) == Some(check_id))
        })
        .ok_or("Check is not present in this completed run")?;
    let declared = check
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or("Completed check has no artifact list")?;
    if declared.len() > MAX_ARTIFACTS {
        return Err("Artifact list exceeds the 256 item limit".into());
    }
    let mut artifacts = Vec::with_capacity(declared.len());
    for relative in declared {
        let relative = relative.as_str().ok_or("Artifact path is not a string")?;
        let canonical = resolve_artifact_path(&run_directory, relative)?;
        let metadata = fs::metadata(&canonical).map_err(|error| error.to_string())?;
        artifacts.push(json!({
            "path": relative,
            "size_bytes": metadata.len(),
            "artifact_type": artifact_type(relative).unwrap_or("unsupported")
        }));
    }
    Ok(json!({
        "run_id": run_id,
        "check_id": check_id,
        "artifacts": artifacts,
        "artifact_limit": MAX_ARTIFACTS
    }))
}

enum ReadArtifact {
    Text {
        path: String,
        text: String,
        size_bytes: u64,
    },
    Png {
        path: String,
        data: String,
        size_bytes: u64,
    },
}

struct RunComparison {
    summary: Value,
    diff_png: Option<String>,
}

fn compare_runs(
    root: &Path,
    baseline_run_id: &str,
    current_run_id: &str,
    check_id: &str,
) -> Result<RunComparison, String> {
    if !matches!(
        check_id,
        "startup" | "save-reopen" | "export" | "glb-texture-roundtrip" | "ui-workflow"
    ) {
        return Err("Unsupported check ID".into());
    }
    let baseline = read_result(root, baseline_run_id)?;
    let current = read_result(root, current_run_id)?;
    let baseline_check = result_check(&baseline, check_id)?;
    let current_check = result_check(&current, check_id)?;
    let baseline_outcome = baseline_check
        .get("outcome")
        .and_then(Value::as_str)
        .ok_or("Baseline check has no outcome")?;
    let current_outcome = current_check
        .get("outcome")
        .and_then(Value::as_str)
        .ok_or("Current check has no outcome")?;
    let same_fixture = fixtures_match(baseline_check, current_check);
    let baseline_metrics = read_report_metrics(root, baseline_run_id, check_id, baseline_check)?;
    let current_metrics = read_report_metrics(root, current_run_id, check_id, current_check)?;
    let (metric_status, changed_metrics) = if !same_fixture {
        ("unsupported", Vec::new())
    } else {
        match (baseline_metrics, current_metrics) {
            (Some(baseline), Some(current)) => {
                ("compared", compare_metric_maps(&baseline, &current))
            }
            _ => ("unsupported", Vec::new()),
        }
    };

    let (visual, diff_png) = compare_visual_artifact(
        root,
        baseline_run_id,
        current_run_id,
        check_id,
        baseline_check,
        current_check,
        same_fixture,
    )?;
    let regression = if !same_fixture || current_outcome == "unsupported" {
        Value::Null
    } else {
        json!(baseline_outcome == "passed" && matches!(current_outcome, "failed" | "timed_out"))
    };
    let regression_status = if regression.is_null() {
        "unsupported"
    } else {
        "compared"
    };
    let summary = json!({
        "baseline_run_id": baseline_run_id,
        "current_run_id": current_run_id,
        "check_id": check_id,
        "comparison_status": if same_fixture { "compared" } else { "unsupported" },
        "fixture_matches": same_fixture,
        "outcomes": {"baseline": baseline_outcome, "current": current_outcome, "changed": baseline_outcome != current_outcome},
        "regression": regression,
        "regression_status": regression_status,
        "metrics": {"status": metric_status, "changed": changed_metrics},
        "visual": visual
    });
    Ok(RunComparison { summary, diff_png })
}

fn result_check<'a>(result: &'a Value, check_id: &str) -> Result<&'a Value, String> {
    result
        .get("results")
        .and_then(Value::as_array)
        .and_then(|checks| {
            checks
                .iter()
                .find(|item| item.get("check_id").and_then(Value::as_str) == Some(check_id))
        })
        .ok_or_else(|| format!("Check '{check_id}' is not present in the selected run"))
}

fn fixtures_match(baseline: &Value, current: &Value) -> bool {
    let Some(baseline) = baseline.get("fixture") else {
        return false;
    };
    let Some(current) = current.get("fixture") else {
        return false;
    };
    let same_id = baseline
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| current.get("id").and_then(Value::as_str) == Some(id));
    if !same_id {
        return false;
    }
    match (
        baseline.get("fingerprint_status").and_then(Value::as_str),
        current.get("fingerprint_status").and_then(Value::as_str),
    ) {
        (Some("available"), Some("available")) => {
            baseline.get("fingerprint") == current.get("fingerprint")
                && baseline
                    .get("fingerprint")
                    .and_then(Value::as_str)
                    .is_some()
        }
        (Some("not_applicable"), Some("not_applicable")) => true,
        _ => false,
    }
}

fn read_report_metrics(
    root: &Path,
    run_id: &str,
    check_id: &str,
    check: &Value,
) -> Result<Option<std::collections::BTreeMap<String, Value>>, String> {
    let Some(artifacts) = check.get("artifacts").and_then(Value::as_array) else {
        return Ok(None);
    };
    if artifacts.len() > MAX_ARTIFACTS {
        return Err("Artifact list exceeds the 256 item limit".into());
    }
    let Some(relative) = artifacts.iter().filter_map(Value::as_str).find(|path| {
        Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    }) else {
        return Ok(None);
    };
    let bytes = read_declared_artifact(root, run_id, check_id, relative, MAX_TEXT_ARTIFACT_BYTES)?;
    let report: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Structured report '{relative}' is invalid JSON: {error}"))?;
    let mut metrics = std::collections::BTreeMap::new();
    collect_structured_metrics(&report, "", &mut metrics);
    Ok(Some(metrics))
}

fn collect_structured_metrics(
    value: &Value,
    prefix: &str,
    metrics: &mut std::collections::BTreeMap<String, Value>,
) {
    if metrics.len() >= MAX_STRUCTURED_METRICS {
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if metrics.len() >= MAX_STRUCTURED_METRICS {
                    break;
                }
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if value.is_number() && is_structured_metric_key(key) {
                    metrics.insert(path, value.clone());
                } else if value.is_object() || value.is_array() {
                    collect_structured_metrics(value, &path, metrics);
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                if metrics.len() >= MAX_STRUCTURED_METRICS {
                    break;
                }
                if value.is_object() || value.is_array() {
                    collect_structured_metrics(value, &format!("{prefix}[{index}]"), metrics);
                }
            }
        }
        _ => {}
    }
}

fn is_structured_metric_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "count",
        "pixels",
        "error",
        "size",
        "width",
        "height",
        "strokes",
        "vertices",
        "triangles",
        "frames",
        "keyframes",
        "samples",
        "vertices",
        "faces",
    ]
    .iter()
    .any(|term| key.contains(term))
}

fn compare_metric_maps(
    baseline: &std::collections::BTreeMap<String, Value>,
    current: &std::collections::BTreeMap<String, Value>,
) -> Vec<Value> {
    let keys = baseline
        .keys()
        .chain(current.keys())
        .collect::<std::collections::BTreeSet<_>>();
    keys.into_iter()
        .filter_map(|key| {
            let old = baseline.get(key);
            let new = current.get(key);
            (old != new).then(|| json!({"metric":key,"baseline":old,"current":new}))
        })
        .take(MAX_STRUCTURED_METRICS)
        .collect()
}

fn compare_visual_artifact(
    root: &Path,
    baseline_run_id: &str,
    current_run_id: &str,
    check_id: &str,
    baseline_check: &Value,
    current_check: &Value,
    same_fixture: bool,
) -> Result<(Value, Option<String>), String> {
    if !same_fixture {
        return Ok((
            json!({"status":"unsupported","reason":"Fixture identity or fingerprint does not match."}),
            None,
        ));
    }
    let baseline_rendering = baseline_check.get("rendering");
    let current_rendering = current_check.get("rendering");
    if baseline_rendering != current_rendering {
        return Ok((
            json!({"status":"unsupported","reason":"Rendering metadata does not match."}),
            None,
        ));
    }
    let (Some(baseline_resolution), Some(current_resolution)) = (
        rendering_resolution(baseline_rendering),
        rendering_resolution(current_rendering),
    ) else {
        return Ok((
            json!({"status":"unsupported","reason":"Both runs need available rendering resolution metadata."}),
            None,
        ));
    };
    if baseline_resolution != current_resolution {
        return Ok((
            json!({"status":"unsupported","reason":"Rendering resolutions do not match."}),
            None,
        ));
    }
    let (Some(baseline_artifacts), Some(current_artifacts)) = (
        baseline_check.get("artifacts").and_then(Value::as_array),
        current_check.get("artifacts").and_then(Value::as_array),
    ) else {
        return Ok((
            json!({"status":"unsupported","reason":"A check has no recorded artifacts."}),
            None,
        ));
    };
    let current_pngs = current_artifacts
        .iter()
        .filter_map(Value::as_str)
        .filter(|path| path.to_ascii_lowercase().ends_with(".png"))
        .collect::<std::collections::BTreeSet<_>>();
    let Some(png_path) = baseline_artifacts
        .iter()
        .filter_map(Value::as_str)
        .find(|path| path.to_ascii_lowercase().ends_with(".png") && current_pngs.contains(path))
    else {
        return Ok((
            json!({"status":"unsupported","reason":"Runs have no PNG artifact with a matching recorded path."}),
            None,
        ));
    };
    let baseline_bytes = read_declared_artifact(
        root,
        baseline_run_id,
        check_id,
        png_path,
        MAX_PNG_ARTIFACT_BYTES,
    )?;
    let current_bytes = read_declared_artifact(
        root,
        current_run_id,
        check_id,
        png_path,
        MAX_PNG_ARTIFACT_BYTES,
    )?;
    validate_png(&baseline_bytes)?;
    validate_png(&current_bytes)?;
    let baseline_dimensions = png_dimensions(&baseline_bytes)?;
    let current_dimensions = png_dimensions(&current_bytes)?;
    if baseline_dimensions != current_dimensions
        || baseline_resolution[0].as_u64() != Some(u64::from(baseline_dimensions.0))
        || baseline_resolution[1].as_u64() != Some(u64::from(baseline_dimensions.1))
    {
        return Ok((
            json!({"status":"unsupported","reason":"PNG dimensions do not match the recorded rendering metadata."}),
            None,
        ));
    }
    let pixel_count = u64::from(baseline_dimensions.0) * u64::from(baseline_dimensions.1);
    if pixel_count > MAX_DIFF_PIXELS {
        return Ok((
            json!({"status":"unsupported","reason":"PNG dimensions exceed the 4,000,000 pixel comparison limit."}),
            None,
        ));
    }
    let baseline_image = image::load_from_memory(&baseline_bytes)
        .map_err(|error| format!("Could not decode baseline PNG: {error}"))?
        .to_rgba8();
    let current_image = image::load_from_memory(&current_bytes)
        .map_err(|error| format!("Could not decode current PNG: {error}"))?
        .to_rgba8();
    let mut diff = image::RgbaImage::new(baseline_image.width(), baseline_image.height());
    let mut changed_pixels = 0u64;
    let mut maximum_channel_delta = 0u8;
    for (x, y, output) in diff.enumerate_pixels_mut() {
        let old = baseline_image.get_pixel(x, y);
        let new = current_image.get_pixel(x, y);
        let delta = [
            old[0].abs_diff(new[0]),
            old[1].abs_diff(new[1]),
            old[2].abs_diff(new[2]),
            old[3].abs_diff(new[3]),
        ];
        if delta.iter().any(|channel| *channel != 0) {
            changed_pixels += 1;
        }
        maximum_channel_delta = maximum_channel_delta.max(*delta.iter().max().unwrap_or(&0));
        *output = image::Rgba([delta[0], delta[1], delta[2], 255]);
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(diff)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| format!("Could not encode comparison PNG: {error}"))?;
    let bytes = encoded.into_inner();
    if bytes.len() > MAX_DIFF_PNG_BYTES {
        return Ok((
            json!({"status":"unsupported","reason":"Encoded diff exceeds the 1,000,000 byte limit."}),
            None,
        ));
    }
    Ok((
        json!({
            "status":"compared",
            "artifact":png_path,
            "resolution_px":baseline_resolution,
            "changed_pixels":changed_pixels,
            "maximum_channel_delta":maximum_channel_delta,
            "diff_size_bytes":bytes.len()
        }),
        Some(encode_base64(&bytes)),
    ))
}

fn read_declared_artifact(
    root: &Path,
    run_id: &str,
    check_id: &str,
    relative: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let (_, run_directory, result_bytes) = result_file(root, run_id)?;
    let result: Value = serde_json::from_slice(&result_bytes)
        .map_err(|error| format!("Invalid result JSON: {error}"))?;
    let check = result_check(&result, check_id)?;
    let declared = check
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or("Completed check has no artifact list")?;
    if declared.len() > MAX_ARTIFACTS {
        return Err("Artifact list exceeds the 256 item limit".into());
    }
    if !declared.iter().any(|path| path.as_str() == Some(relative)) {
        return Err("Artifact path is not recorded for this check".into());
    }
    let path = resolve_artifact_path(&run_directory, relative)?;
    let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
    if metadata.len() > limit {
        return Err(format!(
            "Artifact exceeds the {limit} byte comparison limit"
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "Artifact grew beyond the {limit} byte comparison limit"
        ));
    }
    Ok(bytes)
}

fn rendering_resolution(rendering: Option<&Value>) -> Option<&Value> {
    rendering
        .filter(|value| value.get("status").and_then(Value::as_str) == Some("available"))
        .and_then(|value| value.get("resolution_px"))
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return Err("PNG is missing its required IHDR chunk".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap_or_default());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap_or_default());
    if width == 0 || height == 0 || width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION {
        return Err("PNG dimensions are outside the supported limit".into());
    }
    Ok((width, height))
}

fn read_artifact(
    root: &Path,
    run_id: &str,
    check_id: &str,
    relative: &str,
) -> Result<ReadArtifact, String> {
    if !matches!(
        check_id,
        "startup" | "save-reopen" | "export" | "glb-texture-roundtrip" | "ui-workflow"
    ) {
        return Err("Unsupported check ID".into());
    }
    let (_, run_directory, bytes) = result_file(root, run_id)?;
    let result: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("Invalid result JSON: {error}"))?;
    if result.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err("Run result does not use schema version 1".into());
    }
    let check = result
        .get("results")
        .and_then(Value::as_array)
        .and_then(|checks| {
            checks
                .iter()
                .find(|item| item.get("check_id").and_then(Value::as_str) == Some(check_id))
        })
        .ok_or("Check is not present in this completed run")?;
    let declared = check
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or("Completed check has no artifact list")?;
    if declared.len() > MAX_ARTIFACTS {
        return Err("Artifact list exceeds the 256 item limit".into());
    }
    if !declared.iter().any(|item| item.as_str() == Some(relative)) {
        return Err("Artifact path is not recorded for this check".into());
    }
    let kind = artifact_type(relative).ok_or("Only .log, .txt, and .png artifacts can be read")?;
    let path = resolve_artifact_path(&run_directory, relative)?;
    let limit = if kind == "image/png" {
        MAX_PNG_ARTIFACT_BYTES
    } else {
        MAX_TEXT_ARTIFACT_BYTES
    };
    let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
    if metadata.len() > limit {
        return Err(format!("Artifact exceeds the {limit} byte read limit"));
    }
    let file = fs::File::open(&path).map_err(|error| error.to_string())?;
    let mut content = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1)
        .read_to_end(&mut content)
        .map_err(|error| error.to_string())?;
    if content.len() as u64 > limit {
        return Err(format!("Artifact grew beyond the {limit} byte read limit"));
    }
    match kind {
        "text/plain" => {
            let text = std::str::from_utf8(&content)
                .map_err(|_| "Declared text artifact is not valid UTF-8")?;
            if text.chars().any(|character| character == '\0') {
                return Err("Declared text artifact contains binary data".into());
            }
            Ok(ReadArtifact::Text {
                path: relative.to_owned(),
                text: text.to_owned(),
                size_bytes: content.len() as u64,
            })
        }
        "image/png" => {
            validate_png(&content)?;
            Ok(ReadArtifact::Png {
                path: relative.to_owned(),
                data: encode_base64(&content),
                size_bytes: content.len() as u64,
            })
        }
        _ => Err("Unsupported artifact type".into()),
    }
}

fn artifact_type(relative: &str) -> Option<&'static str> {
    match Path::new(relative)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("log" | "txt") => Some("text/plain"),
        Some("png") => Some("image/png"),
        _ => None,
    }
}

fn resolve_artifact_path(run_directory: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.as_os_str().is_empty()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Artifact path is not a safe relative path".into());
    }
    let mut candidate = run_directory.to_path_buf();
    for component in relative_path.components() {
        candidate.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|_| format!("Artifact is unavailable: {relative}"))?;
        if metadata.file_type().is_symlink() {
            return Err("Artifact path contains a symlink".into());
        }
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|_| format!("Artifact is unavailable: {relative}"))?;
    if !canonical.starts_with(run_directory) || !canonical.is_file() {
        return Err("Artifact path escapes its run directory or is not a file".into());
    }
    Ok(canonical)
}

fn validate_png(bytes: &[u8]) -> Result<(), String> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < SIGNATURE.len() + 25 || !bytes.starts_with(SIGNATURE) {
        return Err("Declared PNG artifact has an invalid PNG structure".into());
    }
    let mut offset = SIGNATURE.len();
    let mut seen_header = false;
    let mut seen_image_data = false;
    let mut seen_end = false;
    while offset < bytes.len() {
        let header_end = offset.checked_add(8).ok_or("PNG chunk offset overflow")?;
        if header_end > bytes.len() {
            return Err("Declared PNG artifact has a truncated chunk header".into());
        }
        let length =
            u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap_or_default()) as usize;
        let data_end = header_end
            .checked_add(length)
            .ok_or("PNG chunk length overflow")?;
        let chunk_end = data_end.checked_add(4).ok_or("PNG chunk length overflow")?;
        if chunk_end > bytes.len() {
            return Err("Declared PNG artifact has a truncated chunk".into());
        }
        let chunk_type = &bytes[offset + 4..header_end];
        let expected_crc =
            u32::from_be_bytes(bytes[data_end..chunk_end].try_into().unwrap_or_default());
        if png_crc32(&bytes[offset + 4..data_end]) != expected_crc {
            return Err("Declared PNG artifact has a corrupt chunk checksum".into());
        }
        if !seen_header {
            if chunk_type != b"IHDR" || length != 13 {
                return Err("PNG must begin with one valid IHDR chunk".into());
            }
            let width = u32::from_be_bytes(
                bytes[header_end..header_end + 4]
                    .try_into()
                    .unwrap_or_default(),
            );
            let height = u32::from_be_bytes(
                bytes[header_end + 4..header_end + 8]
                    .try_into()
                    .unwrap_or_default(),
            );
            if width == 0
                || height == 0
                || width > MAX_PNG_DIMENSION
                || height > MAX_PNG_DIMENSION
                || u64::from(width) * u64::from(height) > 64_000_000
            {
                return Err("PNG dimensions exceed the supported image bounds".into());
            }
            seen_header = true;
        } else if chunk_type == b"IHDR" {
            return Err("PNG contains more than one IHDR chunk".into());
        }
        if chunk_type == b"IDAT" {
            seen_image_data = true;
        }
        if chunk_type == b"IEND" {
            if length != 0 || chunk_end != bytes.len() {
                return Err("PNG has an invalid IEND chunk".into());
            }
            seen_end = true;
            break;
        }
        offset = chunk_end;
    }
    if !seen_header || !seen_image_data || !seen_end {
        return Err("Declared PNG artifact is incomplete".into());
    }
    Ok(())
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().saturating_add(2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

fn verification_support_root(executable: &Path) -> PathBuf {
    if let Some(configured) = std::env::var_os("SKETCH3D_VERIFICATION_ROOT") {
        return PathBuf::from(configured)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from("."));
    }
    let parent = executable.parent().unwrap_or_else(|| Path::new("."));
    for candidate in parent.ancestors().take(4) {
        if candidate.join("scripts/run_verification.ps1").is_file() {
            return candidate.to_path_buf();
        }
        let macos_resources = candidate.join("Resources/verification-support");
        if macos_resources
            .join("scripts/run_verification.ps1")
            .is_file()
        {
            return macos_resources;
        }
    }
    parent.to_path_buf()
}

fn run_check(root: &Path, check_id: &str, arguments: &Value) -> Result<Value, String> {
    if !matches!(
        check_id,
        "startup" | "save-reopen" | "export" | "glb-texture-roundtrip" | "ui-workflow"
    ) {
        return Err("Unsupported check ID".into());
    }
    let timeout_seconds = arguments
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(180);
    if !(5..=MAX_RUN_TIMEOUT_SECONDS).contains(&timeout_seconds) {
        return Err(format!(
            "timeout_seconds must be between 5 and {MAX_RUN_TIMEOUT_SECONDS}"
        ));
    }

    // The texture verifier is a native command on the same release executable.
    // Calling it directly also avoids coupling this self-contained check to a
    // PowerShell install or its host-specific script policy.
    if check_id == "glb-texture-roundtrip" {
        return run_native_texture_check(root, timeout_seconds);
    }

    let executable = std::env::current_exe()
        .and_then(|path| fs::canonicalize(path))
        .map_err(|error| format!("Cannot resolve the installed Sketch3D executable: {error}"))?;
    let install_root = verification_support_root(&executable);
    let mut project_root = install_root.clone();
    let mut runner = project_root.join("scripts/run_verification.ps1");
    // Development builds run directly from target/{debug,release}; retain a
    // source-tree fallback for local workflows. Release packages carry the
    // runner and fixtures alongside the executable, so installed execution
    // never depends on the machine that compiled the binary.
    if !runner.is_file() {
        if std::env::var_os("SKETCH3D_VERIFICATION_ROOT").is_none() {
            let source_root = Path::new(env!("CARGO_MANIFEST_DIR"));
            let source_runner = source_root.join("scripts/run_verification.ps1");
            if source_runner.is_file()
                && source_root
                    .canonicalize()
                    .is_ok_and(|source_root| executable.starts_with(source_root))
            {
                project_root = source_root.to_path_buf();
                runner = source_runner;
            }
        }
    }
    if !runner.is_file() {
        return Err("Shared verification runner is unavailable. Install the verification-support files beside Sketch3D or set SKETCH3D_VERIFICATION_ROOT.".into());
    }
    let runner = runner
        .canonicalize()
        .map_err(|error| format!("Cannot resolve shared verification runner: {error}"))?;
    if !runner.starts_with(&project_root) {
        return Err(
            "Shared verification runner is outside the application source directory".into(),
        );
    }

    // PowerShell cannot bind `$PSScriptRoot` correctly when `-File` receives
    // Windows' extended `\\?\` path returned by canonicalize(). Keep the
    // canonical paths for containment checks, but pass ordinary DOS paths to
    // the child process.
    let runner_argument = if cfg!(windows) {
        runner
            .to_string_lossy()
            .strip_prefix(r"\\?\")
            .map(PathBuf::from)
            .unwrap_or_else(|| runner.clone())
    } else {
        runner.clone()
    };
    let output_argument = if cfg!(windows) {
        root.to_string_lossy()
            .strip_prefix(r"\\?\")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.to_path_buf())
    } else {
        root.to_path_buf()
    };
    let executable_argument = if cfg!(windows) {
        executable
            .to_string_lossy()
            .strip_prefix(r"\\?\")
            .map(PathBuf::from)
            .unwrap_or_else(|| executable.clone())
    } else {
        executable.clone()
    };
    let mut command = Command::new(if cfg!(windows) { "pwsh.exe" } else { "pwsh" });
    command
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&runner_argument)
        .arg("-Executable")
        .arg(&executable_argument)
        .arg("-Check")
        .arg(check_id)
        .arg("-OutputDirectory")
        .arg(&output_argument)
        .arg("-TimeoutSeconds")
        .arg(timeout_seconds.to_string())
        .current_dir(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return persist_tool_result(
                root,
                check_id,
                "unsupported",
                "PowerShell 7 (pwsh) is not installed or is not on PATH.",
                None,
            );
        }
        Err(error) => {
            return persist_tool_result(
                root,
                check_id,
                "failed",
                &format!("Could not start the shared runner: {error}"),
                None,
            );
        }
    };

    let stdout_reader = child
        .stdout
        .take()
        .map(|stream| thread::spawn(move || read_capped(stream, MAX_PROCESS_LOG_BYTES)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stream| thread::spawn(move || read_capped(stream, MAX_PROCESS_LOG_BYTES)));
    let outer_timeout = Duration::from_secs(timeout_seconds + RUNNER_TIMEOUT_PADDING_SECONDS);
    let started = Instant::now();
    let status = loop {
        if MCP_CLIENT_DISCONNECTED.load(Ordering::Relaxed) {
            terminate_process_tree(&mut child);
            let _ = child.wait();
            return Err("MCP client disconnected; stopped the verifier process tree.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < outer_timeout => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                terminate_process_tree(&mut child);
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                terminate_process_tree(&mut child);
                let _ = child.wait();
                return Err(format!("Could not wait for the shared runner: {error}"));
            }
        }
    };
    let stdout = stdout_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();

    if status.is_none() {
        return persist_tool_result(
            root,
            check_id,
            "timed_out",
            &format!(
                "Shared runner exceeded its {} second outer time limit.",
                outer_timeout.as_secs()
            ),
            Some((&stdout, &stderr)),
        );
    }

    let stdout_text = String::from_utf8_lossy(&stdout);
    let result_path = stdout_text
        .lines()
        .find_map(|line| line.trim().strip_prefix("Normalized result: "))
        .map(PathBuf::from)
        .ok_or_else(|| {
            let detail = String::from_utf8_lossy(&stderr);
            if detail.trim().is_empty() {
                "Shared runner did not report its normalized result path".to_owned()
            } else {
                format!(
                    "Shared runner failed: {}",
                    detail.lines().last().unwrap_or_default()
                )
            }
        })?;
    let result_path = result_path
        .canonicalize()
        .map_err(|error| format!("Shared runner result is unavailable: {error}"))?;
    let run_directory = result_path
        .parent()
        .ok_or("Shared result has no run directory")?;
    if run_directory.parent() != Some(root)
        || result_path.file_name().and_then(|name| name.to_str()) != Some("result.json")
        || !result_path.starts_with(root)
    {
        return Err("Shared runner result is outside the configured verification directory".into());
    }
    let run_id = run_directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Shared result has an invalid run directory name")?;
    if !valid_run_id(run_id) {
        return Err("Shared runner returned an invalid run ID".into());
    }
    let result = read_result(root, run_id)?;
    let check_result = result
        .get("results")
        .and_then(Value::as_array)
        .and_then(|checks| {
            checks
                .iter()
                .find(|item| item.get("check_id").and_then(Value::as_str) == Some(check_id))
        })
        .ok_or("Normalized result omitted the requested check")?;
    let artifacts = list_artifacts(root, run_id, check_id)?;
    let exit_code = status.and_then(|status| status.code());
    Ok(json!({
        "run_id": run_id,
        "check": check_result,
        "artifacts": artifacts.get("artifacts").cloned().unwrap_or_else(|| json!([])),
        "process_exit_code": exit_code,
        "result_schema_version": 1
    }))
}

fn run_native_texture_check(root: &Path, timeout_seconds: u64) -> Result<Value, String> {
    let run_id = unique_tool_run_id();
    let run_directory = root.join(&run_id);
    fs::create_dir(&run_directory).map_err(|error| error.to_string())?;
    let check_directory = run_directory.join("glb-texture-roundtrip-original");
    fs::create_dir(&check_directory).map_err(|error| error.to_string())?;
    let stdout_path = run_directory.join("glb-texture-roundtrip.stdout.log");
    let stderr_path = run_directory.join("glb-texture-roundtrip.stderr.log");
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let executable_hash = sha256_file(&executable);
    let fixture_path = executable
        .parent()
        .map(|parent| parent.join("src/glb_texture_roundtrip_check.rs"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/glb_texture_roundtrip_check.rs")
        });
    let fixture_hash = sha256_file(&fixture_path);
    let mut command = Command::new(&executable);
    command
        .arg("--verify-glb-texture-roundtrip")
        .arg(&check_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start native GLB verifier: {error}"))?;
    let stdout_reader = child
        .stdout
        .take()
        .map(|stream| thread::spawn(move || read_capped(stream, MAX_PROCESS_LOG_BYTES)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stream| thread::spawn(move || read_capped(stream, MAX_PROCESS_LOG_BYTES)));
    let started = Instant::now();
    let status = loop {
        if MCP_CLIENT_DISCONNECTED.load(Ordering::Relaxed) {
            terminate_process_tree(&mut child);
            let _ = child.wait();
            return Err("MCP client disconnected; stopped the verifier process tree.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < Duration::from_secs(timeout_seconds) => {
                thread::sleep(Duration::from_millis(25))
            }
            Ok(None) => {
                terminate_process_tree(&mut child);
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                terminate_process_tree(&mut child);
                let _ = child.wait();
                return Err(format!("Could not wait for native GLB verifier: {error}"));
            }
        }
    };
    let stdout = stdout_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    fs::write(&stdout_path, &stdout).map_err(|error| error.to_string())?;
    fs::write(&stderr_path, &stderr).map_err(|error| error.to_string())?;
    let timed_out = status.is_none();
    let exit_code = status.as_ref().and_then(|status| status.code());
    let outcome = if timed_out {
        "timed_out"
    } else if exit_code == Some(0) {
        "passed"
    } else {
        "failed"
    };
    let failure_reason = if outcome == "timed_out" {
        Some(format!("Timed out after {timeout_seconds} seconds."))
    } else if outcome == "failed" {
        let detail = String::from_utf8_lossy(if stderr.is_empty() { &stdout } else { &stderr });
        Some(
            detail
                .lines()
                .last()
                .unwrap_or("Native GLB verifier failed.")
                .to_owned(),
        )
    } else {
        None
    };
    let mut artifacts = vec![
        "glb-texture-roundtrip.stdout.log".to_owned(),
        "glb-texture-roundtrip.stderr.log".to_owned(),
    ];
    for path in [
        "glb-texture-roundtrip-report.json",
        "texture-preview.png",
        "roundtrip.sketch.json",
        "whole-scene.glb",
    ] {
        if check_directory.join(path).is_file() {
            artifacts.push(format!("glb-texture-roundtrip-original/{path}"));
        }
    }
    let result = json!({
        "schema_version": 1,
        "run_id": run_id,
        "generated_utc": utc_timestamp(),
        "build": {
            "version": env!("CARGO_PKG_VERSION"), "version_status": "available",
            "file_version": Value::Null, "product_version": env!("CARGO_PKG_VERSION"),
            "fingerprint_algorithm": "sha256", "fingerprint": executable_hash, "fingerprint_status": if executable_hash.is_some() { "available" } else { "unavailable" }
        },
        "source_revision": {"value": Value::Null, "status": "unavailable"},
        "results": [{
            "check_id": "glb-texture-roundtrip", "outcome": outcome,
            "failure_reason": failure_reason, "exit_code": exit_code,
            "fixture": {"id": "textured-glb-managed-native-roundtrip-v1", "fingerprint_algorithm": "sha256", "fingerprint": fixture_hash, "fingerprint_status": if fixture_hash.is_some() { "available" } else { "unavailable" }},
            "rendering": {"status":"not_run","reason":"Interactive viewport rendering is tracked separately from this structural check."}, "artifacts": artifacts
        }]
    });
    let result_path = run_directory.join("result.json");
    fs::write(
        &result_path,
        serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let check = &result["results"][0];
    let artifact_rows: Vec<_> = artifacts.iter().map(|path| {
        let size = fs::metadata(run_directory.join(path)).map(|metadata| metadata.len()).unwrap_or(0);
        json!({"path":path,"size_bytes":size,"artifact_type":artifact_type(path).unwrap_or("unsupported")})
    }).collect();
    Ok(
        json!({"run_id":run_id,"check":check,"artifacts":artifact_rows,"process_exit_code":exit_code,"result_schema_version":1}),
    )
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

/// Stop the complete verifier process tree after timeout or a supervision
/// error. A runner can start the actual app as a child, so killing only the
/// shell would leave that app running without a client.
fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let killed = Command::new("taskkill.exe")
            .args(["/PID", &pid, "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !killed {
            let _ = child.kill();
        }
    }
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let killed = Command::new("kill")
            .args(["-KILL", "--", &process_group])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !killed {
            let _ = child.kill();
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = child.kill();
    }
}

fn persist_tool_result(
    root: &Path,
    check_id: &str,
    outcome: &str,
    reason: &str,
    logs: Option<(&[u8], &[u8])>,
) -> Result<Value, String> {
    let run_id = unique_tool_run_id();
    let run_directory = root.join(&run_id);
    fs::create_dir(&run_directory).map_err(|error| error.to_string())?;
    let (stdout, stderr) = logs.unwrap_or((&[], &[]));
    let stdout_path = run_directory.join(format!("{check_id}.stdout.log"));
    let stderr_path = run_directory.join(format!("{check_id}.stderr.log"));
    fs::write(&stdout_path, stdout).map_err(|error| error.to_string())?;
    fs::write(&stderr_path, stderr).map_err(|error| error.to_string())?;
    let fixture_id = match check_id {
        "startup" => "startup-smoke-v1",
        "save-reopen" => "artwork-animation-roundtrip-v1",
        "export" => "still-export-visual-v1",
        _ => "unknown",
    };
    let result = json!({
        "schema_version": 1,
        "run_id": run_id,
        "generated_utc": utc_timestamp(),
        "build": {
            "version": env!("CARGO_PKG_VERSION"),
            "version_status": "available",
            "file_version": Value::Null,
            "product_version": env!("CARGO_PKG_VERSION"),
            "fingerprint_algorithm": "sha256",
            "fingerprint": Value::Null,
            "fingerprint_status": "unavailable"
        },
        "source_revision": {"value": Value::Null, "status": "unavailable"},
        "results": [{
            "check_id": check_id,
            "outcome": outcome,
            "failure_reason": reason,
            "exit_code": Value::Null,
            "fixture": {
                "id": fixture_id,
                "fingerprint_algorithm": "sha256",
                "fingerprint": Value::Null,
                "fingerprint_status": if check_id == "startup" { "not_applicable" } else { "unavailable" }
            },
            "rendering": if check_id == "export" { json!({"status":"unavailable"}) } else { Value::Null },
            "artifacts": [
                format!("{check_id}.stdout.log"),
                format!("{check_id}.stderr.log")
            ]
        }]
    });
    fs::write(
        run_directory.join("result.json"),
        serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(json!({
        "run_id": run_id,
        "check": result["results"][0],
        "artifacts": [
            {"path": format!("{check_id}.stdout.log"), "size_bytes": stdout.len()},
            {"path": format!("{check_id}.stderr.log"), "size_bytes": stderr.len()}
        ],
        "result_schema_version": 1
    }))
}

fn unique_tool_run_id() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let sequence = NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed);
    format!("mcp-{seconds}-{}-{sequence}", std::process::id())
}

fn utc_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
    let month = month_piece + if month_piece < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        day_seconds / 3_600,
        (day_seconds % 3_600) / 60,
        day_seconds % 60
    )
}

fn read_capped<R: Read>(mut reader: R, maximum: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(maximum.min(4096));
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = maximum.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..count.min(remaining)]);
            }
        }
    }
    output
}

fn sha256_file(path: &Path) -> Option<String> {
    let argument = if cfg!(windows) {
        path.strip_prefix(r"\\?\")
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    };
    let output = if cfg!(windows) {
        Command::new("certutil.exe")
            .arg("-hashfile")
            .arg(argument)
            .arg("SHA256")
            .output()
    } else {
        Command::new("sha256sum").arg(argument).output()
    }
    .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .find(|word| word.len() == 64 && word.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|hash| hash.to_ascii_lowercase())
}

fn valid_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
