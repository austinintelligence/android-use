use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use crate::cli::Cli;
use crate::contract::{self, Request};
use crate::error::{AuError, Result};
use crate::runtime;

pub fn run(cli: &Cli) -> Result<()> {
    if cli.mcp {
        serve_mcp()
    } else {
        serve_jsonl()
    }
}

fn serve_jsonl() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut runtime = runtime::ContractRuntime::new()?;
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| AuError::code("E_IO", format!("read JSONL: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > contract::MAX_CONTRACT_MESSAGE_BYTES {
            let response = contract::error(
                "?",
                &AuError::code(
                    "E_LIMIT",
                    format!(
                        "contract message exceeds {} bytes",
                        contract::MAX_CONTRACT_MESSAGE_BYTES
                    ),
                ),
            );
            serde_json::to_writer(&mut stdout, &response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
            continue;
        }
        let response = match contract::parse_request(&line) {
            Ok(request) => runtime.contract_response(&request),
            Err(error) => contract::error("?", &error),
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

fn serve_mcp() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut runtime = runtime::ContractRuntime::new()?;
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| AuError::code("E_IO", format!("read MCP: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > contract::MAX_CONTRACT_MESSAGE_BYTES {
            return Err(AuError::code(
                "E_LIMIT",
                format!(
                    "MCP message exceeds {} bytes",
                    contract::MAX_CONTRACT_MESSAGE_BYTES
                ),
            ));
        }
        let message: Value = serde_json::from_str(&line).map_err(|error| {
            AuError::code("E_PROTOCOL", format!("invalid MCP message: {error}"))
        })?;
        let response = mcp_message(&message, &mut runtime)?;
        if let Some(response) = response {
            serde_json::to_writer(&mut stdout, &response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn mcp_message(message: &Value, runtime: &mut runtime::ContractRuntime) -> Result<Option<Value>> {
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    match method {
        "notifications/initialized" | "notifications/cancelled" => Ok(None),
        "initialize" => Ok(Some(json!({
            "jsonrpc":"2.0","id":id,
            "result":{
                "protocolVersion":"2025-11-25",
                "capabilities":{"tools":{}},
                "serverInfo":{"name":"android-use","version":crate::VERSION}
            }
        }))),
        "tools/list" => Ok(Some(
            json!({"jsonrpc":"2.0","id":id,"result":contract::mcp_tools()}),
        )),
        "tools/call" => {
            let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| AuError::code("E_ARGS", "tools/call requires name"))?;
            let request = Request {
                v: crate::CONTRACT_VERSION,
                id: mcp_request_id(&id)?,
                method: name.into(),
                params: params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            };
            let response = match contract::validate_request(&request) {
                Ok(()) => runtime.contract_response(&request),
                Err(error) => contract::error(request.id.clone(), &error),
            };
            let structured = if response.ok {
                response.result.unwrap_or_else(|| json!({}))
            } else {
                json!({"error":response.error})
            };
            Ok(Some(json!({
                "jsonrpc":"2.0","id":id,
                "result":{
                    "content":[{"type":"text","text":serde_json::to_string(&structured)?}],
                    "structuredContent":structured,
                    "isError":!response.ok
                }
            })))
        }
        _ => Ok(Some(json!({
            "jsonrpc":"2.0","id":id,
            "error":{"code":-32601,"message":format!("method not found: {method}")}
        }))),
    }
}

fn mcp_request_id(value: &Value) -> Result<String> {
    match value {
        Value::String(value) if !value.is_empty() && value.len() <= 128 => Ok(value.clone()),
        Value::Number(value) => {
            let text = value.to_string();
            if text.len() <= 128 {
                Ok(text)
            } else {
                Err(AuError::code("E_LIMIT", "MCP request id is too long"))
            }
        }
        _ => Err(AuError::code(
            "E_PROTOCOL",
            "tools/call requires a string or number JSON-RPC id",
        )),
    }
}
