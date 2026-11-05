//! Opt-in read-only MCP bridge to a running Sketch3D UI session.
//!
//! The JSON-RPC stdio loop runs off the UI thread. Requests that need live
//! application state are handed to the egui thread through a bounded channel.

use image::ImageEncoder;
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Write},
    sync::atomic::{AtomicU64, Ordering},
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    time::Duration,
};

const PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PNG_BYTES: usize = 1_000_000;
const MAX_PNG_DIMENSION: usize = 4096;
const UI_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
static NEXT_PREVIEW_ID: AtomicU64 = AtomicU64::new(0);

pub enum LiveUiCommand {
    Snapshot,
    CaptureViewport,
    PreviewColor {
        revision: String,
        color: [f32; 3],
    },
    ApplyEdit {
        preview_id: String,
        revision: String,
    },
    CancelEdit {
        preview_id: String,
    },
}

pub struct LiveUiRequest {
    pub command: LiveUiCommand,
    pub reply: SyncSender<Result<Value, String>>,
}

pub fn channel() -> (Sender<LiveUiRequest>, Receiver<LiveUiRequest>) {
    mpsc::channel()
}

pub fn serve(ui_requests: Sender<LiveUiRequest>) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    loop {
        let mut line = Vec::new();
        match read_bounded_line(&mut input, &mut line).map_err(|error| error.to_string())? {
            ReadLine::Eof => return Ok(()),
            ReadLine::Oversized => write_message(
                &mut output,
                &json!({
                    "jsonrpc":"2.0", "id":null,
                    "error":{"code":-32600,"message":"Request exceeds the 64 KiB limit"}
                }),
            )
            .map_err(|error| error.to_string())?,
            ReadLine::Line => {
                let response = match serde_json::from_slice::<Value>(&line) {
                    Ok(message) => handle_message(&ui_requests, message),
                    Err(error) => Some(json!({
                        "jsonrpc":"2.0", "id":null,
                        "error":{"code":-32700,"message":format!("Invalid JSON: {error}")}
                    })),
                };
                if let Some(response) = response {
                    write_message(&mut output, &response).map_err(|error| error.to_string())?;
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
        let ended = available.get(take.saturating_sub(1)) == Some(&b'\n');
        let oversized = output.len().saturating_add(take) > MAX_REQUEST_BYTES;
        if !oversized {
            output.extend_from_slice(&available[..take]);
        }
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
        serde_json::to_writer(
            &mut *writer,
            &json!({
                "jsonrpc":"2.0",
                "id":message.get("id").cloned().unwrap_or(Value::Null),
                "error":{"code":-32603,"message":"Response exceeds the 2 MiB limit"}
            }),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
    } else {
        writer.write_all(&encoded)?;
    }
    writer.write_all(b"\n")?;
    writer.flush()
}

fn handle_message(ui_requests: &Sender<LiveUiRequest>, message: Value) -> Option<Value> {
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
            let version = if matches!(requested, "2025-06-18" | "2025-11-25") {
                requested
            } else {
                PROTOCOL_VERSION
            };
            Some(rpc_result(
                id,
                json!({
                    "protocolVersion":version,
                    "capabilities":{"tools":{"listChanged":false}},
                    "serverInfo":{"name":"sketch3d-live-session","version":env!("CARGO_PKG_VERSION")},
                "instructions":"Access the active Sketch3D session. Snapshot and viewport tools are read-only; one selected-stroke solid color edit is available through explicit preview, apply, or cancel calls. Local file paths are not returned. Unsaved state is identified explicitly."
                }),
            ))
        }
        "notifications/initialized" | "notifications/cancelled" | "notifications/progress" => None,
        "ping" => id.map(|id| rpc_result(id, json!({}))),
        "tools/list" => id.map(|id| rpc_result(id, json!({"tools":tools()}))),
        "tools/call" => {
            let id = id?;
            let name = message
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = message
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let command = match name {
                "live_document_snapshot" => LiveUiCommand::Snapshot,
                "live_viewport_capture" => LiveUiCommand::CaptureViewport,
                "live_preview_selected_color" => {
                    let revision = match arguments.get("revision").and_then(Value::as_str) {
                        Some(value) if !value.is_empty() && value.len() <= 80 => value.to_owned(),
                        _ => {
                            return Some(rpc_result(id, tool_error("A valid revision is required")))
                        }
                    };
                    let Some(values) = arguments.get("color").and_then(Value::as_array) else {
                        return Some(rpc_result(
                            id,
                            tool_error("Color must contain three numeric components"),
                        ));
                    };
                    let parsed: Option<Vec<f32>> = values
                        .iter()
                        .map(|value| {
                            value.as_f64().and_then(|number| {
                                (number.is_finite() && (0.0..=1.0).contains(&number))
                                    .then_some(number as f32)
                            })
                        })
                        .collect();
                    let Some(values) = parsed.filter(|values| values.len() == 3) else {
                        return Some(rpc_result(
                            id,
                            tool_error("Color must contain three values from 0 to 1"),
                        ));
                    };
                    LiveUiCommand::PreviewColor {
                        revision,
                        color: [values[0], values[1], values[2]],
                    }
                }
                "live_apply_selected_color" => {
                    let Some(preview_id) = bounded_string(&arguments, "preview_id", 96) else {
                        return Some(rpc_result(id, tool_error("A valid preview_id is required")));
                    };
                    let Some(revision) = bounded_string(&arguments, "revision", 80) else {
                        return Some(rpc_result(id, tool_error("A valid revision is required")));
                    };
                    LiveUiCommand::ApplyEdit {
                        preview_id,
                        revision,
                    }
                }
                "live_cancel_selected_color" => {
                    let Some(preview_id) = bounded_string(&arguments, "preview_id", 96) else {
                        return Some(rpc_result(id, tool_error("A valid preview_id is required")));
                    };
                    LiveUiCommand::CancelEdit { preview_id }
                }
                _ => return Some(rpc_result(id, tool_error("Unknown tool"))),
            };
            Some(rpc_result(id, call_ui(ui_requests, command)))
        }
        _ if id.is_none() => None,
        _ => id.map(|id| rpc_error(id, -32601, "Method not found")),
    }
}

fn tools() -> Value {
    json!([
        {
            "name":"live_document_snapshot",
            "description":"Read the active document identity, change revision, bounded counts, selected stroke IDs, and unsaved state from the live UI session. No drawing content or local file path is returned.",
            "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}
        },
        {
            "name":"live_viewport_capture",
            "description":"Capture the current application viewport as a bounded PNG image.",
            "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}
        },
        {
            "name":"live_preview_selected_color",
            "description":"Preview a solid color edit on the current selected strokes, bound to the revision returned by live_document_snapshot.",
            "inputSchema":{"type":"object","properties":{"revision":{"type":"string","maxLength":80},"color":{"type":"array","items":{"type":"number","minimum":0,"maximum":1},"minItems":3,"maxItems":3}},"required":["revision","color"],"additionalProperties":false},
            "annotations":{"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false}
        },
        {
            "name":"live_apply_selected_color",
            "description":"Apply the active selected-stroke color preview as one undoable edit if the revision is still current.",
            "inputSchema":{"type":"object","properties":{"preview_id":{"type":"string","maxLength":96},"revision":{"type":"string","maxLength":80}},"required":["preview_id","revision"],"additionalProperties":false},
            "annotations":{"readOnlyHint":false,"destructiveHint":true,"openWorldHint":false}
        },
        {
            "name":"live_cancel_selected_color",
            "description":"Cancel the selected-stroke color preview and restore its exact source values.",
            "inputSchema":{"type":"object","properties":{"preview_id":{"type":"string","maxLength":96}},"required":["preview_id"],"additionalProperties":false},
            "annotations":{"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false}
        }
    ])
}

fn bounded_string(arguments: &Value, field: &str, maximum: usize) -> Option<String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= maximum)
        .map(str::to_owned)
}

pub fn preview_id() -> String {
    format!(
        "live-{}-{}",
        std::process::id(),
        NEXT_PREVIEW_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn call_ui(ui_requests: &Sender<LiveUiRequest>, command: LiveUiCommand) -> Value {
    let (reply, response) = mpsc::sync_channel(1);
    if ui_requests.send(LiveUiRequest { command, reply }).is_err() {
        return tool_error("The live UI session has disconnected");
    }
    match response.recv_timeout(UI_REQUEST_TIMEOUT) {
        Ok(Ok(value)) if value.get("mimeType").and_then(Value::as_str) == Some("image/png") => {
            json!({"content":[{"type":"image","data":value["data"],"mimeType":"image/png"}],"structuredContent":{"width":value["width"],"height":value["height"],"size_bytes":value["size_bytes"]},"isError":false})
        }
        Ok(Ok(value)) => {
            json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&value).unwrap_or_else(|_|"{}".into())}],"structuredContent":value,"isError":false})
        }
        Ok(Err(error)) => tool_error(&error),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            tool_error("The UI did not answer within 15 seconds")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            tool_error("The live UI session has disconnected")
        }
    }
}

fn tool_error(message: &str) -> Value {
    json!({"content":[{"type":"text","text":message}],"isError":true})
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

pub fn encode_viewport_png(image: &eframe::egui::ColorImage) -> Result<Value, String> {
    let [width, height] = image.size;
    if width == 0 || height == 0 || width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION {
        return Err("Viewport dimensions exceed the 4096 × 4096 capture limit".into());
    }
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &rgba,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("Could not encode viewport capture: {error}"))?;
    if png.len() > MAX_PNG_BYTES {
        return Err("Viewport PNG exceeds the 1,000,000-byte capture limit".into());
    }
    Ok(json!({
        "mimeType":"image/png",
        "data":base64(&png),
        "width":width,
        "height":height,
        "size_bytes":png.len()
    }))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 3) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 15) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}
