use crate::{
    api::{parse_read, tool_schemas, BrowserPlan, BrowserRead, Code, Error, Plan, Read, Result, VisualPlan, VisualRead, MAX_FRAME},
    engine::Engine,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Write};

pub fn mcp(mut engine: Engine) -> Result<()> {
    serve(move |v| rpc(&mut engine, v))
}
pub fn jsonl(mut engine: Engine) -> Result<()> {
    serve(move |v| direct(&mut engine, v))
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

fn rpc(engine: &mut Engine, v: Value) -> Option<Value> {
    let id = v.get("id").cloned();
    let method = v.get("method").and_then(Value::as_str).unwrap_or("");
    id.as_ref()?;
    let result = match method {
        "initialize" => Ok(json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"android-use","version":env!("CARGO_PKG_VERSION")}})),
        "tools/list" => Ok(json!({"tools":tool_schemas()})),
        "tools/call" => {
            let p = v.get("params").cloned().unwrap_or(Value::Null);
            let name = p.get("name").and_then(Value::as_str).unwrap_or("");
            let args = p.get("arguments").cloned().unwrap_or_else(|| json!({}));
            call(engine, name, args)
                .map(|data| json!({"structuredContent":data,"content":[],"isError":false}))
                .or_else(|e| Ok(json!({"structuredContent":e.json(),"content":[],"isError":true})))
        }
        _ => Err(Error::new(Code::Unsupported, "unknown JSON-RPC method")),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc":"2.0","id":id,"result":r}),
        Err(e) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":e.message,"data":{"e":e.code.wire()}}}),
    })
}

fn direct(engine: &mut Engine, v: Value) -> Option<Value> {
    let name = v.get("tool").and_then(Value::as_str).unwrap_or("").to_string();
    let args = v.get("arguments").cloned().unwrap_or(v);
    Some(call(engine, &name, args).unwrap_or_else(|e| e.json()))
}
fn call(engine: &mut Engine, name: &str, args: Value) -> Result<Value> {
    match name {
        "android.read" => engine.read(parse_read(args)?),
        "android.act" if args.get("target").and_then(Value::as_str) == Some("browser") => engine.browser_act(BrowserPlan::parse(args)?),
        "android.act" if args.get("target").and_then(Value::as_str) == Some("visual") => engine.visual_act(VisualPlan::parse(args)?),
        "android.act" => engine.act(Plan::parse(args)?),
        _ => Err(Error::new(Code::Args, "tool must be android.read or android.act")),
    }
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

pub fn one_read(engine: &mut Engine, q: &str, base: Option<&str>, detail: u8, id: Option<&str>, range: Option<crate::api::Range>) -> Result<Value> {
    let read = match q {
        "status" => Read::Status,
        "observe" => Read::Observe { base: base.map(Into::into), detail },
        "browser" => Read::Browser {
            op: match base.unwrap_or("observe") {
                "tabs" => BrowserRead::Tabs,
                "observe" => BrowserRead::Observe,
                "text" => BrowserRead::Text,
                _ => return Err(Error::new(Code::Args, "browser read must be tabs, observe, or text")),
            },
        },
        "capabilities" => Read::Capabilities,
        "location" => Read::Location,
        "notifications" => Read::Notifications,
        "visual" => Read::Visual(VisualRead::Hash(id.ok_or_else(|| Error::new(Code::Args, "visual hash requires an artifact id"))?.into())),
        "artifact" => Read::Artifact { id: id.ok_or_else(|| Error::new(Code::Args, "artifact id is required"))?.into(), range },
        _ => return Err(Error::new(Code::Args, "unknown read command")),
    };
    engine.read(read)
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
}
