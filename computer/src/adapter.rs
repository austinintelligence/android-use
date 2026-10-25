use crate::{
    api::{tool_schemas, Code, Error, Result, MAX_FRAME},
    engine::{plain_error, Engine, ModelResponse},
    spec,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

pub fn mcp(mut engine: Engine) -> Result<()> {
    engine.agent.strict = true;
    // A pause or stop belongs to the session it was pressed in; a new session starts running.
    let _ = std::fs::remove_file(engine.home().join("control.json"));
    serve(move |v| rpc(&mut engine, v))
}

fn serve(mut handle: impl FnMut(Value) -> Option<Value>) -> Result<()> {
    let stdin = io::stdin();
    serve_lines(stdin.lock(), io::stdout().lock(), &mut handle)
}

/// Answers each frame and keeps reading. A bad frame gets a JSON-RPC error with a null id; the
/// loop ends only at end of input or when the output cannot be written.
fn serve_lines(mut input: impl BufRead, mut output: impl Write, mut handle: impl FnMut(Value) -> Option<Value>) -> Result<()> {
    loop {
        let reply = match bounded_line(&mut input)? {
            Frame::End => return Ok(()),
            Frame::Oversized => Some(rpc_error(Value::Null, -32600, "request exceeds the 1 MiB frame limit")),
            Frame::Line(line) if line.iter().all(u8::is_ascii_whitespace) => None,
            Frame::Line(line) => match serde_json::from_slice::<Value>(&line) {
                // JSON-RPC batches are not supported, so any frame that is not one object is invalid.
                Ok(v) if !v.is_object() => Some(rpc_error(Value::Null, -32600, "request must be one JSON object; batches are not supported")),
                Ok(v) => handle(v),
                Err(_) => Some(rpc_error(Value::Null, -32700, "parse error")),
            },
        };
        if let Some(v) = reply {
            let b = serde_json::to_vec(&v).map_err(|e| Error::new(Code::Protocol, e.to_string()))?;
            output.write_all(&b)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
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
        "" => return Some(rpc_error(id, -32600, "request has no method")),
        _ => return Some(rpc_error(id, -32601, "method not found")),
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

pub(crate) fn new_call(engine: &mut Engine, name: &str, args: Value, request_identity: Option<&str>) -> Result<ModelResponse> {
    if args.as_object().is_none_or(|object| object.len() != 1 || !object.contains_key("command")) {
        return Err(Error::new(Code::Args, "tool calls accept only the command string"));
    }
    let command = args.get("command").and_then(Value::as_str).ok_or_else(|| Error::new(Code::Args, "command must be a string"))?;
    // The dotted names of earlier releases are still accepted, though no longer listed: some
    // model APIs reject dots in tool names.
    match name {
        spec::READ_TOOL | "android.read" => crate::session::call(engine, true, command, request_identity),
        spec::ACT_TOOL | "android.act" => crate::session::call(engine, false, command, request_identity),
        _ => Err(Error::new(Code::Args, format!("tool must be {} or {}", spec::READ_TOOL, spec::ACT_TOOL))),
    }
}

fn model_result(result: Result<ModelResponse>) -> Value {
    result.map(|response| model_json(&response)).unwrap_or_else(|error| json!({"content":[{"type":"text","text":plain_error(&error)}],"isError":true}))
}

fn model_json(response: &ModelResponse) -> Value {
    let mut content = vec![json!({"type":"text","text":response.text})];
    if let Some(image) = response.image.as_ref().filter(|image| image.mime_type.starts_with("image/")) {
        content.push(json!({"type":"image","data":STANDARD.encode(image.bytes.as_ref()),"mimeType":image.mime_type}));
    }
    // No `structuredContent`. This server declares no `outputSchema`, so a client may prefer
    // structured content over `content`, and Claude Code does: it showed the model
    // `{"status":"ok"}` in place of the screen. The status is in `text` for anyone who wants it.
    json!({"content":content,"isError":response.failed})
}
enum Frame {
    Line(Vec<u8>),
    /// Over the frame limit; the rest of the line is consumed and dropped.
    Oversized,
    End,
}

fn bounded_line<R: BufRead>(r: &mut R) -> Result<Frame> {
    let mut out = Vec::new();
    let mut oversized = false;
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            return Ok(match (oversized, out.is_empty()) {
                (true, _) => Frame::Oversized,
                (false, true) => Frame::End,
                (false, false) => Frame::Line(out),
            });
        }
        let take = buf.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(buf.len());
        let ended = buf[take - 1] == b'\n';
        if oversized || out.len() + take > MAX_FRAME {
            oversized = true;
        } else {
            out.extend_from_slice(&buf[..take]);
        }
        r.consume(take);
        if ended {
            if oversized {
                return Ok(Frame::Oversized);
            }
            out.pop();
            if out.last() == Some(&b'\r') {
                out.pop();
            }
            return Ok(Frame::Line(out));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn line_reader_is_bounded() {
        let mut r = BufReader::new(&b"abc\r\n"[..]);
        assert!(matches!(bounded_line(&mut r).unwrap(), Frame::Line(line) if line == b"abc"));
        assert!(matches!(bounded_line(&mut r).unwrap(), Frame::End));
        let mut bytes = vec![b'x'; MAX_FRAME + 1];
        bytes.extend_from_slice(b"\nnext\n");
        let mut r = BufReader::new(bytes.as_slice());
        assert!(matches!(bounded_line(&mut r).unwrap(), Frame::Oversized));
        assert!(matches!(bounded_line(&mut r).unwrap(), Frame::Line(line) if line == b"next"), "the rest of the oversized line is discarded");
    }

    #[test]
    fn bad_frames_get_errors_and_the_loop_keeps_serving() {
        let d = tempfile::tempdir().unwrap();
        let mut engine = Engine::at(crate::device::Paths::at(d.path().to_path_buf()).unwrap()).unwrap();
        let mut input = vec![b'x'; MAX_FRAME + 1];
        input.extend_from_slice(b"\n{not json\n[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]\n\n");
        input.extend_from_slice(br#"{"jsonrpc":"2.0","id":9,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#);
        input.push(b'\n');
        let mut output = Vec::new();
        serve_lines(input.as_slice(), &mut output, |v| rpc(&mut engine, v)).unwrap();
        let replies: Vec<Value> = output.split(|&b| b == b'\n').filter(|line| !line.is_empty()).map(|line| serde_json::from_slice(line).unwrap()).collect();
        assert_eq!(replies.len(), 4, "three errors, then the initialize reply");
        assert_eq!(replies[0]["error"]["code"], -32600);
        assert_eq!(replies[1]["error"]["code"], -32700);
        assert_eq!(replies[2]["error"]["code"], -32600);
        assert!(replies[..3].iter().all(|reply| reply["id"].is_null()));
        assert_eq!(replies[3]["id"], 9);
        assert_eq!(replies[3]["result"]["protocolVersion"], "2025-03-26");
    }

    #[test]
    fn model_response_uses_text_and_native_image_content() {
        let response = ModelResponse {
            text: "Captured the screen. The image is attached.".into(),
            image: Some(crate::engine::ModelImage { bytes: b"png".to_vec().into(), mime_type: "image/png" }),
            ..Default::default()
        };
        let value = model_json(&response);
        assert!(value.get("structuredContent").is_none(), "a client that prefers structured content would show this instead of the screen");
        assert_eq!(value["content"][0]["type"], "text");
        assert_eq!(value["content"][1]["type"], "image");
        assert_eq!(value["content"][1]["mimeType"], "image/png");
        assert_eq!(value["isError"], false);
    }

    #[test]
    fn the_screen_text_and_the_failure_explanation_reach_the_client_in_content() {
        let read = model_result(Ok(ModelResponse { text: "Settings\nWi-Fi".into(), ..Default::default() }));
        assert_eq!((read["content"][0]["text"].as_str(), read["isError"].as_bool(), read.get("structuredContent").is_none()), (Some("Settings\nWi-Fi"), Some(false), true));
        let failed = model_result(Err(Error::plain(Code::Blocked, "Blocked by policy.toml: nothing was run.")));
        assert_eq!(
            (failed["content"][0]["text"].as_str(), failed["isError"].as_bool(), failed.get("structuredContent").is_none()),
            (Some("Blocked by policy.toml: nothing was run."), Some(true), true)
        );
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
