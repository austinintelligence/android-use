use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::error::{AuError, Result};

const MAX_TRACE_LINE_BYTES: usize = 16 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<Option<TraceState>>> = OnceLock::new();

struct TraceState {
    path: PathBuf,
    id: String,
    started: Instant,
    sequence: u64,
}

/// Configure the optional process-local trace sink. The same path and ID may
/// be configured repeatedly by sequential daemon requests without emitting a
/// second synthetic trace start. A trace is append-only so an existing report
/// is never silently overwritten.
pub fn configure(path: Option<&Path>, requested_id: Option<&str>) -> Result<Option<String>> {
    let Some(path) = path else {
        disable();
        return Ok(None);
    };
    if path.as_os_str().is_empty() {
        return Err(AuError::code("E_TRACE", "trace path must not be empty"));
    }
    let id = requested_id.map(str::to_owned).unwrap_or_else(new_trace_id);
    validate_id(&id)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            AuError::code("E_TRACE", format!("create trace directory: {error}"))
        })?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| AuError::code("E_TRACE", format!("open trace file: {error}")))?;

    let lock = state();
    let mut guard = lock
        .lock()
        .map_err(|_| AuError::code("E_TRACE", "trace state lock poisoned"))?;
    let same = guard
        .as_ref()
        .is_some_and(|current| current.path == path && current.id == id);
    if !same {
        *guard = Some(TraceState {
            path: path.to_owned(),
            id: id.clone(),
            started: Instant::now(),
            sequence: 0,
        });
    }
    ENABLED.store(true, Ordering::Release);
    drop(guard);
    if !same {
        event("trace.start", serde_json::json!({"pid":std::process::id()}));
    }
    Ok(Some(id))
}

pub fn disable() {
    ENABLED.store(false, Ordering::Release);
    if let Some(lock) = STATE.get() {
        if let Ok(mut guard) = lock.lock() {
            *guard = None;
        }
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// Emit one bounded JSONL trace event. Trace I/O is intentionally best-effort
/// after explicit configuration: a full agent command must not fail merely
/// because an optional diagnostic disk is unavailable.
pub fn event(phase: &str, fields: Value) {
    if !enabled() {
        return;
    }
    let Some(lock) = STATE.get() else {
        return;
    };
    let (path, line) = {
        let Ok(mut guard) = lock.lock() else {
            return;
        };
        let Some(trace) = guard.as_mut() else {
            return;
        };
        trace.sequence = trace.sequence.wrapping_add(1);
        let mut record = Map::new();
        record.insert("v".into(), Value::from(1));
        record.insert("id".into(), Value::String(trace.id.clone()));
        record.insert("q".into(), Value::from(trace.sequence));
        record.insert("pid".into(), Value::from(std::process::id()));
        record.insert(
            "us".into(),
            Value::from(trace.started.elapsed().as_micros() as u64),
        );
        record.insert("p".into(), Value::String(phase.to_owned()));
        if let Value::Object(fields) = fields {
            for (key, value) in fields {
                if !matches!(key.as_str(), "v" | "id" | "q" | "pid" | "us" | "p") {
                    record.insert(key, value);
                }
            }
        }
        let mut bytes = serde_json::to_vec(&Value::Object(record))
            .unwrap_or_else(|_| br#"{"v":1,"p":"trace.serialization_error"}"#.to_vec());
        if bytes.len() > MAX_TRACE_LINE_BYTES {
            bytes = serde_json::to_vec(&serde_json::json!({
                "v":1,
                "id":trace.id,
                "q":trace.sequence,
                "pid":std::process::id(),
                "us":trace.started.elapsed().as_micros() as u64,
                "p":"trace.event_truncated",
                "bytes":bytes.len()
            }))
            .unwrap_or_else(|_| br#"{"v":1,"p":"trace.event_truncated"}"#.to_vec());
        }
        bytes.push(b'\n');
        (trace.path.clone(), bytes)
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(&line);
        let _ = file.flush();
    }
}

pub struct Span {
    phase: String,
    fields: Value,
    started: Instant,
    active: bool,
}

pub fn span(phase: impl Into<String>, fields: Value) -> Span {
    let phase = phase.into();
    let active = enabled();
    if active {
        event(
            &phase,
            merge(fields.clone(), serde_json::json!({"e":"begin"})),
        );
    }
    Span {
        phase,
        fields,
        started: Instant::now(),
        active,
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if self.active {
            event(
                &self.phase,
                merge(
                    self.fields.clone(),
                    serde_json::json!({"e":"end","dur_us":self.started.elapsed().as_micros() as u64}),
                ),
            );
        }
    }
}

fn merge(left: Value, right: Value) -> Value {
    let mut merged = match left {
        Value::Object(object) => object,
        _ => Map::new(),
    };
    if let Value::Object(object) = right {
        merged.extend(object);
    }
    Value::Object(merged)
}

fn state() -> &'static Mutex<Option<TraceState>> {
    STATE.get_or_init(|| Mutex::new(None))
}

fn new_trace_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{:x}", nanos, std::process::id())
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AuError::code(
            "E_TRACE",
            "trace id must be 1..96 ASCII letters, digits, '.', '_' or '-'",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    use super::{configure, disable, event, span};

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[test]
    fn trace_events_are_bounded_and_reuse_the_configured_id() {
        let _guard = TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("trace lock");
        let root = tempfile::tempdir().expect("trace temp");
        let path = root.path().join("trace.jsonl");
        let id = configure(Some(&path), Some("test-trace")).expect("configure");
        assert_eq!(id.as_deref(), Some("test-trace"));
        {
            let _span = span("unit", serde_json::json!({"a":1}));
            event("unit.event", serde_json::json!({"ok":true}));
        }
        let second = configure(Some(&path), Some("test-trace")).expect("reconfigure");
        assert_eq!(second.as_deref(), Some("test-trace"));
        disable();
        let text = fs::read_to_string(path).expect("trace output");
        let rows = text.lines().collect::<Vec<_>>();
        assert!(rows.len() >= 4);
        assert!(rows.iter().all(|row| row.len() <= 16 * 1024));
        assert!(rows.iter().all(|row| row.contains("test-trace")));
        assert!(rows.iter().any(|row| row.contains("trace.start")));
        assert!(rows.iter().any(|row| row.contains("unit.event")));
    }

    #[test]
    fn invalid_trace_ids_are_rejected() {
        let _guard = TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("trace lock");
        let root = tempfile::tempdir().expect("trace temp");
        let error = configure(Some(&root.path().join("trace.jsonl")), Some("bad/id"))
            .expect_err("invalid id");
        assert_eq!(error.kind(), "E_TRACE");
        disable();
    }
}
