use std::fs;

use serde_json::json;

use crate::actions::{ActionResult, Brief};
use crate::config::{atomic_write, AppPaths};
use crate::error::{AuError, Result};

const SUPPORTED: &[&str] = &[
    "codex", "claude", "cursor", "gemini", "opencode", "mcp", "jsonl",
];

pub fn action(args: &[String]) -> Result<ActionResult> {
    let operation = args.first().map(String::as_str).unwrap_or("list");
    let paths = AppPaths::discover()?;
    let config_path = paths.state.join("agent.json");
    match operation {
        "list" => Ok(ActionResult {
            brief: Brief::Ok,
            data: json!({"supported":SUPPORTED,"configured":read_config(&config_path)?}),
        }),
        "configure" => {
            let agent = args.get(1).map(String::as_str).unwrap_or("auto");
            let agent = if agent == "auto" { detect() } else { agent };
            if !SUPPORTED.contains(&agent) {
                return Err(AuError::code(
                    "E_AGENT",
                    format!("unsupported agent {agent}"),
                ));
            }
            let adapter_dir = paths.state.join("agents").join(agent);
            let adapter_path = adapter_dir.join("adapter.json");
            let transport = if matches!(agent, "mcp") {
                "mcp-stdio"
            } else if matches!(agent, "jsonl") {
                "jsonl-stdio"
            } else {
                "stdio"
            };
            let value = json!({
                "schema":1,
                "agent":agent,
                "contract_version":crate::CONTRACT_VERSION,
                "transport":transport,
                "command":"au serve",
                "args":if transport == "mcp-stdio" { vec!["--mcp"] } else { vec!["--jsonl"] },
                "owned_config":adapter_path
            });
            atomic_write(&config_path, &serde_json::to_vec(&value)?)?;
            atomic_write(&adapter_path, &serde_json::to_vec(&value)?)?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: value,
            })
        }
        "remove" => {
            let configured_agent = read_config(&config_path).ok().and_then(|value| {
                value
                    .get("agent")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            });
            if config_path.exists() {
                fs::remove_file(&config_path)?;
            }
            if let Some(agent) = configured_agent {
                let _ = fs::remove_dir_all(paths.state.join("agents").join(agent));
            }
            Ok(ActionResult {
                brief: Brief::Ok,
                data: json!({"configured":false}),
            })
        }
        _ => Err(AuError::code(
            "E_ARGS",
            "agent expects list, configure, or remove",
        )),
    }
}

fn detect() -> &'static str {
    if std::env::var_os("CODEX_HOME").is_some() {
        "codex"
    } else {
        "jsonl"
    }
}

fn read_config(path: &std::path::Path) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::Value::Null);
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|error| AuError::code("E_AGENT", format!("invalid agent config: {error}")))
}
