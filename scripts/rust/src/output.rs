use serde::Serialize;
use serde_json::{json, Value};

use crate::actions::{ActionResult, Brief};
use crate::error::AuError;
use crate::trace;
use crate::MAX_OUTPUT_BYTES;

#[derive(Clone, Copy, Debug, Default)]
pub struct OutputMode {
    pub json: bool,
    pub compact: bool,
    pub wire: bool,
    pub quiet: bool,
    pub binary: bool,
}

#[derive(Debug)]
pub enum Success {
    Ok,
    Count(u32),
    Path(String),
    Text(String),
    Data(Value),
}

/// Render one action result at the final process boundary. Keeping this here
/// prevents the CLI binary and daemon child from drifting into different wire
/// formats as new result types are added.
pub fn emit_action_result(mode: OutputMode, result: ActionResult) {
    trace::event(
        "output.success",
        json!({"json":mode.json,"compact":mode.compact,"wire":mode.wire,"quiet":mode.quiet}),
    );
    if mode.quiet {
        return;
    }
    if mode.json && !mode.compact && !mode.wire {
        let brief = serde_json::to_value(&result.brief).unwrap_or_else(|_| json!("ok"));
        emit_success(
            mode,
            Success::Data(json!({"brief":brief,"data":result.data})),
        );
        return;
    }
    emit_success(mode, action_success(result));
}

fn action_success(result: ActionResult) -> Success {
    match result.brief {
        Brief::Ok => Success::Data(result.data),
        Brief::Count(count) => Success::Count(count),
        Brief::Path(path) => Success::Path(path),
        Brief::Text(text) => Success::Text(text),
    }
}

#[derive(Serialize)]
struct JsonError<'a> {
    ok: bool,
    code: &'a str,
    message: String,
}

pub fn emit_success(mode: OutputMode, success: Success) {
    if mode.quiet {
        return;
    }
    println!("{}", format_success(mode, success));
}

fn format_success(mode: OutputMode, success: Success) -> String {
    let line = if mode.wire {
        let mut value = serde_json::Map::new();
        value.insert("v".into(), json!(1));
        value.insert("o".into(), json!(1));
        match success {
            Success::Ok => {}
            Success::Count(count) => {
                value.insert("n".into(), json!(count));
            }
            Success::Path(path) => {
                value.insert("p".into(), json!(path));
            }
            Success::Text(text) => {
                value.insert("t".into(), json!(text));
            }
            Success::Data(data) => {
                value.insert("d".into(), data);
            }
        }
        serde_json::to_string(&Value::Object(value))
            .unwrap_or_else(|_| wire_error("E_JSON", "serialization failed", 0))
    } else if mode.compact {
        let mut value = serde_json::Map::new();
        value.insert("o".into(), json!(1));
        match success {
            Success::Ok => {}
            Success::Count(count) => {
                value.insert("n".into(), json!(count));
            }
            Success::Path(path) => {
                value.insert("p".into(), json!(path));
            }
            Success::Text(text) => {
                value.insert("t".into(), json!(text));
            }
            Success::Data(data) => {
                value.insert("d".into(), data);
            }
        }
        serde_json::to_string(&Value::Object(value))
            .unwrap_or_else(|_| wire_error("E_JSON", "serialization failed", 0))
    } else if mode.json {
        let value = match success {
            Success::Ok => json!({"ok":true}),
            Success::Count(count) => json!({"ok":true,"count":count}),
            Success::Path(path) => json!({"ok":true,"path":path}),
            Success::Text(text) => json!({"ok":true,"text":text}),
            Success::Data(data) => json!({"ok":true,"data":data}),
        };
        serde_json::to_string(&value).unwrap_or_else(|_| {
            "{\"ok\":false,\"code\":\"E_JSON\",\"message\":\"serialization failed\"}".into()
        })
    } else {
        match success {
            Success::Ok | Success::Data(_) => "ok".into(),
            Success::Count(count) => format!("ok {count}"),
            Success::Path(path) => format!("ok {path}"),
            Success::Text(text) => format!("ok {text}"),
        }
    };
    if line.len() <= MAX_OUTPUT_BYTES || !mode.json && !mode.wire && line == "ok" {
        line
    } else {
        wire_or_json_overflow(mode, line.len())
    }
}

pub fn emit_error(mode: OutputMode, error: &AuError) {
    trace::event(
        "output.error",
        json!({"e":error.kind(),"json":mode.json,"compact":mode.compact,"wire":mode.wire}),
    );
    let message = bounded_message(error.compact_message());
    if mode.wire {
        println!("{}", wire_error(error.kind(), &message, message.len()));
        return;
    }
    if mode.compact {
        println!("{}", json!({"o":0,"e":error.kind(),"m":message}));
        return;
    }
    if mode.json {
        let value = JsonError {
            ok: false,
            code: error.kind(),
            message,
        };
        println!(
            "{}",
            serde_json::to_string(&value).unwrap_or_else(|_| {
                "{\"ok\":false,\"code\":\"E_JSON\",\"message\":\"serialization failed\"}".into()
            })
        );
    } else {
        println!("err {} {}", error.kind(), message);
    }
}

fn bounded_message(message: String) -> String {
    message.chars().take(512).collect()
}

fn wire_error(code: &str, message: &str, bytes: usize) -> String {
    serde_json::to_string(&json!({"v":1,"o":0,"e":code,"m":message,"b":bytes}))
        .unwrap_or_else(|_| "{\"v\":1,\"o\":0,\"e\":\"E_JSON\"}".into())
}

fn wire_or_json_overflow(mode: OutputMode, bytes: usize) -> String {
    if mode.wire {
        wire_error(
            "E_OUTPUT_LIMIT",
            "structured output exceeds transcript bound; use --out",
            bytes,
        )
    } else if mode.compact {
        serde_json::to_string(&json!({
            "o": 0,
            "e": "E_OUTPUT_LIMIT",
            "m": "structured output exceeds transcript bound; use --out",
            "b": bytes
        }))
        .unwrap_or_else(|_| "{\"o\":0,\"e\":\"E_OUTPUT_LIMIT\"}".into())
    } else {
        serde_json::to_string(&json!({
            "ok": false,
            "code": "E_OUTPUT_LIMIT",
            "message": "structured output exceeds transcript bound; use --out",
            "bytes": bytes
        }))
        .unwrap_or_else(|_| "{\"ok\":false,\"code\":\"E_OUTPUT_LIMIT\"}".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{format_success, OutputMode, Success};
    use serde_json::json;

    #[test]
    fn default_data_is_one_line_proof() {
        assert_eq!(
            format_success(OutputMode::default(), Success::Data(json!({"large": true}))),
            "ok"
        );
    }

    #[test]
    fn wire_output_is_versioned_and_minified() {
        let line = format_success(
            OutputMode {
                wire: true,
                ..OutputMode::default()
            },
            Success::Count(20),
        );
        let value: serde_json::Value = serde_json::from_str(&line).expect("wire json");
        assert_eq!(value, serde_json::json!({"v":1,"o":1,"n":20}));
        assert!(!line.contains(' '));
    }

    #[test]
    fn structured_output_returns_a_bounded_error_instead_of_flooding_stdout() {
        let value = (0..super::MAX_OUTPUT_BYTES)
            .map(|_| "x")
            .collect::<String>();
        let line = format_success(
            OutputMode {
                compact: true,
                ..OutputMode::default()
            },
            Success::Data(json!({"value":value})),
        );
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("overflow proof");
        assert_eq!(parsed["e"], "E_OUTPUT_LIMIT");
        assert!(line.len() < 512);
    }
}
