//! The stdio MCP lifecycle and five tools. Tool calls use the same socket
//! protocol as the CLI; initialization and discovery require no open TUI.

use super::{Control, Request, transport};
use anyhow::{Result, ensure};
use clap::ValueEnum;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
};

const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

pub fn serve(workspace: PathBuf) -> Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut line = Vec::new();
    let mut server = StdioServer {
        workspace: &workspace,
        protocol: None,
        ready: false,
    };
    loop {
        line.clear();
        if (&mut input)
            .take(transport::MAX_REQUEST as u64 + 1)
            .read_until(b'\n', &mut line)?
            == 0
        {
            return Ok(());
        }
        ensure!(
            line.len() <= transport::MAX_REQUEST,
            "MCP message exceeded the size limit"
        );
        if let Some(response) = server.handle(&line) {
            serde_json::to_writer(&mut output, &response)?;
            writeln!(output)?;
            output.flush()?;
        }
    }
}

struct StdioServer<'a> {
    workspace: &'a Path,
    protocol: Option<&'static str>,
    ready: bool,
}

impl StdioServer<'_> {
    fn handle(&mut self, line: &[u8]) -> Option<Value> {
        let message: Value = match serde_json::from_slice(line) {
            Ok(message) => message,
            Err(_) => return Some(error(&Value::Null, -32700, "Invalid JSON")),
        };
        let id = message.get("id");
        let method = message.get("method").and_then(Value::as_str);
        if message["jsonrpc"] != "2.0"
            || method.is_none()
            || id.is_some_and(|id| !(id.is_string() || id.is_i64() || id.is_u64()))
        {
            return Some(error(&Value::Null, -32600, "Invalid JSON-RPC request"));
        }
        let params = &message["params"];
        let Some(id) = id else {
            if method == Some("notifications/initialized") && self.protocol.is_some() {
                self.ready = true;
            }
            return None;
        };
        let result = match method {
            Some("initialize") => {
                let Some(requested) = params["protocolVersion"].as_str() else {
                    return Some(error(id, -32602, "A protocolVersion is required"));
                };
                let version = VERSIONS
                    .iter()
                    .copied()
                    .find(|version| *version == requested)
                    .unwrap_or(VERSIONS[0]);
                self.protocol = Some(version);
                self.ready = false;
                json!({
                    "protocolVersion": version,
                    "serverInfo": {"name": "flimru", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"tools": {}},
                    "instructions": "Use Flimru to start, inspect, and control Flutter runs in this workspace. The user opens the Flimru TUI. List sessions before launching, use exact target values from flimru_targets, and keep the next cursor from flimru_logs."
                })
            }
            Some("ping") => json!({}),
            Some("tools/list") if self.ready => json!({"tools": tools()}),
            Some("tools/call") if self.ready => match self.call(params) {
                Some(result) => result,
                None => return Some(error(id, -32602, "Unknown MCP tool")),
            },
            Some("tools/list" | "tools/call") => {
                return Some(error(id, -32002, "Initialize the MCP session first"));
            }
            _ => return Some(error(id, -32601, "Method not found")),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn call(&self, params: &Value) -> Option<Value> {
        let tool = params["name"].as_str()?;
        let action = match tool {
            "flimru_targets" => "targets",
            "flimru_sessions" => "sessions",
            "flimru_start" => "start",
            "flimru_logs" => "logs",
            "flimru_control" => "control",
            _ => return None,
        };
        let result = (|| -> Result<Value> {
            let mut args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let Some(fields) = args.as_object_mut() else {
                anyhow::bail!("Tool arguments must be an object.");
            };
            ensure!(
                !fields.contains_key("action") && !fields.contains_key("until"),
                "Unexpected tool argument."
            );
            fields.insert("action".into(), json!(action));
            let request: Request = serde_json::from_value(args)?;
            if let Request::Logs { limit, .. } = &request {
                ensure!(
                    (4..=flimru::session_log::MAX_READ).contains(limit),
                    "Log limit must be between 4 and 65536 bytes."
                );
            }
            transport::request(self.workspace, request)
        })();
        Some(match result {
            Ok(value) => {
                let mut reply = json!({"content": [{"type": "text", "text": value.to_string()}]});
                if matches!(self.protocol, Some("2025-11-25" | "2025-06-18")) {
                    reply["structuredContent"] = value;
                }
                reply
            }
            Err(error) => {
                json!({"content": [{"type": "text", "text": format!("{error:#}")}], "isError": true})
            }
        })
    }
}

fn error(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tools() -> [Value; 5] {
    let id = || json!({"type": "string", "description": "Full session ID returned by Flimru."});
    [
        tool(
            "flimru_targets",
            "List exact device IDs and VS Code launch configurations known to the open Flimru window. Use next_config_offset to get the next configuration page.",
            json!({"config_offset": {"type": "integer", "minimum": 0, "default": 0}}),
            &[],
            true,
        ),
        tool(
            "flimru_sessions",
            "List live and retained ended sessions, or inspect one by ID. Check state, exit outcome and control availability. A null control reason means available.",
            json!({"id": id()}),
            &[],
            true,
        ),
        tool(
            "flimru_start",
            "Start one run in the open Flimru window. Provide the exact device ID and either config (exact name) or config_index (one-based). Save the returned session ID.",
            json!({
                "device": {"type": "string", "description": "Exact device ID from flimru_targets."},
                "config": {"type": "string", "description": "Exact unambiguous configuration name."},
                "config_index": {"type": "integer", "minimum": 1, "description": "One-based configuration index from flimru_targets."}
            }),
            &["device"],
            false,
        ),
        tool(
            "flimru_logs",
            "Read bounded plain-text console output and the run state. Omit cursor for recent output, use 0 for the earliest retained output, or pass the previous cursor for later output. A truncated result indicates omitted earlier output. Call again for more data.",
            json!({
                "id": id(),
                "cursor": {"type": "integer", "minimum": 0, "description": "Byte cursor returned by the preceding read of this session."},
                "limit": {"type": "integer", "minimum": 4, "maximum": flimru::session_log::MAX_READ, "default": super::MCP_LOG_BYTES}
            }),
            &["id"],
            true,
        ),
        tool(
            "flimru_control",
            "Request a named control on one Flutter session. Reload and restart report delivery to Flimru; read logs for Flutter's result. Stop preserves the other sessions and the window.",
            json!({
                "id": id(),
                "control": {"type": "string", "enum": Control::value_variants()}
            }),
            &["id", "control"],
            false,
        ),
    ]
}

fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        },
        "annotations": {"readOnlyHint": read_only, "openWorldHint": false}
    })
}
