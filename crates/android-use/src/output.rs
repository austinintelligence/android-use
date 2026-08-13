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
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a Value>,
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
            .unwrap_or_else(|_| wire_error("E_JSON", "serialization failed", 0, None))
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
            .unwrap_or_else(|_| wire_error("E_JSON", "serialization failed", 0, None))
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
    println!("{}", format_error(mode, error));
}

fn format_error(mode: OutputMode, error: &AuError) -> String {
    let message = bounded_message(error.compact_message());
    let details = error.details();
    let line = if mode.wire {
        wire_error(error.kind(), &message, message.len(), details)
    } else if mode.compact {
        let mut value = serde_json::Map::new();
        value.insert("o".into(), json!(0));
        value.insert("e".into(), json!(error.kind()));
        value.insert("m".into(), json!(message));
        if let Some(details) = details {
            value.insert("d".into(), details.clone());
        }
        serde_json::to_string(&Value::Object(value))
            .unwrap_or_else(|_| wire_error("E_JSON", "serialization failed", 0, None))
    } else if mode.json {
        let value = JsonError {
            ok: false,
            code: error.kind(),
            message,
            details,
        };
        serde_json::to_string(&value).unwrap_or_else(|_| {
            "{\"ok\":false,\"code\":\"E_JSON\",\"message\":\"serialization failed\"}".into()
        })
    } else {
        format!("err {} {}", error.kind(), message)
    };

    if line.len() <= MAX_OUTPUT_BYTES || !mode.json && !mode.wire {
        line
    } else {
        wire_or_json_overflow(mode, line.len())
    }
}

fn bounded_message(message: String) -> String {
    message.chars().take(512).collect()
}

fn wire_error(code: &str, message: &str, bytes: usize, details: Option<&Value>) -> String {
    let mut value = serde_json::Map::new();
    value.insert("v".into(), json!(1));
    value.insert("o".into(), json!(0));
    value.insert("e".into(), json!(code));
    value.insert("m".into(), json!(message));
    value.insert("b".into(), json!(bytes));
    if let Some(details) = details {
        value.insert("d".into(), details.clone());
    }
    serde_json::to_string(&Value::Object(value))
        .unwrap_or_else(|_| "{\"v\":1,\"o\":0,\"e\":\"E_JSON\"}".into())
}

fn wire_or_json_overflow(mode: OutputMode, bytes: usize) -> String {
    if mode.wire {
        wire_error(
            "E_OUTPUT_LIMIT",
            "structured output exceeds transcript bound; use --out",
            bytes,
            None,
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
    use super::{format_error, format_success, OutputMode, Success};
    use crate::error::AuError;
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

    #[test]
    fn structured_errors_preserve_recovery_details_in_compact_and_wire_modes() {
        let error = AuError::code("E_PARTIAL", "step failed")
            .with_details(json!({"failed_index":2,"next":"observe"}));
        let compact: serde_json::Value = serde_json::from_str(&format_error(
            OutputMode {
                compact: true,
                ..OutputMode::default()
            },
            &error,
        ))
        .expect("compact error");
        assert_eq!(compact["d"]["failed_index"], 2);

        let wire: serde_json::Value = serde_json::from_str(&format_error(
            OutputMode {
                wire: true,
                ..OutputMode::default()
            },
            &error,
        ))
        .expect("wire error");
        assert_eq!(wire["d"]["next"], "observe");
    }

    #[test]
    fn json_errors_preserve_recovery_details_without_forcing_expanded_output() {
        let error = AuError::code("E_UNKNOWN_COMMIT", "observe first")
            .with_details(json!({"operation_id":"op-1","next":"observe"}));
        let value: serde_json::Value = serde_json::from_str(&format_error(
            OutputMode {
                json: true,
                ..OutputMode::default()
            },
            &error,
        ))
        .expect("json error");
        assert_eq!(value["details"]["operation_id"], "op-1");
    }
}
