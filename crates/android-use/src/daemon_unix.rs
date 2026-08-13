use std::env;
use std::fs::{self};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{atomic_write, AppPaths};
use crate::error::{AuError, Result};
use crate::protocol::{
    read_daemon_request, read_native_response, validate_request, write_daemon_response,
    write_native_request, FrameMode, Request, RequestBody, Response,
};
use crate::{trace, PROTOCOL_VERSION, VERSION};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonState {
    pub pid: u32,
    pub executable: String,
    pub binary_version: String,
    pub protocol_version: u16,
}

pub fn client_execute(argv: Vec<String>) -> Result<Value> {
    let _span = trace::span("daemon.client_execute", json!({"a":argv.len()}));
    response_to_value(send(Request {
        version: PROTOCOL_VERSION,
        id: request_id(),
        body: RequestBody::Execute { argv },
    })?)
}

pub fn execute_or_start(paths: &AppPaths, argv: Vec<String>) -> Result<Value> {
    match client_execute(argv.clone()) {
        Ok(value) => Ok(value),
        Err(error) if is_connectivity_error(&error) => {
            ensure_started(paths)?;
            client_execute(argv)
        }
        Err(error) => Err(error),
    }
}

fn is_connectivity_error(error: &AuError) -> bool {
    let message = error.compact_message();
    message.contains("connect daemon socket")
        || message.contains("write daemon request")
        || message.contains("read daemon response")
}

pub fn ensure_started(paths: &AppPaths) -> Result<()> {
    if hello().is_ok() {
        return Ok(());
    }
    let executable =
        env::current_exe().map_err(|error| AuError::code("E_DAEMON", error.to_string()))?;
    spawn_daemon(&executable)?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(1_500) {
        if hello().is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(15));
    }
    let state = read_state(paths).ok();
    Err(AuError::code(
        "E_DAEMON",
        format!("daemon did not complete handshake; state={state:?}"),
    ))
}

fn spawn_daemon(executable: &Path) -> Result<()> {
    let mut command = Command::new(executable);
    command
        .args(["--daemon-child", "daemon", "serve"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    command
        .spawn()
        .map_err(|error| AuError::code("E_DAEMON", format!("start daemon: {error}")))?;
    Ok(())
}

pub fn hello() -> Result<Value> {
    response_to_value(send(Request {
        version: PROTOCOL_VERSION,
        id: request_id(),
        body: RequestBody::Hello {
            client_version: VERSION.into(),
        },
    })?)
}

pub fn stop(paths: &AppPaths) -> Result<()> {
    let state = read_state(paths)?;
    if !process_is_alive(state.pid) {
        return Err(AuError::code(
            "E_DAEMON",
            "recorded daemon process is not alive",
        ));
    }
    validate_private_socket(paths)?;
    let mut stream = open_client_socket_for(paths)?;
    let identity = response_to_value(send_on_stream(
        &mut stream,
        &Request {
            version: PROTOCOL_VERSION,
            id: request_id(),
            body: RequestBody::Hello {
                client_version: VERSION.into(),
            },
        },
    )?)?;
    let pid = identity
        .get("pid")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    if pid != state.pid
        || identity.get("executable").and_then(Value::as_str) != Some(state.executable.as_str())
        || identity.get("binary_version").and_then(Value::as_str) != Some(VERSION)
        || identity.get("protocol_version").and_then(Value::as_u64) != Some(PROTOCOL_VERSION as u64)
    {
        return Err(AuError::code(
            "E_DAEMON",
            "daemon identity does not match its private socket and state record",
        ));
    }
    drop(stream);
    let mut stop_stream = open_client_socket_for(paths)?;
    response_to_value(send_on_stream(
        &mut stop_stream,
        &Request {
            version: PROTOCOL_VERSION,
            id: request_id(),
            body: RequestBody::Stop,
        },
    )?)?;
    Ok(())
}

fn process_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn status(paths: &AppPaths) -> Result<Value> {
    let handshake = hello()?;
    let state = read_state(paths)?;
    Ok(json!({"handshake":handshake,"state":state}))
}

pub fn serve<F>(paths: &AppPaths, mut execute: F) -> Result<()>
where
    F: FnMut(Vec<String>) -> Result<Value>,
{
    fs::set_permissions(&paths.state, fs::Permissions::from_mode(0o700))?;
    let socket = socket_path(paths);
    let listener = bind_private_socket(&socket)?;
    let executable = env::current_exe()?.display().to_string();
    let state = DaemonState {
        pid: std::process::id(),
        executable,
        binary_version: VERSION.into(),
        protocol_version: PROTOCOL_VERSION,
    };
    atomic_write(&paths.daemon, &serde_json::to_vec(&state)?)?;
    let result = serve_loop(listener, &state, &mut execute);
    let _ = remove_own_state(paths, state.pid);
    let _ = fs::remove_file(socket);
    result
}

fn bind_private_socket(path: &Path) -> Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(listener)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Err(AuError::code("E_DAEMON", "daemon already running"));
            }
            fs::remove_file(path).map_err(|remove| {
                AuError::code(
                    "E_DAEMON",
                    format!("remove stale daemon socket after {error}: {remove}"),
                )
            })?;
            let listener = UnixListener::bind(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(listener)
        }
        Err(error) => Err(AuError::code(
            "E_DAEMON",
            format!("bind daemon socket: {error}"),
        )),
    }
}

fn serve_loop<F>(listener: UnixListener, state: &DaemonState, execute: &mut F) -> Result<()>
where
    F: FnMut(Vec<String>) -> Result<Value>,
{
    loop {
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| AuError::code("E_DAEMON", format!("accept daemon socket: {error}")))?;
        loop {
            let (request, mode) = match read_daemon_request(&mut stream) {
                Ok(request) => request,
                Err(error) => {
                    let response = Response::error(0, &error);
                    let _ = write_daemon_response(&mut stream, &response, FrameMode::Json);
                    break;
                }
            };
            let (response, stop) = handle_request(state, request, execute);
            if write_daemon_response(&mut stream, &response, mode).is_err() || stop {
                if stop {
                    return Ok(());
                }
                break;
            }
        }
    }
}

fn handle_request<F>(state: &DaemonState, request: Request, execute: &mut F) -> (Response, bool)
where
    F: FnMut(Vec<String>) -> Result<Value>,
{
    if let Err(error) = validate_request(&request) {
        return (Response::error(request.id, &error), false);
    }
    match request.body {
        RequestBody::Hello { .. } => (
            Response::ok(
                request.id,
                json!({
                    "pid":state.pid,
                    "executable":state.executable,
                    "binary_version":state.binary_version,
                    "protocol_version":state.protocol_version,
                    "capabilities":["framed-ipc","unix-socket","persistent-shell","bounded-exec"]
                }),
            ),
            false,
        ),
        RequestBody::Execute { argv } => match execute(argv) {
            Ok(value) => (Response::ok(request.id, value), false),
            Err(error) => (Response::error(request.id, &error), false),
        },
        RequestBody::Stop => (Response::ok(request.id, json!({"stopping":true})), true),
    }
}

fn response_to_value(response: Response) -> Result<Value> {
    if response.version != PROTOCOL_VERSION {
        return Err(AuError::code(
            "E_PROTOCOL",
            "daemon replied with incompatible version",
        ));
    }
    if response.ok {
        return response
            .data
            .ok_or_else(|| AuError::code("E_PROTOCOL", "daemon response omitted data"));
    }
    let error = response.error.unwrap_or(crate::protocol::ProtocolError {
        code: "E_PROTOCOL".into(),
        message: "daemon response omitted error".into(),
        details: None,
    });
    Err(AuError::protocol(error.code, error.message).with_optional_details(error.details))
}

fn send(request: Request) -> Result<Response> {
    let paths = AppPaths::discover()?;
    let mut stream = open_client_socket_retry(&paths)?;
    send_on_stream(&mut stream, &request)
}

fn send_on_stream(stream: &mut UnixStream, request: &Request) -> Result<Response> {
    write_native_request(stream, request).map_err(|error| {
        AuError::code(
            "E_DAEMON",
            format!("write daemon request: {}", error.compact_message()),
        )
    })?;
    read_native_response(stream).map_err(|error| {
        AuError::code(
            "E_DAEMON",
            format!("read daemon response: {}", error.compact_message()),
        )
    })
}

fn open_client_socket_for(paths: &AppPaths) -> Result<UnixStream> {
    UnixStream::connect(socket_path(paths))
        .map_err(|error| AuError::code("E_DAEMON", format!("connect daemon socket: {error}")))
}

fn open_client_socket_retry(paths: &AppPaths) -> Result<UnixStream> {
    let started = Instant::now();
    loop {
        match open_client_socket_for(paths) {
            Ok(stream) => return Ok(stream),
            Err(error) if started.elapsed() < Duration::from_millis(500) => {
                let _ = error;
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
}

fn validate_private_socket(paths: &AppPaths) -> Result<()> {
    let metadata = fs::metadata(socket_path(paths))?;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(AuError::code(
            "E_DAEMON",
            "daemon socket is accessible outside the current user",
        ));
    }
    Ok(())
}

fn socket_path(paths: &AppPaths) -> PathBuf {
    paths.state.join("daemon.sock")
}

fn read_state(paths: &AppPaths) -> Result<DaemonState> {
    let text = fs::read_to_string(&paths.daemon)
        .map_err(|error| AuError::code("E_DAEMON", format!("read daemon state: {error}")))?;
    serde_json::from_str(&text).map_err(|error| AuError::code("E_DAEMON", error.to_string()))
}

fn remove_own_state(paths: &AppPaths, pid: u32) -> Result<()> {
    if let Ok(state) = read_state(paths) {
        if state.pid == pid {
            fs::remove_file(&paths.daemon)?;
        }
    }
    Ok(())
}

fn request_id() -> u64 {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    epoch ^ ((std::process::id() as u64) << 32)
}

#[cfg(test)]
mod tests {
    use super::{response_to_value, DaemonState};
    use crate::protocol::{ProtocolError, Response};
    use crate::PROTOCOL_VERSION;

    #[test]
    fn state_serializes_with_protocol_identity() {
        let state = DaemonState {
            pid: 1,
            executable: "au".into(),
            binary_version: "1.0.0".into(),
            protocol_version: 1,
        };
        assert!(serde_json::to_string(&state)
            .expect("json")
            .contains("protocol_version"));
    }

    #[test]
    fn daemon_preserves_action_error_codes_across_ipc() {
        let response = Response {
            version: PROTOCOL_VERSION,
            id: 7,
            ok: false,
            data: None,
            error: Some(ProtocolError {
                code: "E_STALE".into(),
                message: "stale node handle".into(),
                details: Some(serde_json::json!({"next":"observe"})),
            }),
        };
        let error = response_to_value(response).expect_err("action error");
        assert_eq!(error.kind(), "E_STALE");
    }
}
