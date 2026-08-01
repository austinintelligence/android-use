use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adb::Adb;
use crate::config::{atomic_write, AppPaths};
use crate::error::{AuError, Result};
use crate::process::text;
use crate::protocol::{read_frame, write_frame};
use crate::{trace, MAX_PROTOCOL_FRAME, PROTOCOL_VERSION};

pub const HELPER_PACKAGE: &str = "dev.codex.aubridge";
const HELPER_SOCKET: &str = "codex_au_bridge";

#[derive(Clone, Deserialize, Serialize)]
pub struct HelperRequest {
    pub version: u16,
    pub id: u64,
    pub sequence: u64,
    pub token: String,
    pub nonce: String,
    pub operation: String,
    pub args: Value,
}

impl fmt::Debug for HelperRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HelperRequest")
            .field("version", &self.version)
            .field("id", &self.id)
            .field("sequence", &self.sequence)
            .field("token", &"<redacted>")
            .field("nonce", &self.nonce)
            .field("operation", &self.operation)
            .field("args", &self.args)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HelperResponse {
    pub version: u16,
    pub id: u64,
    pub ok: bool,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ForwardRecord {
    serial: String,
    local: String,
    remote: String,
    created_by: String,
}

#[derive(Debug)]
pub struct HelperSession {
    adb: Adb,
    serial: String,
    local: String,
    token: String,
    stream: TcpStream,
    paths: AppPaths,
    closed: bool,
    sequence: u64,
}

impl HelperSession {
    pub fn open(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<Self> {
        let _span = trace::span("helper.open", json!({"serial":serial}));
        // The helper service is intentionally restartable. Starting the
        // foreground service is idempotent when it is already alive and
        // closes the recovery gap after an app/service restart: a dead
        // daemon-owned socket must not require a manual tap in MainActivity.
        start_bridge_service(adb, serial)?;
        let token = read_token(adb, serial)?;
        let local = create_forward(adb, paths, serial)?;
        let port = local
            .strip_prefix("tcp:")
            .ok_or_else(|| AuError::code("E_HELPER", "adb forward did not return a tcp endpoint"))?
            .parse::<u16>()?;
        let address =
            format!("127.0.0.1:{port}")
                .parse()
                .map_err(|error: std::net::AddrParseError| {
                    AuError::code("E_HELPER", error.to_string())
                })?;
        let stream = match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            Ok(stream) => stream,
            Err(error) => {
                // `open` owns the forward from the moment `create_forward`
                // returns. If the helper service is stopped or exposes an
                // incompatible socket, do not leak that forward while
                // reporting the connection failure.
                let _ = remove_forward(adb, paths, serial, &local);
                return Err(AuError::code(
                    "E_HELPER",
                    format!("connect through ADB forward: {error}"),
                ));
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        Ok(Self {
            adb: adb.clone(),
            serial: serial.into(),
            local,
            token,
            stream,
            paths: paths.clone(),
            closed: false,
            sequence: 0,
        })
    }

    pub fn call(&mut self, operation: &str, args: Value) -> Result<Value> {
        self.call_with_timeout(operation, args, Duration::from_secs(5))
    }

    pub fn call_with_timeout(
        &mut self,
        operation: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let io_timeout = timeout
            .min(Duration::from_secs(600))
            .saturating_add(Duration::from_secs(1));
        self.stream.set_read_timeout(Some(io_timeout))?;
        self.stream.set_write_timeout(Some(io_timeout))?;
        let result = self.call_inner(operation, args);
        let _ = self.stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = self.stream.set_write_timeout(Some(Duration::from_secs(5)));
        result
    }

    fn call_inner(&mut self, operation: &str, args: Value) -> Result<Value> {
        let _span = trace::span("helper.call", json!({"op":operation}));
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| AuError::code("E_PROTOCOL", "helper request sequence exhausted"))?;
        let request = HelperRequest {
            version: PROTOCOL_VERSION,
            id: request_id(),
            sequence: self.sequence,
            token: self.token.clone(),
            nonce: nonce(),
            operation: operation.into(),
            args,
        };
        write_frame(&mut self.stream, &request)
            .map_err(|error| helper_transport_error("write request", error))?;
        let response: HelperResponse = read_frame(&mut self.stream)
            .map_err(|error| helper_transport_error("read response", error))?;
        if response.version != PROTOCOL_VERSION || response.id != request.id {
            return Err(AuError::code(
                "E_HELPER",
                "helper response protocol mismatch",
            ));
        }
        if !response.ok {
            let code = helper_error_code(&response.code);
            return Err(AuError::code(code, response.message));
        }
        Ok(response.data)
    }

    pub fn heartbeat(&mut self) -> Result<()> {
        self.call("heartbeat", json!({}))?;
        Ok(())
    }

    /// Keep finite camera/microphone work alive without widening the helper's
    /// authority: heartbeat traffic uses the already-created, tracked ADB forward
    /// on independent local-socket connections. If this host exits, the heartbeat
    /// stops and the helper watchdog shuts media down.
    pub fn call_media(
        &mut self,
        operation: &str,
        args: Value,
        maximum_seconds: u64,
    ) -> Result<Value> {
        let port = self
            .local
            .strip_prefix("tcp:")
            .ok_or_else(|| AuError::code("E_HELPER", "invalid helper local forward"))?
            .parse::<u16>()?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let token = self.token.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(750));
                if worker_stop.load(Ordering::Relaxed) {
                    break;
                }
                let _ = send_heartbeat(port, &token);
            }
        });
        let result = self.call_with_timeout(
            operation,
            args,
            Duration::from_secs(maximum_seconds.saturating_add(10)),
        );
        stop.store(true, Ordering::Relaxed);
        let _ = worker.join();
        result
    }

    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let _ = self.stream.shutdown(Shutdown::Both);
        remove_forward(&self.adb, &self.paths, &self.serial, &self.local)?;
        self.closed = true;
        Ok(())
    }
}

fn helper_error_code(code: &str) -> &'static str {
    match code {
        "E_ARGS" => "E_ARGS",
        "E_AMBIGUOUS" => "E_AMBIGUOUS",
        "E_ASSERT" => "E_ASSERT",
        "E_AUTH" => "E_AUTH",
        "E_CAPABILITY" => "E_CAPABILITY",
        "E_FRAME" => "E_FRAME",
        "E_LOCATION" => "E_LOCATION",
        "E_MEDIA" => "E_MEDIA",
        "E_PROTOCOL" => "E_PROTOCOL",
        "E_STALE" => "E_STALE",
        "E_TIMEOUT" => "E_TIMEOUT",
        "E_UI" => "E_UI",
        "E_UNSUPPORTED" => "E_UNSUPPORTED",
        _ => "E_HELPER",
    }
}

fn is_transport_error(error: &AuError) -> bool {
    // A socket read timeout is normalized by helper_transport_error to
    // E_HELPER.  E_TIMEOUT that reaches this layer is an explicit helper
    // result (for example, a semantic ui.wait whose selector did not appear),
    // not evidence that replaying the operation is safe or useful.
    matches!(error.kind(), "E_HELPER" | "E_PROTOCOL")
}

fn is_accessibility_binding_error(error: &AuError) -> bool {
    error.kind() == "E_CAPABILITY"
        && error
            .to_string()
            .contains("Accessibility service is not enabled")
}

fn is_retryable_operation(operation: &str) -> bool {
    matches!(
        operation,
        "heartbeat"
            | "ui.snapshot"
            | "ui.snap"
            | "ui.find"
            | "ui.wait"
            | "ui.assert"
            | "ui.watch"
            | "notification.ls"
            | "notification.watch"
            | "camera.list"
            | "location.status"
            | "location.get"
    )
}

impl Drop for HelperSession {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// One authenticated JSON session per active endpoint in a daemon epoch.
///
/// The pool owns both the ADB/config handles and the helper forward, so the
/// daemon can keep semantic actions on one framed connection without leaking
/// references to per-request stack state. Any transport/protocol failure
/// evicts the session; the next request starts a fresh authenticated epoch.
#[derive(Debug, Default)]
pub struct HelperPool {
    sessions: HashMap<String, HelperSession>,
}

impl HelperPool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn call(
        &mut self,
        adb: &Adb,
        paths: &AppPaths,
        serial: &str,
        operation: &str,
        args: Value,
    ) -> Result<Value> {
        self.call_with_timeout(adb, paths, serial, operation, args, Duration::from_secs(5))
    }

    pub fn call_with_timeout(
        &mut self,
        adb: &Adb,
        paths: &AppPaths,
        serial: &str,
        operation: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<Value> {
        if !self.sessions.contains_key(serial) {
            let session = HelperSession::open(adb, paths, serial)?;
            self.sessions.insert(serial.into(), session);
        }
        let retry_args = args.clone();
        let result = self
            .sessions
            .get_mut(serial)
            .ok_or_else(|| AuError::code("E_HELPER", "helper session was not retained"))?
            .call_with_timeout(operation, args, timeout);
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                self.close(serial);
                // A service restart can leave the Android abstract socket
                // absent for a bounded interval. Retry only read-only
                // operations; repeating a tap, text mutation, media capture,
                // or location change after a lost response could duplicate an
                // external side effect.
                let binding_recovery =
                    is_retryable_operation(operation) && is_accessibility_binding_error(&error);
                if binding_recovery {
                    // Android can retain the secure enabled-service string
                    // while leaving an updated user package in the default
                    // disabled state. Re-enable only the AU package, then
                    // let the normal bounded read-only retry wait for the
                    // accessibility binding to come back. This is a repair
                    // for the package-update edge case, not a safety bypass
                    // and never replays a state-changing operation.
                    let _ = ensure_helper_package_enabled(adb, serial);
                }
                if is_retryable_operation(operation)
                    && (is_transport_error(&error) || binding_recovery)
                {
                    let mut last_error = error;
                    for _ in 0..8 {
                        thread::sleep(Duration::from_millis(200));
                        match HelperSession::open(adb, paths, serial) {
                            Ok(mut session) => {
                                let retry = session.call_with_timeout(
                                    operation,
                                    retry_args.clone(),
                                    timeout,
                                );
                                if let Ok(value) = retry {
                                    self.sessions.insert(serial.into(), session);
                                    return Ok(value);
                                }
                                last_error =
                                    retry.expect_err("helper retry was checked as an error");
                                let _ = session.close();
                            }
                            Err(open_error) => last_error = open_error,
                        }
                    }
                    return Err(last_error);
                }
                Err(error)
            }
        }
    }

    pub fn close(&mut self, serial: &str) {
        if let Some(mut session) = self.sessions.remove(serial) {
            let _ = session.close();
        }
    }

    pub fn shutdown(&mut self) {
        let sessions = std::mem::take(&mut self.sessions);
        for (_, mut session) in sessions {
            let _ = session.close();
        }
    }
}

impl Drop for HelperPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn call(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    operation: &str,
    args: Value,
) -> Result<Value> {
    let mut session = HelperSession::open(adb, paths, serial)?;
    let result = session.call(operation, args);
    let close = session.close();
    match result {
        Ok(value) => {
            close?;
            Ok(value)
        }
        Err(error) => {
            let _ = close;
            Err(error)
        }
    }
}

pub fn call_media(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    operation: &str,
    args: Value,
    maximum_seconds: u64,
) -> Result<Value> {
    let mut session = HelperSession::open(adb, paths, serial)?;
    let result = session.call_media(operation, args, maximum_seconds);
    let close = session.close();
    match result {
        Ok(value) => {
            close?;
            Ok(value)
        }
        Err(error) => {
            let _ = close;
            Err(error)
        }
    }
}

pub fn capability(adb: &Adb, serial: &str) -> Result<Value> {
    let installed = adb.device(
        serial,
        &[
            "shell".into(),
            "pm".into(),
            "path".into(),
            HELPER_PACKAGE.into(),
        ],
    );
    match installed {
        Ok(result) if text(&result.stdout).starts_with("package:") => {
            let paths = AppPaths::discover()?;
            let mut value = json!({
                "installed": true,
                "package": HELPER_PACKAGE,
                "available": false,
                "protocol": PROTOCOL_VERSION
            });
            match HelperSession::open(adb, &paths, serial) {
                Ok(mut session) => {
                    let probe = session.call("heartbeat", json!({}));
                    let _ = session.close();
                    match probe {
                        Ok(_) => value["available"] = json!(true),
                        Err(error) => value["error"] = error_value(&error),
                    }
                }
                Err(error) => value["error"] = error_value(&error),
            }
            Ok(value)
        }
        Ok(_) => Ok(json!({"installed":false,"package":HELPER_PACKAGE})),
        Err(error) => Err(error),
    }
}

fn read_token(adb: &Adb, serial: &str) -> Result<String> {
    // The Android service creates the private token in onCreate(). Starting
    // the service and reading the file are separate ADB transactions, so a
    // clean install can briefly expose a valid service with no file yet.
    // Retry only this bounded bootstrap race; never wait indefinitely.
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_error: String;
    loop {
        match adb.device(
            serial,
            &[
                "shell".into(),
                "run-as".into(),
                HELPER_PACKAGE.into(),
                "cat".into(),
                "files/bridge_token".into(),
            ],
        ) {
            Ok(result) => {
                let token = text(&result.stdout);
                if token.len() >= 16
                    && token.len() <= 512
                    && !token.chars().any(char::is_whitespace)
                {
                    return Ok(token);
                }
                last_error = "token file was empty or malformed".to_string();
            }
            Err(error) => last_error = error.compact_message(),
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(75));
    }
    Err(AuError::code(
        "E_HELPER",
        format!(
            "helper token unavailable; install a debuggable signed helper first ({})",
            last_error
        ),
    ))
}

fn start_bridge_service(adb: &Adb, serial: &str) -> Result<()> {
    let result = adb.device(
        serial,
        &[
            "shell".into(),
            "am".into(),
            "start-foreground-service".into(),
            "-n".into(),
            format!("{HELPER_PACKAGE}/.AuBridgeService"),
        ],
    )?;
    let proof = text(&result.stdout);
    if proof.contains("Error") || proof.contains("Exception") {
        return Err(AuError::code(
            "E_HELPER",
            format!("Android rejected helper service start: {}", proof.trim()),
        ));
    }
    Ok(())
}

fn ensure_helper_package_enabled(adb: &Adb, serial: &str) -> Result<()> {
    adb.device(
        serial,
        &[
            "shell".into(),
            "pm".into(),
            "enable".into(),
            "--user".into(),
            "0".into(),
            HELPER_PACKAGE.into(),
        ],
    )?;
    Ok(())
}

fn create_forward(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<String> {
    let result = adb.device(
        serial,
        &[
            "forward".into(),
            "tcp:0".into(),
            format!("localabstract:{HELPER_SOCKET}"),
        ],
    )?;
    let port = text(&result.stdout);
    if port.parse::<u16>().is_err() {
        return Err(AuError::code(
            "E_HELPER",
            "adb did not allocate a local TCP forward",
        ));
    }
    let local = format!("tcp:{port}");
    let mut records = load_forwards(&paths.forwards)?;
    // Registry files can outlive a crashed host process. Reconcile only
    // AU-owned helper records for this exact endpoint against ADB's live
    // list; never infer ownership for another serial or another remote.
    if let Ok(active) = active_forwards(adb, serial) {
        records.retain(|record| {
            record.serial != serial
                || record.created_by != "helper"
                || active.contains(&record.local)
        });
    }
    records.push(ForwardRecord {
        serial: serial.into(),
        local: local.clone(),
        remote: format!("localabstract:{HELPER_SOCKET}"),
        created_by: "helper".into(),
    });
    if let Err(error) = atomic_write(&paths.forwards, &serde_json::to_vec(&records)?) {
        let _ = adb.device(
            serial,
            &["forward".into(), "--remove".into(), local.clone()],
        );
        return Err(error);
    }
    Ok(local)
}

fn active_forwards(adb: &Adb, serial: &str) -> Result<HashSet<String>> {
    let result = adb.device(serial, &["forward".into(), "--list".into()])?;
    let mut active = HashSet::new();
    for line in text(&result.stdout).lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() >= 3 && fields[0] == serial {
            active.insert(fields[1].to_owned());
        }
    }
    Ok(active)
}

pub fn remove_forward(adb: &Adb, paths: &AppPaths, serial: &str, local: &str) -> Result<()> {
    let records = load_forwards(&paths.forwards)?;
    let owned = records
        .iter()
        .any(|record| record.serial == serial && record.local == local);
    if !owned {
        return Err(AuError::code(
            "E_FORWARD",
            format!("refusing to remove untracked forward {local}"),
        ));
    }
    adb.device(serial, &["forward".into(), "--remove".into(), local.into()])?;
    let remaining = records
        .into_iter()
        .filter(|record| !(record.serial == serial && record.local == local))
        .collect::<Vec<_>>();
    atomic_write(&paths.forwards, &serde_json::to_vec(&remaining)?)?;
    Ok(())
}

pub fn cleanup_owned_forwards(adb: &Adb, paths: &AppPaths) -> Result<u32> {
    let records = load_forwards(&paths.forwards)?;
    let mut kept = Vec::new();
    let mut removed = 0u32;
    for record in records {
        if adb
            .device(
                &record.serial,
                &["forward".into(), "--remove".into(), record.local.clone()],
            )
            .is_ok()
        {
            removed += 1;
        } else {
            kept.push(record);
        }
    }
    atomic_write(&paths.forwards, &serde_json::to_vec(&kept)?)?;
    Ok(removed)
}

fn load_forwards(path: &Path) -> Result<Vec<ForwardRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|error| AuError::code("E_FORWARD", format!("corrupt forward registry: {error}")))
}

fn helper_transport_error(stage: &str, error: AuError) -> AuError {
    let kind = error.kind();
    let detail = error.compact_message();
    if stage == "read response"
        && (kind == "E_FRAME" || detail.contains("failed to fill whole buffer"))
    {
        return AuError::code(
            "E_PROTOCOL",
            format!("helper returned no valid framed response; update {HELPER_PACKAGE}: {detail}"),
        );
    }
    AuError::code("E_HELPER", format!("helper {stage}: {detail}"))
}

fn error_value(error: &AuError) -> Value {
    json!({
        "code": error.kind(),
        "message": error.compact_message().chars().take(240).collect::<String>()
    })
}

fn request_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn send_heartbeat(port: u16, token: &str) -> Result<()> {
    let address = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|error: std::net::AddrParseError| AuError::code("E_HELPER", error.to_string()))?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let request = HelperRequest {
        version: PROTOCOL_VERSION,
        id: request_id(),
        sequence: 1,
        token: token.into(),
        nonce: nonce(),
        operation: "heartbeat".into(),
        args: json!({}),
    };
    write_frame(&mut stream, &request)?;
    let response: HelperResponse = read_frame(&mut stream)?;
    if !response.ok || response.id != request.id || response.version != PROTOCOL_VERSION {
        return Err(AuError::code("E_HEARTBEAT", "helper rejected heartbeat"));
    }
    Ok(())
}

fn nonce() -> String {
    format!("{:x}-{:x}", request_id(), std::process::id())
}

pub fn helper_frame_limit() -> usize {
    MAX_PROTOCOL_FRAME
}

#[cfg(test)]
mod tests {
    use super::{
        helper_error_code, helper_frame_limit, helper_transport_error,
        is_accessibility_binding_error, HelperRequest, HELPER_PACKAGE,
    };
    use crate::error::AuError;

    #[test]
    fn helper_requests_keep_tokens_out_of_debug_contract() {
        let request = HelperRequest {
            version: 1,
            id: 1,
            sequence: 1,
            token: "secret".into(),
            nonce: "n".into(),
            operation: "ui.snapshot".into(),
            args: serde_json::json!({}),
        };
        assert_eq!(request.operation, "ui.snapshot");
        assert_eq!(request.sequence, 1);
        assert!(helper_frame_limit() >= 1024);
    }

    #[test]
    fn helper_request_debug_redacts_token() {
        let request = HelperRequest {
            version: 1,
            id: 1,
            sequence: 1,
            token: "secret-token".into(),
            nonce: "nonce-00000001".into(),
            operation: "heartbeat".into(),
            args: serde_json::json!({}),
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn stale_handles_keep_their_public_error_code() {
        assert_eq!(helper_error_code("E_STALE"), "E_STALE");
        assert_eq!(helper_error_code("unknown"), "E_HELPER");
    }

    #[test]
    fn incomplete_helper_response_is_reported_as_upgradeable_protocol_error() {
        let error = helper_transport_error(
            "read response",
            AuError::code("E_FRAME", "invalid frame size"),
        );
        assert_eq!(error.kind(), "E_PROTOCOL");
        assert!(error.compact_message().contains(HELPER_PACKAGE));
    }

    #[test]
    fn ordinary_helper_transport_failures_keep_a_helper_error() {
        let error = helper_transport_error(
            "read response",
            AuError::code("E_TIMEOUT", "helper response timed out"),
        );
        assert_eq!(error.kind(), "E_HELPER");
    }

    #[test]
    fn accessibility_binding_recovery_is_narrowly_classified() {
        assert!(is_accessibility_binding_error(&AuError::code(
            "E_CAPABILITY",
            "Accessibility service is not enabled"
        )));
        assert!(!is_accessibility_binding_error(&AuError::code(
            "E_CAPABILITY",
            "Camera permission is not granted"
        )));
        assert!(!is_accessibility_binding_error(&AuError::code(
            "E_UI",
            "Accessibility service is not enabled"
        )));
    }

    #[test]
    fn semantic_timeout_is_not_classified_as_transport_failure() {
        assert!(!super::is_transport_error(&AuError::code(
            "E_TIMEOUT",
            "selector did not appear"
        )));
        assert!(super::is_transport_error(&AuError::code(
            "E_HELPER",
            "helper read response timed out"
        )));
    }
}
