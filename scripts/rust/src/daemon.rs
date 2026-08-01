use std::env;
use std::ffi::c_void;
use std::fs::{self, File};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, GENERIC_READ,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, CreateProcessW, GetCurrentProcess, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
};

use crate::config::{atomic_write, AppPaths};
use crate::error::{AuError, Result};
use crate::protocol::{
    read_daemon_request, read_native_response, validate_request, write_daemon_response,
    write_native_request, FrameMode, Request, RequestBody, Response,
};
use crate::{trace, PROTOCOL_VERSION, VERSION};

const PIPE_NAME: &str = r"\\.\pipe\codex-android-use-v1";
const MUTEX_NAME: &str = r"Local\codex-android-use-v1-daemon";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonState {
    pub pid: u32,
    pub executable: String,
    pub binary_version: String,
    pub protocol_version: u16,
}

pub fn client_execute(argv: Vec<String>) -> Result<Value> {
    let _span = trace::span("daemon.client_execute", json!({"a":argv.len()}));
    trace::event("daemon.request", json!({"a":argv.len()}));
    let response = send(Request {
        version: PROTOCOL_VERSION,
        id: request_id(),
        body: RequestBody::Execute { argv },
    })?;
    let value = response_to_value(response);
    trace::event(
        "daemon.response",
        match &value {
            Ok(_) => json!({"ok":true}),
            Err(error) => json!({"ok":false,"e":error.kind()}),
        },
    );
    value
}

/// Execute on the already-running daemon without an extra hello request.
///
/// The pipe response is itself framed and version-checked.  A handshake is
/// still required when starting, replacing, or stopping a daemon, but normal
/// commands should pay for one IPC round trip rather than hello + execute.
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
    message.contains("open daemon pipe")
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
    while started.elapsed() < Duration::from_millis(900) {
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
    let application = executable
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut command_line = "--daemon-child daemon serve"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(last_error("start daemon"));
    }
    unsafe {
        let _ = CloseHandle(process_info.hThread);
        let _ = CloseHandle(process_info.hProcess);
    }
    Ok(())
}

pub fn hello() -> Result<Value> {
    let response = send(Request {
        version: PROTOCOL_VERSION,
        id: request_id(),
        body: RequestBody::Hello {
            client_version: VERSION.into(),
        },
    })?;
    response_to_value(response)
}

pub fn stop(paths: &AppPaths) -> Result<()> {
    let state = read_state(paths).ok();
    let mut pipe = match state.as_ref().filter(|state| process_is_alive(state.pid)) {
        Some(state) => open_client_pipe_for_pid(state.pid).or_else(|_| open_client_pipe_retry())?,
        None => open_client_pipe_retry()?,
    };
    let server_pid = pipe_server_pid(&pipe)?;
    let identity = response_to_value(send_on_pipe(
        &mut pipe,
        &Request {
            version: PROTOCOL_VERSION,
            id: request_id(),
            body: RequestBody::Hello {
                client_version: VERSION.into(),
            },
        },
    )?)?;
    let expected_pid = identity
        .get("pid")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let executable = identity
        .get("executable")
        .and_then(Value::as_str)
        .ok_or_else(|| AuError::code("E_DAEMON", "daemon handshake omitted executable"))?;
    if expected_pid == 0
        || server_pid != expected_pid
        || identity.get("binary_version").and_then(Value::as_str) != Some(VERSION)
        || identity.get("protocol_version").and_then(Value::as_u64) != Some(PROTOCOL_VERSION as u64)
    {
        return Err(AuError::code(
            "E_DAEMON",
            format!(
                "daemon identity does not match trusted handshake: server_pid={server_pid} handshake_pid={expected_pid} executable={executable:?} version={:?} protocol={:?}",
                identity.get("binary_version"),
                identity.get("protocol_version")
            ),
        ));
    }
    if !process_owner_matches(expected_pid, executable)? {
        return Err(AuError::code(
            "E_DAEMON",
            "daemon is not owned by the current user",
        ));
    }
    drop(pipe);
    let mut stop_pipe = open_client_pipe_for_pid(expected_pid)?;
    let response = send_on_pipe(
        &mut stop_pipe,
        &Request {
            version: PROTOCOL_VERSION,
            id: request_id(),
            body: RequestBody::Stop,
        },
    )?;
    response_to_value(response)?;
    Ok(())
}

fn process_is_alive(pid: u32) -> bool {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    unsafe {
        let _ = CloseHandle(process);
    }
    true
}

fn process_owner_matches(pid: u32, expected_executable: &str) -> Result<bool> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(last_error("open daemon process for ownership check"));
    }
    let target = token_user_sid(process);
    let executable = process_executable(process);
    unsafe {
        let _ = CloseHandle(process);
    }
    let target = target?;
    let executable = executable?;
    let current = token_user_sid(unsafe { GetCurrentProcess() })?;
    Ok(target == current && executable.eq_ignore_ascii_case(expected_executable))
}

fn process_executable(process: HANDLE) -> Result<String> {
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    let result =
        unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) };
    if result == 0 {
        return Err(last_error("query daemon executable"));
    }
    Ok(String::from_utf16_lossy(&buffer[..length as usize]))
}

fn token_user_sid(process: HANDLE) -> Result<Vec<u8>> {
    let mut token = std::ptr::null_mut();
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(last_error("open process token"));
    }
    let result = (|| {
        let mut required = 0u32;
        unsafe {
            let _ = GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut required);
        }
        if required == 0 {
            return Err(last_error("query token user size"));
        }
        let mut buffer = vec![0u8; required as usize];
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr() as *mut c_void,
                required,
                &mut required,
            )
        };
        if ok == 0 {
            return Err(last_error("query token user"));
        }
        let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
        let length = unsafe { GetLengthSid(user.User.Sid) };
        if length == 0 {
            return Err(last_error("query user SID length"));
        }
        let sid =
            unsafe { std::slice::from_raw_parts(user.User.Sid as *const u8, length as usize) };
        Ok(sid.to_vec())
    })();
    unsafe {
        let _ = CloseHandle(token);
    }
    result
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
    let _mutex = acquire_daemon_mutex()?;
    // AU is a short-transaction control daemon. Above-normal priority is a
    // bounded latency optimization for the daemon and its inherited ADB
    // clients; it does not use realtime priority or alter other processes.
    unsafe {
        let _ = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
    }
    let executable = env::current_exe()?.display().to_string();
    let state = DaemonState {
        pid: std::process::id(),
        executable,
        binary_version: VERSION.into(),
        protocol_version: PROTOCOL_VERSION,
    };
    atomic_write(&paths.daemon, &serde_json::to_vec(&state)?)?;
    let result = serve_loop(&state, &mut execute);
    let _ = remove_own_state(paths, state.pid);
    result
}

struct DaemonMutex(HANDLE);

impl Drop for DaemonMutex {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn acquire_daemon_mutex() -> Result<DaemonMutex> {
    let name = wide(MUTEX_NAME);
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if handle.is_null() {
        return Err(last_error("create daemon mutex"));
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Err(AuError::code("E_DAEMON", "daemon already running"));
    }
    Ok(DaemonMutex(handle))
}

fn serve_loop<F>(state: &DaemonState, execute: &mut F) -> Result<()>
where
    F: FnMut(Vec<String>) -> Result<Value>,
{
    loop {
        let mut pipe = create_server_pipe()?;
        connect_server_pipe(&pipe)?;
        // Keep a client pipe alive across framed requests. One-shot `au` calls
        // still close after their response, while `au pipe` and native clients
        // can amortize named-pipe creation and handshake overhead across a
        // whole action stream.
        loop {
            let (request, mode) = match read_daemon_request(&mut pipe) {
                Ok(request) => request,
                Err(error) => {
                    // A malformed or truncated frame cannot be safely
                    // resynchronized. Preserve a structured error when the
                    // peer is still readable, then discard only this pipe
                    // instance and accept a fresh one.
                    let response = Response::error(0, &error);
                    let _ = write_daemon_response(&mut pipe, &response, FrameMode::Json);
                    break;
                }
            };
            let (response, stop) = handle_request(state, request, execute);
            if write_daemon_response(&mut pipe, &response, mode).is_err() || stop {
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
                    "capabilities":["framed-ipc","persistent-shell","bounded-exec"]
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
    });
    Err(AuError::protocol(error.code, error.message))
}

fn send(request: Request) -> Result<Response> {
    let mut pipe = open_client_pipe_retry()?;
    send_on_pipe(&mut pipe, &request)
}

fn send_on_pipe(pipe: &mut File, request: &Request) -> Result<Response> {
    write_native_request(pipe, request).map_err(|error| {
        AuError::code(
            "E_DAEMON",
            format!("write daemon request: {}", error.compact_message()),
        )
    })?;
    read_native_response(pipe).map_err(|error| {
        AuError::code(
            "E_DAEMON",
            format!("read daemon response: {}", error.compact_message()),
        )
    })
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

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn create_server_pipe() -> Result<File> {
    let name = wide(&pipe_name());
    let sddl = wide("D:P(A;;GA;;;OW)");
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let mut size = 0u32;
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            &mut size,
        )
    };
    if converted == 0 {
        return Err(last_error("build current-user named-pipe ACL"));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            65_536,
            65_536,
            1_000,
            &attributes,
        )
    };
    unsafe {
        LocalFree(descriptor);
    }
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error("create named pipe"));
    }
    let file = unsafe { File::from_raw_handle(handle as _) };
    Ok(file)
}

fn connect_server_pipe(pipe: &File) -> Result<()> {
    let result = unsafe { ConnectNamedPipe(pipe.as_raw_handle() as HANDLE, std::ptr::null_mut()) };
    if result != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
        Ok(())
    } else {
        Err(last_error("connect named pipe"))
    }
}

fn open_client_pipe() -> Result<File> {
    let name = wide(&pipe_name());
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error("open daemon pipe"));
    }
    let file = unsafe { File::from_raw_handle(handle as _) };
    Ok(file)
}

fn open_client_pipe_retry() -> Result<File> {
    let started = Instant::now();
    loop {
        match open_client_pipe() {
            Ok(pipe) => return Ok(pipe),
            Err(error) if started.elapsed() < Duration::from_millis(500) => {
                thread::sleep(Duration::from_millis(5));
                let _ = error;
            }
            Err(error) => return Err(error),
        }
    }
}

fn open_client_pipe_for_pid(expected_pid: u32) -> Result<File> {
    let started = Instant::now();
    let mut last_error = None;
    let mut last_server_pid = None;
    loop {
        match open_client_pipe() {
            Ok(pipe) => match pipe_server_pid(&pipe) {
                Ok(server_pid) if server_pid == expected_pid => return Ok(pipe),
                Ok(server_pid) => {
                    last_server_pid = Some(server_pid);
                    drop(pipe);
                }
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
        if started.elapsed() >= Duration::from_millis(500) {
            if let Some(error) = last_error {
                return Err(error);
            }
            return Err(AuError::code(
                "E_DAEMON",
                format!(
                    "could not find validated daemon pipe instance: expected={expected_pid} observed={last_server_pid:?}"
                ),
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn pipe_server_pid(pipe: &File) -> Result<u32> {
    let mut pid = 0u32;
    let result = unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) };
    if result == 0 || pid == 0 {
        return Err(last_error("query named-pipe server PID"));
    }
    Ok(pid)
}

fn pipe_name() -> String {
    PIPE_NAME.into()
}

fn last_error(context: &str) -> AuError {
    let code = unsafe { GetLastError() };
    AuError::code("E_DAEMON", format!("{context}: Windows error {code}"))
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
            executable: "au.exe".into(),
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
            }),
        };
        let error = response_to_value(response).expect_err("action error");
        assert_eq!(error.kind(), "E_STALE");
        assert_eq!(error.compact_message(), "stale node handle");
    }
}
