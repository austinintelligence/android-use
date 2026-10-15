use crate::{
    api::{parse_act_command, parse_read_command, tool_schemas, Code, Error, Result, MAX_FRAME},
    engine::{plain_error, Engine, ModelResponse},
    spec,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Write};

pub fn mcp(mut engine: Engine) -> Result<()> {
    engine.agent.strict = true;
    serve(move |v| rpc(&mut engine, v))
}

fn serve(mut handle: impl FnMut(Value) -> Option<Value>) -> Result<()> {
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut output = stdout.lock();
    loop {
        let Some(line) = bounded_line(&mut input)? else { return Ok(()) };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let reply = match serde_json::from_slice::<Value>(&line) {
            Ok(v) => handle(v),
            Err(e) => Some(json!({"ok":0,"e":"args","m":e.to_string()})),
        };
        if let Some(v) = reply {
            let b = serde_json::to_vec(&v).map_err(|e| Error::new(Code::Protocol, e.to_string()))?;
            output.write_all(&b)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

/// MCP protocol revisions this server speaks, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

fn rpc(engine: &mut Engine, v: Value) -> Option<Value> {
    // Notifications carry no id and never get a reply.
    let id = v.get("id").cloned()?;
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    let result = match v.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => initialize(params.get("protocolVersion").and_then(Value::as_str)),
        "ping" => json!({}),
        "tools/list" => json!({"tools":tool_schemas()}),
        "tools/call" => call_tool(engine, &id, &params),
        _ => return Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}})),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn initialize(requested: Option<&str>) -> Value {
    let version = requested.filter(|version| PROTOCOL_VERSIONS.contains(version)).unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "android-use", "title": "Android Use", "version": env!("CARGO_PKG_VERSION")},
        "instructions": spec::INSTRUCTIONS,
    })
}

fn call_tool(engine: &mut Engine, id: &Value, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    // A resent JSON-RPC id maps to the same operation id, so the journal
    // returns the earlier outcome (or `unknown`) instead of acting twice.
    let identity = serde_json::to_string(id).unwrap_or_default();
    model_result(new_call(engine, name, args, Some(&identity)))
}

fn new_call(engine: &mut Engine, name: &str, args: Value, request_identity: Option<&str>) -> Result<ModelResponse> {
    if args.as_object().is_none_or(|object| object.len() != 1 || !object.contains_key("command")) {
        return Err(Error::new(Code::Args, "tool calls accept only the command string"));
    }
    let command = args.get("command").and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "command must be a string"))?;
    // The dotted names of earlier releases are still accepted, though no longer listed: some
    // model APIs reject dots in tool names.
    match name {
        spec::READ_TOOL | "android.read" => engine.model_read(parse_read_command(command)?),
        spec::ACT_TOOL | "android.act" => {
            let actions = parse_act_command(command)?;
            engine.model_act(&actions, request_identity)
        }
        _ => Err(Error::new(Code::Args, format!("tool must be {} or {}", spec::READ_TOOL, spec::ACT_TOOL))),
    }
}

fn model_result(result: Result<ModelResponse>) -> Value {
    result
        .map(|response| model_json(&response))
        .unwrap_or_else(|error| json!({"content":[{"type":"text","text":plain_error(&error)}],"isError":true,"structuredContent":{"status":error.code.wire()}}))
}

fn model_json(response: &ModelResponse) -> Value {
    let mut content = vec![json!({"type":"text","text":response.text})];
    if let Some(image) = response.image.as_ref().filter(|image| image.mime_type.starts_with("image/")) {
        content.push(json!({"type":"image","data":STANDARD.encode(image.bytes.as_ref()),"mimeType":image.mime_type}));
    }
    let mut result = json!({"content":content,"isError":response.failed});
    // Small on purpose: clients that copy structured content into the model's context pay for it.
    result["structuredContent"] = if response.status.is_empty() { json!({"status":"ok"}) } else { json!({"status":response.status,"changed":response.changed}) };
    result
}
fn bounded_line<R: BufRead>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            return Ok((!out.is_empty()).then_some(out));
        }
        let take = buf.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(buf.len());
        if out.len() + take > MAX_FRAME {
            return Err(Error::new(Code::Bounds, "input line exceeds frame limit"));
        }
        out.extend_from_slice(&buf[..take]);
        r.consume(take);
        if out.last() == Some(&b'\n') {
            out.pop();
            if out.last() == Some(&b'\r') {
                out.pop();
            }
            return Ok(Some(out));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_reader_is_bounded() {
        let mut r = BufReader::new(&b"abc\n"[..]);
        assert_eq!(bounded_line(&mut r).unwrap(), Some(b"abc".to_vec()));
        let bytes = vec![b'x'; MAX_FRAME + 1];
        let mut r = BufReader::new(bytes.as_slice());
        assert_eq!(bounded_line(&mut r).unwrap_err().code, Code::Bounds);
    }

    #[test]
    fn model_response_uses_text_and_native_image_content() {
        let response = ModelResponse {
            text: "Captured the screen. The image is attached.".into(),
            image: Some(crate::engine::ModelImage { bytes: b"png".to_vec().into(), mime_type: "image/png" }),
            ..Default::default()
        };
        let value = model_json(&response);
        assert_eq!(value["structuredContent"], json!({"status":"ok"}));
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][1]["type"], "image");
        assert_eq!(value["content"][1]["mimeType"], "image/png");
        assert_eq!(value["isError"], false);
    }

    fn request(engine: &mut Engine, value: Value) -> Value {
        rpc(engine, value).unwrap()
    }

    #[test]
    fn mcp_lifecycle_works_without_a_device() {
        let d = tempfile::tempdir().unwrap();
        let mut engine = Engine::at(crate::device::Paths::at(d.path().to_path_buf()).unwrap()).unwrap();
        let init = request(&mut engine, json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}));
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(init["result"]["instructions"].as_str().unwrap().contains("screen"));
        let future = request(&mut engine, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}));
        assert_eq!(future["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        assert!(rpc(&mut engine, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        assert_eq!(request(&mut engine, json!({"jsonrpc":"2.0","id":2,"method":"ping"}))["result"], json!({}));
        assert_eq!(request(&mut engine, json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}))["error"]["code"], -32601);
        assert_eq!(request(&mut engine, json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}))["result"]["tools"].as_array().unwrap().len(), 2);
        let bad = request(&mut engine, json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"android_act","arguments":{"command":"tap Save"}}}));
        assert_eq!(bad["result"]["isError"], true);
        assert!(bad["result"]["content"][0]["text"].as_str().unwrap().contains("tap \"LABEL\""));
    }

    #[test]
    fn non_image_media_is_never_sent_as_image_content() {
        let response =
            ModelResponse { text: "Saved.".into(), image: Some(crate::engine::ModelImage { bytes: b"RIFF".to_vec().into(), mime_type: "audio/wav" }), ..Default::default() };
        assert_eq!(model_json(&response)["content"].as_array().unwrap().len(), 1);
    }
}
