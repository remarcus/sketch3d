//! Read-only MCP access to normalized verification reports.
//!
//! Start explicitly with `--mcp-verification-server <verification-directory>`.
//! The directory is canonicalized once, and every reported file is checked
//! against the selected run directory before it is returned.

use serde_json::{json, Value};
use std::{
    fs,
    io::{self, BufRead, Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
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
static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(0);

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
        },
        {
            "name": "verification_run_check",
            "description": "Run one allowlisted local verifier in a unique output run directory and return its normalized result. This starts the configured Sketch3D executable and is bounded by timeout_seconds.",
            "inputSchema": {"type": "object", "properties": {"check_id": {"type": "string", "enum": ["startup", "save-reopen", "export"]}, "timeout_seconds": {"type": "integer", "minimum": 5, "maximum": 600, "default": 180}}, "required": ["check_id"], "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}
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

fn run_check(root: &Path, check_id: &str, arguments: &Value) -> Result<Value, String> {
    if !matches!(check_id, "startup" | "save-reopen" | "export") {
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

    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runner = project_root.join("scripts/run_verification.ps1");
    if !runner.is_file() {
        return Err("Shared verification runner is unavailable in this installation".into());
    }
    let runner = runner
        .canonicalize()
        .map_err(|error| format!("Cannot resolve shared verification runner: {error}"))?;
    if !runner.starts_with(project_root) {
        return Err("Shared verification runner is outside the application source directory".into());
    }

    let mut command = Command::new(if cfg!(windows) { "pwsh.exe" } else { "pwsh" });
    command
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-File")
        .arg(&runner)
        .arg("-Check")
        .arg(check_id)
        .arg("-OutputDirectory")
        .arg(root)
        .arg("-TimeoutSeconds")
        .arg(timeout_seconds.to_string())
        .current_dir(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

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
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < outer_timeout => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
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
                format!("Shared runner failed: {}", detail.lines().last().unwrap_or_default())
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
        .and_then(|checks| checks.iter().find(|item| item.get("check_id").and_then(Value::as_str) == Some(check_id)))
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
    let result = json!({
        "schema_version": 1,
        "run_id": run_id,
        "generated_utc": utc_timestamp(),
        "results": [{
            "check_id": check_id,
            "outcome": outcome,
            "failure_reason": reason,
            "exit_code": Value::Null,
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
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
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

fn valid_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
