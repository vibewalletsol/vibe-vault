// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
// Portions derived from Cinder (https://github.com/gregnb/cinder),
// Copyright (c) 2026 Gregory Nowakowski, MIT License (see LICENSES/MIT-Cinder.txt).
//! MCP stdio server. Read tools are live RPC. Write tools open vibe-vault Confirm.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};
use vibe_core::mcp;
use vibe_core::store;

fn main() {
    let rpc = store::load_settings(&store::settings_path()).rpc_url();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if req.get("method").and_then(|m| m.as_str()) == Some("notifications/initialized") {
            continue;
        }
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "vibe-mcp", "version": vibe_core::APP_VERSION }
            }),
            "tools/list" => json!({ "tools": mcp::tools_list_json() }),
            "tools/call" => {
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match mcp::dispatch(name, &args, &rpc) {
                    Ok(v) => json!({
                        "content": [{ "type": "text", "text": v.to_string() }],
                        "isError": false
                    }),
                    Err(e) => json!({
                        "content": [{ "type": "text", "text": e.to_string() }],
                        "isError": true
                    }),
                }
            }
            "ping" => json!({}),
            _ => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    json!({"jsonrpc":"2.0","id": id, "error": {"code": -32601, "message": format!("unknown method {method}")}})
                );
                let _ = stdout.flush();
                continue;
            }
        };
        let _ = writeln!(stdout, "{}", json!({"jsonrpc":"2.0","id": id, "result": result}));
        let _ = stdout.flush();
    }
}
