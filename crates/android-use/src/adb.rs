use std::env;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::config::Config;
use crate::error::{AuError, Result};
use crate::process::{run, text, CaptureDestination, ProcessResult, RunOptions};
use crate::trace;

const MAX_HOST_SERVICE_BYTES: usize = 65_535;

#[derive(Clone, Debug)]
pub struct Adb {
    path: PathBuf,
    timeout: Duration,
    server_addr: Option<SocketAddr>,
}

impl Adb {
    pub fn from_config(config: &Config, timeout_ms: u64) -> Result<Self> {
        let path = locate_adb(config)?;
        Ok(Self {
            path,
            timeout: Duration::from_millis(timeout_ms),
            server_addr: local_adb_server_addr(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn with_timeout(&self, timeout: Duration) -> Self {
        Self {
            path: self.path.clone(),
            timeout,
            server_addr: self.server_addr,
        }
    }

    /// Restart only a loopback ADB server. This is intentionally exposed for
    /// an explicit repair flow, never as an implicit retry around mutations:
    /// killing the shared server invalidates every transport and forward.
    pub fn restart_local_server(&self) -> Result<()> {
        if self.server_addr.is_none() {
            return Err(AuError::code(
                "E_ADB_REPAIR",
                "refusing to restart a non-loopback or unsupported ADB server",
            ));
        }
        let _ = self.global(&["kill-server".into()]);
        self.global(&["start-server".into()])?;
        trace::event("adb.server.restarted", json!({"scope":"loopback"}));
        Ok(())
    }

    /// Read the ADB server inventory without spawning `adb` when the standard
    /// server is on loopback. A failed direct query falls back to the official
    /// client so it can start or repair the server. Mutating and device-shell
    /// services deliberately remain owned by the official platform-tools CLI.
    pub fn devices_long(&self) -> Result<String> {
        if let Some(server_addr) = self.server_addr {
            match adb_host_query(server_addr, "host:devices-l", self.timeout) {
                Ok(body) => {
                    trace::event("adb.host", json!({"op":"devices-l","path":"direct"}));
                    return Ok(format!("List of devices attached\n{body}"));
                }
                Err(error) => trace::event(
                    "adb.host.fallback",
                    json!({"op":"devices-l","e":error.kind()}),
                ),
            }
        }
        let result = self.global(&["devices".into(), "-l".into()])?;
        Ok(text(&result.stdout))
    }

    /// Query one already-selected transport's state over the bounded local
    /// host protocol. Endpoint text is never interpreted as another service.
    pub fn get_state(&self, serial: &str) -> Result<String> {
        if valid_service_serial(serial) {
            if let Some(server_addr) = self.server_addr {
                let service = format!("host-serial:{serial}:get-state");
                match adb_host_query(server_addr, &service, self.timeout) {
                    Ok(body) if !body.trim().is_empty() => {
                        trace::event("adb.host", json!({"op":"get-state","path":"direct"}));
                        return Ok(body.trim().to_owned());
                    }
                    Ok(_) => {
                        trace::event("adb.host.fallback", json!({"op":"get-state","e":"E_EMPTY"}))
                    }
                    Err(error) => trace::event(
                        "adb.host.fallback",
                        json!({"op":"get-state","e":error.kind()}),
                    ),
                }
            }
        }
        let result = self.device(serial, &["get-state".into()])?;
        Ok(text(&result.stdout))
    }

    pub fn global(&self, args: &[String]) -> Result<ProcessResult> {
        self.invoke(None, args, CaptureDestination::Memory)
    }

    pub fn device(&self, serial: &str, args: &[String]) -> Result<ProcessResult> {
        if args.first().map(String::as_str) == Some("shell") && args.len() > 1 {
            let script = fixed_shell_command(&args[1..]);
            return self.invoke(
                Some(serial),
                &shell_script_args(&script),
                CaptureDestination::Memory,
            );
        }
        self.invoke(Some(serial), args, CaptureDestination::Memory)
    }

    pub fn device_to_file(
        &self,
        serial: &str,
        args: &[String],
        path: PathBuf,
    ) -> Result<ProcessResult> {
        self.invoke(Some(serial), args, CaptureDestination::File(path))
    }

    pub fn shell_script(&self, serial: &str, script: &str) -> Result<ProcessResult> {
        // Android 13's ADB shell protocol treats each argument as an exact
        // boundary. Passing `sh -c script` splits the script at the protocol
        // boundary and makes `sh` execute only the first token. A single
        // `shell` script argument is interpreted by the device shell and keeps
        // the quoting produced by fixed_shell_command intact.
        self.invoke(
            Some(serial),
            &shell_script_args(script),
            CaptureDestination::Memory,
        )
    }

    pub fn raw_shell(&self, serial: &str, raw: &[String]) -> Result<ProcessResult> {
        let mut args = Vec::with_capacity(raw.len() + 1);
        args.push("shell".into());
        args.extend(raw.iter().cloned());
        self.invoke(Some(serial), &args, CaptureDestination::Memory)
    }

    fn invoke(
        &self,
        serial: Option<&str>,
        args: &[String],
        stdout: CaptureDestination,
    ) -> Result<ProcessResult> {
        let mut command = Command::new(&self.path);
        if let Some(serial) = serial {
            command.args(["-s", serial]);
        }
        command.args(args);
        let result = run(
            &mut command,
            RunOptions {
                deadline: self.timeout,
                stdout,
                stderr: CaptureDestination::Memory,
                cancellation: Arc::new(AtomicBool::new(false)),
                ..RunOptions::default()
            },
        )?;
        if !result.status.success() {
            let message = bounded_error(&result);
            return Err(AuError::code("E_ADB", message));
        }
        Ok(result)
    }
}

fn local_adb_server_addr() -> Option<SocketAddr> {
    if let Ok(value) = env::var("ADB_SERVER_SOCKET") {
        return parse_loopback_server_socket(&value);
    }
    let port = match env::var("ANDROID_ADB_SERVER_PORT").or_else(|_| env::var("ADB_SERVER_PORT")) {
        Ok(value) => value.parse::<u16>().ok()?,
        Err(_) => 5037,
    };
    Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
}

fn parse_loopback_server_socket(value: &str) -> Option<SocketAddr> {
    let value = value.strip_prefix("tcp:")?;
    if let Ok(port) = value.parse::<u16>() {
        return Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    }
    let (host, port) = value.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    match host.trim_matches(['[', ']']) {
        "127.0.0.1" | "localhost" => Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)),
        "::1" => Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)),
        _ => None,
    }
}

fn valid_service_serial(serial: &str) -> bool {
    !serial.is_empty()
        && serial.len() <= 255
        && serial
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'\0')
}

fn adb_host_query(addr: SocketAddr, service: &str, timeout: Duration) -> Result<String> {
    if service.is_empty() || service.len() > 4096 || !service.is_ascii() {
        return Err(AuError::code("E_ADB_HOST", "invalid ADB host service"));
    }
    let started = Instant::now();
    let mut stream = TcpStream::connect_timeout(&addr, remaining(started, timeout)?)
        .map_err(|error| host_io("connect", error))?;
    let _ = stream.set_nodelay(true);
    set_deadline(&stream, started, timeout)?;
    let request = format!("{:04X}{service}", service.len());
    stream
        .write_all(request.as_bytes())
        .map_err(|error| host_io("write", error))?;
    set_deadline(&stream, started, timeout)?;
    let mut status = [0u8; 4];
    stream
        .read_exact(&mut status)
        .map_err(|error| host_io("read status", error))?;
    match &status {
        b"OKAY" => read_host_string(&mut stream, started, timeout),
        b"FAIL" => {
            let message = read_host_string(&mut stream, started, timeout)
                .unwrap_or_else(|_| "ADB server rejected the service".into());
            Err(AuError::code(
                "E_ADB_HOST",
                message.chars().take(400).collect::<String>(),
            ))
        }
        _ => Err(AuError::code(
            "E_ADB_HOST",
            "ADB server returned an invalid status frame",
        )),
    }
}

fn read_host_string(stream: &mut TcpStream, started: Instant, timeout: Duration) -> Result<String> {
    set_deadline(stream, started, timeout)?;
    let mut encoded_length = [0u8; 4];
    stream
        .read_exact(&mut encoded_length)
        .map_err(|error| host_io("read length", error))?;
    let encoded_length = std::str::from_utf8(&encoded_length)
        .map_err(|_| AuError::code("E_ADB_HOST", "ADB response length was not ASCII"))?;
    let length = usize::from_str_radix(encoded_length, 16)
        .map_err(|_| AuError::code("E_ADB_HOST", "ADB response length was not hexadecimal"))?;
    if length > MAX_HOST_SERVICE_BYTES {
        return Err(AuError::code(
            "E_ADB_HOST",
            "ADB host response is too large",
        ));
    }
    let mut body = vec![0u8; length];
    set_deadline(stream, started, timeout)?;
    stream
        .read_exact(&mut body)
        .map_err(|error| host_io("read body", error))?;
    String::from_utf8(body)
        .map_err(|_| AuError::code("E_ADB_HOST", "ADB host response was not UTF-8"))
}

fn remaining(started: Instant, timeout: Duration) -> Result<Duration> {
    timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| AuError::code("E_TIMEOUT", "ADB host query deadline exceeded"))
}

fn set_deadline(stream: &TcpStream, started: Instant, timeout: Duration) -> Result<()> {
    let remaining = remaining(started, timeout)?;
    stream
        .set_read_timeout(Some(remaining))
        .map_err(|error| host_io("set read deadline", error))?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| host_io("set write deadline", error))
}

fn host_io(operation: &str, error: std::io::Error) -> AuError {
    let code = if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        "E_TIMEOUT"
    } else {
        "E_ADB_HOST"
    };
    AuError::code(code, format!("ADB host {operation}: {error}"))
}

pub fn shell_script_args(script: &str) -> Vec<String> {
    vec!["shell".into(), script.to_owned()]
}

pub fn locate_adb(config: &Config) -> Result<PathBuf> {
    let executable = if cfg!(windows) { "adb.exe" } else { "adb" };
    let mut candidates = vec![config.adb_path.clone()];
    candidates.push(
        env::var_os("AU_INSTALL_ROOT")
            .map(|root| PathBuf::from(root).join("platform-tools").join(executable)),
    );
    candidates.push(
        env::var_os("ANDROID_SDK_ROOT")
            .or_else(|| env::var_os("ANDROID_HOME"))
            .map(|root| PathBuf::from(root).join("platform-tools").join(executable)),
    );
    #[cfg(windows)]
    {
        candidates.push(env::var_os("LOCALAPPDATA").map(|root| {
            PathBuf::from(root)
                .join("Codex")
                .join("android-use")
                .join("platform-tools")
                .join("adb.exe")
        }));
        candidates.push(env::var_os("LOCALAPPDATA").map(|root| {
            PathBuf::from(root)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join("adb.exe")
        }));
        candidates.push(env::var_os("LOCALAPPDATA").map(|root| {
            PathBuf::from(root)
                .join("Codex")
                .join("android-agent-display")
                .join("platform-tools")
                .join("adb.exe")
        }));
    }
    #[cfg(target_os = "macos")]
    candidates.push(env::var_os("HOME").map(|root| {
        PathBuf::from(root)
            .join("Library")
            .join("Android")
            .join("sdk")
            .join("platform-tools")
            .join("adb")
    }));
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        candidates.push(env::var_os("HOME").map(|root| {
            PathBuf::from(root)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join("adb")
        }));
        candidates.push(env::var_os("HOME").map(|root| {
            PathBuf::from(root)
                .join("Android")
                .join("sdk")
                .join("platform-tools")
                .join("adb")
        }));
    }
    if let Some(path) = candidates.into_iter().flatten().find(|path| path.is_file()) {
        return Ok(path);
    }
    if let Some(path) = find_on_path(executable) {
        return Ok(path);
    }
    Err(AuError::code(
        "E_ADB",
        format!("{executable} not found; install Android platform-tools or configure adb_path"),
    ))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

pub fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

pub fn fixed_shell_command(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn bounded_error(result: &ProcessResult) -> String {
    let stderr = text(&result.stderr);
    let stdout = text(&result.stdout);
    let source = if stderr.is_empty() { stdout } else { stderr };
    if source.is_empty() {
        format!("adb exited {}", result.status)
    } else {
        source.chars().take(400).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use super::{
        adb_host_query, fixed_shell_command, parse_loopback_server_socket, shell_quote,
        shell_script_args, Adb,
    };

    #[test]
    fn shell_quote_preserves_metacharacters() {
        assert_eq!(shell_quote("a;$(touch x) ' z"), "'a;$(touch x) '\"'\"' z'");
    }

    #[test]
    fn fixed_command_quotes_each_boundary() {
        let command =
            fixed_shell_command(&["am".into(), "start".into(), "https://a/?x=1;id".into()]);
        assert_eq!(command, "'am' 'start' 'https://a/?x=1;id'");
    }

    #[test]
    fn shell_script_is_one_adb_argument() {
        assert_eq!(
            shell_script_args("input tap '640' '106'; input keyevent HOME"),
            ["shell", "input tap '640' '106'; input keyevent HOME"]
        );
    }

    #[test]
    fn direct_host_query_uses_bounded_classic_adb_framing() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut length = [0u8; 4];
            stream.read_exact(&mut length).expect("length");
            let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
            let mut request = vec![0u8; length];
            stream.read_exact(&mut request).expect("request");
            assert_eq!(request, b"host:devices-l");
            let body = b"fixture\tdevice transport_id:7\n";
            stream.write_all(b"OKAY").expect("status");
            stream
                .write_all(format!("{:04X}", body.len()).as_bytes())
                .expect("body length");
            stream.write_all(body).expect("body");
        });
        assert_eq!(
            adb_host_query(address, "host:devices-l", Duration::from_secs(1)).unwrap(),
            "fixture\tdevice transport_id:7\n"
        );
        worker.join().expect("server");
    }

    #[test]
    fn direct_host_query_preserves_bounded_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0u8; 8];
            stream.read_exact(&mut request[..4]).expect("length");
            let length =
                usize::from_str_radix(std::str::from_utf8(&request[..4]).unwrap(), 16).unwrap();
            let mut body = vec![0u8; length];
            stream.read_exact(&mut body).expect("request");
            stream.write_all(b"FAIL0004nope").expect("failure");
        });
        let error =
            adb_host_query(address, "host:version", Duration::from_secs(1)).expect_err("failure");
        assert_eq!(error.kind(), "E_ADB_HOST");
        assert_eq!(error.to_string(), "nope");
        worker.join().expect("server");
    }

    #[test]
    fn only_loopback_adb_server_sockets_use_the_direct_path() {
        assert_eq!(
            parse_loopback_server_socket("tcp:5038")
                .unwrap()
                .to_string(),
            "127.0.0.1:5038"
        );
        assert!(parse_loopback_server_socket("tcp:192.0.2.1:5037").is_none());
        assert!(parse_loopback_server_socket("localfilesystem:/tmp/adb").is_none());
    }

    #[test]
    fn repair_refuses_non_loopback_adb_servers_before_spawning_a_client() {
        let adb = Adb {
            path: "missing-adb-for-test".into(),
            timeout: Duration::from_secs(1),
            server_addr: None,
        };
        let error = adb.restart_local_server().expect_err("must fail closed");
        assert_eq!(error.kind(), "E_ADB_REPAIR");
    }
}
