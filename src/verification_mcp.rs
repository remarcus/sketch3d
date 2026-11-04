//! Read-only MCP access to normalized verification reports.
//!
//! Start explicitly with `--mcp-verification-server <verification-directory>`.
//! The directory is canonicalized once, and every reported file is checked
//! against the selected run directory before it is returned.

use serde_json::{json, Value};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Component, Path, PathBuf},
};

const PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RESULT_BYTES: u64 = 512 * 1024;
const MAX_ARTIFACTS: usize = 256;
const MAX_RUNS: usize = 100;
const MAX_SCANNED_ENTRIES: usize = 1000;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub fn serve(configured_root: &Path) -> Result<(), String> {
    let root = configured_root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve verification directory: {error}"))?;
    if !root.is_dir() {
        return Err("Configured verification path is not a directory".into());
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    loop {
        let mut line = Vec::new();
        match read_bounded_line(&mut input, &mut line)? {
            ReadLine::Eof => return Ok(()),
            ReadLine::Oversized => {
                write_message(
                    &mut output,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {"code": -32600, "message": "Request exceeds the 64 KiB limit"}
                    }),
                )?;
            }
            ReadLine::Line => {
                let parsed = serde_json::from_slice::<Value>(&line);
                let response = match parsed {
                    Ok(message) => handle_message(&root, message),
                    Err(error) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {"code": -32700, "message": format!("Invalid JSON: {error}")}
                    })),
                };
                if let Some(response) = response {
                    write_message(&mut output, &response)?;
                }
            }
        }
    }
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

fn handle_message(root: &Path, message: Value) -> Option<Value> {
    let id = message.get("id").cloned();
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return id.map(|id| rpc_error(id, -32600, "Expected JSON-RPC 2.0"));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return id.map(|id| rpc_error(id, -32600, "Missing JSON-RPC method"));
    };

    match method {
        "initialize" => {
            let Some(id) = id else { return None };
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
                    "serverInfo": {"name": "sketch3d-verification-reports", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "Read-only access to normalized verification results beneath the configured verification directory."
                }),
            ))
        }
        "notifications/initialized" | "notifications/cancelled" | "notifications/progress" => None,
        "ping" => id.map(|id| rpc_result(id, json!({}))),
        "tools/list" => id.map(|id| rpc_result(id, json!({ "tools": tool_definitions() }))),
        "tools/call" => {
            let Some(id) = id else { return None };
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let result = call_tool(root, name, &arguments);
            Some(rpc_result(id, result))
        }
        _ if id.is_none() => None,
        _ => id.map(|id| rpc_error(id, -32601, "Method not found")),
    }
}

fn supported_protocol(version: &str) -> bool {
    matches!(
        version,
        "2025-06-18" | "2025-11-25"
    )
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_definitions() -> Value {
    json!([
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
            "name": "verification_list_artifacts",
            "description": "List the bounded artifact paths and sizes recorded for one check in a completed run.",
            "inputSchema": {"type": "object", "properties": {"run_id": {"type": "string", "maxLength": 64}, "check_id": {"type": "string", "enum": ["startup", "save-reopen", "export"]}}, "required": ["run_id", "check_id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
        }
    ])
}

fn call_tool(root: &Path, name: &str, arguments: &Value) -> Value {
    let result = match name {
        "verification_list_checks" => list_checks(root),
        "verification_read_result" => required_string(arguments, "run_id")
            .and_then(|run_id| read_result(root, run_id)),
        "verification_list_artifacts" => required_string(arguments, "run_id").and_then(|run_id| {
            required_string(arguments, "check_id")
                .and_then(|check_id| list_artifacts(root, run_id, check_id))
        }),
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
    runs.sort_by(|left, right| right.0.cmp(&left.0));
    let completed = runs
        .into_iter()
        .take(MAX_RUNS)
        .map(|(_, run_id)| run_id)
        .collect::<Vec<_>>();
    Ok(json!({
        "checks": [
            {"check_id":"startup", "description":"Application startup smoke check"},
            {"check_id":"save-reopen", "description":"Animation native save/reopen check"},
            {"check_id":"export", "description":"Still image export check"}
        ],
        "completed_runs": completed,
        "run_limit": MAX_RUNS,
        "scan_limit": MAX_SCANNED_ENTRIES
    }))
}

fn read_result(root: &Path, run_id: &str) -> Result<Value, String> {
    let (_, _, bytes) = result_file(root, run_id)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| format!("Invalid result JSON: {error}"))?;
    if value.get("schema_version").and_then(Value::as_u64) != Some(1)
        || !value.get("results").is_some_and(Value::is_array)
    {
        return Err("Run result does not use schema version 1".into());
    }
    Ok(value)
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
    if !matches!(check_id, "startup" | "save-reopen" | "export") {
        return Err("Unsupported check ID".into());
    }
    let (_, run_directory, bytes) = result_file(root, run_id)?;
    let result: Value = serde_json::from_slice(&bytes).map_err(|error| format!("Invalid result JSON: {error}"))?;
    if result.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err("Run result does not use schema version 1".into());
    }
    let check = result
        .get("results")
        .and_then(Value::as_array)
        .and_then(|checks| checks.iter().find(|item| item.get("check_id").and_then(Value::as_str) == Some(check_id)))
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
        let relative = relative
            .as_str()
            .ok_or("Artifact path is not a string")?;
        let path = Path::new(relative);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("Artifact path is not a safe relative path".into());
        }
        let canonical = run_directory
            .join(path)
            .canonicalize()
            .map_err(|_| format!("Artifact is unavailable: {relative}"))?;
        if !canonical.starts_with(&run_directory) || !canonical.is_file() {
            return Err("Artifact path escapes its run directory".into());
        }
        let metadata = fs::metadata(&canonical).map_err(|error| error.to_string())?;
        artifacts.push(json!({"path": relative, "size_bytes": metadata.len()}));
    }
    Ok(json!({
        "run_id": run_id,
        "check_id": check_id,
        "artifacts": artifacts,
        "artifact_limit": MAX_ARTIFACTS
    }))
}

fn valid_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
