//! MCP stdio server. Read tools are live RPC. Write tools open vibe-vault Confirm.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};
use vibe_core::mcp;
use vibe_core::store;

/// `vibe-mcp admin …` — used by the desk's Options → Agents page. Prints one JSON object.
fn admin(args: &[String]) -> i32 {
    use vibe_core::agents;
    let a = |i: usize| args.get(i).map(|s| s.as_str()).unwrap_or("");
    let num = |i: usize| a(i).trim().parse::<f64>().unwrap_or(0.0);
    let res: Result<Value, String> = (|| {
        Ok(match a(0) {
            "list" => {
                let reg = agents::load_registry();
                let st = store::load_store(&store::wallets_path()).map_err(|e| e.to_string())?;
                json!({
                    "agents": reg.agents.iter().filter(|x| !x.revoked).map(agents::agent_json).collect::<Vec<_>>(),
                    "tasks": agents::load_tasks().tasks.iter().rev().take(100).map(agents::task_json).collect::<Vec<_>>(),
                    "wallets": st.wallets.iter().map(|w| json!({"id": w.id, "name": w.name, "address": w.address, "key_type": w.key_type})).collect::<Vec<_>>(),
                    "tools": mcp::tools().iter().filter(|t| agents::tool_kind(t.name) != Some(agents::Kind::Meta)).map(|t| json!({
                        "name": t.name, "description": t.description,
                        "kind": format!("{:?}", agents::tool_kind(t.name).unwrap_or(agents::Kind::Read)).to_lowercase(),
                    })).collect::<Vec<_>>(),
                })
            }
            "create" => {
                let c = agents::create_agent(a(1), a(2), num(3), num(4)).map_err(|e| e.to_string())?;
                json!({"agent": agents::agent_json(&c.agent), "key": c.key})
            }
            "revoke" => {
                agents::revoke_agent(a(1)).map_err(|e| e.to_string())?;
                json!({"ok": true})
            }
            "set-level" => {
                let lv = agents::Level::parse(a(3)).ok_or("level must be blocked, approval or allowed")?;
                agents::set_level(a(1), a(2), lv).map_err(|e| e.to_string())?;
                json!({"ok": true})
            }
            "set-caps" => {
                agents::set_caps(a(1), num(2), num(3)).map_err(|e| e.to_string())?;
                json!({"ok": true})
            }
            "approve" => json!({"task": agents::task_json(&agents::approve(a(1)).map_err(|e| e.to_string())?)}),
            "reject" => json!({"task": agents::task_json(&agents::reject(a(1)).map_err(|e| e.to_string())?)}),
            other => return Err(format!("unknown admin command {other:?}")),
        })
    })();
    match res {
        Ok(v) => {
            println!("{v}");
            0
        }
        Err(e) => {
            println!("{}", json!({"error": e}));
            1
        }
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(|s| s.as_str()) == Some("admin") {
        std::process::exit(admin(&argv[1..]));
    }
    let rpc = store::load_settings(&store::settings_path()).rpc_url();
    // 1.9: per-agent key from the MCP client config. Checked on every call (revocation is immediate).
    let agent_key = std::env::var("VIBE_AGENT_KEY").ok();
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
                match mcp::dispatch_as(agent_key.as_deref(), name, &args, &rpc) {
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
