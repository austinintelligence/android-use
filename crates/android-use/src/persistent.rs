use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::adb::Adb;
use crate::cli::Cli;
use crate::config::Config;
use crate::device::{endpoint_matches_requested, DeviceInventory, Endpoint};
use crate::error::{AuError, Result};
use crate::MAX_OUTPUT_BYTES;

#[derive(Clone, Debug)]
pub struct ShellReply {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
}

enum ShellEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Eof,
}

pub struct PersistentShell {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<ShellEvent>,
    readers: Vec<JoinHandle<()>>,
    nonce: String,
    sequence: u64,
    pending: Vec<u8>,
    stdout_overflow: Arc<AtomicBool>,
    stderr_overflow: Arc<AtomicBool>,
}

impl PersistentShell {
    pub fn spawn(adb: &Adb, serial: &str) -> Result<Self> {
        let mut child = Command::new(adb.path())
            // Disable PTY allocation so the completion frame stays byte-accurate and
            // ADB does not add terminal buffering/translation overhead.
            .args(["-s", serial, "shell", "-T"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                AuError::code("E_SHELL", format!("start persistent shell: {error}"))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AuError::code("E_SHELL", "persistent shell stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AuError::code("E_SHELL", "persistent shell stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AuError::code("E_SHELL", "persistent shell stderr unavailable"))?;
        let (sender, events) = mpsc::sync_channel(32);
        let stdout_overflow = Arc::new(AtomicBool::new(false));
        let stderr_overflow = Arc::new(AtomicBool::new(false));
        let readers = vec![
            reader(stdout, true, sender.clone(), Arc::clone(&stdout_overflow)),
            reader(stderr, false, sender, Arc::clone(&stderr_overflow)),
        ];
        Ok(Self {
            child,
            stdin,
            events,
            readers,
            nonce: nonce(),
            sequence: 0,
            pending: Vec::new(),
            stdout_overflow,
            stderr_overflow,
        })
    }

    pub fn transact(&mut self, script: &str, deadline: Duration) -> Result<ShellReply> {
        self.sequence = self.sequence.wrapping_add(1);
        let zero_noop = script.trim() == ":";
        let marker = if zero_noop {
            // The zero-wait proof has no failure path, so it needs no status
            // byte. Keeping the nonce and sequence still prevents a stale or
            // unrelated frame from completing the transaction.
            format!("\x1eAU:{}:{}", self.nonce, self.sequence)
        } else {
            format!("\x1eAU:{}:{}:", self.nonce, self.sequence)
        };
        let wrapped = if zero_noop {
            // A zero-wait batch is a real remote transaction, but its status is
            // known. Avoid the extra command group, `$?` assignment, and
            // shell expansion while retaining the same framed completion
            // protocol and error-proof transport measurement.
            format!("printf '\\036AU:{}:{}\\037'", self.nonce, self.sequence)
        } else {
            format!(
                "{{ {script}; }}; __au_rc=$?; printf '\\036AU:{}:{}:%s\\037' \"$__au_rc\"",
                self.nonce, self.sequence
            )
        };
        // The ADB shell is already persistent. Execute the bounded transaction in that
        // shell instead of starting a second remote `sh -c` for every request.
        // `script` is produced by batch lowering and quotes every user-controlled
        // argument, while the marker is generated locally and contains no input data.
        let command = format!("{wrapped}\n");
        self.stdin.write_all(command.as_bytes())?;
        self.stdin.flush()?;
        let started = Instant::now();
        let mut stderr = Vec::new();
        loop {
            if self.stdout_overflow.load(Ordering::Relaxed)
                || self.stderr_overflow.load(Ordering::Relaxed)
            {
                self.terminate();
                return Err(AuError::code(
                    "E_OUTPUT_LIMIT",
                    "persistent shell event queue exceeded its bound",
                ));
            }
            if let Some(reply) = self.try_finish(&marker, &mut stderr, started.elapsed())? {
                return Ok(reply);
            }
            let remaining = deadline.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                self.terminate();
                return Err(AuError::code(
                    "E_TIMEOUT",
                    "persistent remote shell transaction timed out",
                ));
            }
            match self
                .events
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(ShellEvent::Stdout(bytes)) => {
                    append_bounded(&mut self.pending, &bytes, "persistent shell stdout")?;
                }
                Ok(ShellEvent::Stderr(bytes)) => {
                    append_bounded(&mut stderr, &bytes, "persistent shell stderr")?;
                }
                Ok(ShellEvent::Eof) => {
                    self.terminate();
                    return Err(AuError::code("E_SHELL", "persistent remote shell exited"));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.terminate();
                    return Err(AuError::code(
                        "E_SHELL",
                        "persistent shell reader disconnected",
                    ));
                }
            }
        }
    }

    fn try_finish(
        &mut self,
        marker: &str,
        stderr: &mut Vec<u8>,
        elapsed: Duration,
    ) -> Result<Option<ShellReply>> {
        let marker = marker.as_bytes();
        let Some(start) = find_bytes(&self.pending, marker) else {
            return Ok(None);
        };
        let payload_start = start + marker.len();
        let Some(end_offset) = self.pending[payload_start..]
            .iter()
            .position(|byte| *byte == 0x1f)
        else {
            return Ok(None);
        };
        let end = payload_start + end_offset;
        let status_bytes = &self.pending[payload_start..end];
        let status = if status_bytes.is_empty() && !marker.ends_with(b":") {
            0
        } else {
            std::str::from_utf8(status_bytes)
                .map_err(|_| {
                    AuError::code("E_SHELL", "persistent shell returned invalid status frame")
                })?
                .parse::<i32>()
                .map_err(|_| AuError::code("E_SHELL", "persistent shell returned invalid status"))?
        };
        let output = self.pending[..start].to_vec();
        let remainder = self.pending[end + 1..].to_vec();
        self.pending = remainder;
        if status != 0 {
            let summary = String::from_utf8_lossy(&output)
                .chars()
                .take(300)
                .collect::<String>();
            let stderr = String::from_utf8_lossy(stderr)
                .chars()
                .take(300)
                .collect::<String>();
            let message = format!("shell exit {status}: {summary} {stderr}");
            return Err(AuError::code("E_REMOTE", message.trim()));
        }
        Ok(Some(ShellReply {
            stdout: output,
            stderr: std::mem::take(stderr),
            elapsed,
        }))
    }

    pub fn terminate(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}

impl Drop for PersistentShell {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub struct ShellPool {
    adb: Adb,
    shells: HashMap<String, PersistentShell>,
    selected: Option<Endpoint>,
}

impl ShellPool {
    pub fn new(adb: Adb) -> Self {
        Self {
            adb,
            shells: HashMap::new(),
            selected: None,
        }
    }

    /// Resolve a target once per healthy daemon session. A cache hit is valid only while
    /// it still names the configured hardware and, when requested, the exact endpoint.
    /// Any persistent-shell failure clears it, forcing the next request to rediscover
    /// endpoints and apply the USB > known Wi-Fi > matching mDNS policy again.
    pub fn endpoint(&mut self, cli: &Cli, config: &Config) -> Result<Endpoint> {
        if let Some(endpoint) = self.selected.as_ref() {
            let requested_matches = endpoint_matches_requested(endpoint, cli.serial.as_deref());
            let identity_matches = config.identity_matches(endpoint.hardware_serial.as_deref());
            if requested_matches && identity_matches {
                return Ok(endpoint.clone());
            }
        }
        let inventory = DeviceInventory::discover_for_identity(
            &self.adb,
            config.enrolled_serial().unwrap_or_default(),
        )?;
        let endpoint = inventory.resolve(config, cli.serial.as_deref())?;
        self.selected = Some(endpoint.clone());
        Ok(endpoint)
    }

    pub fn transact(
        &mut self,
        serial: &str,
        script: &str,
        deadline: Duration,
    ) -> Result<ShellReply> {
        if !self.shells.contains_key(serial) {
            self.shells
                .insert(serial.into(), PersistentShell::spawn(&self.adb, serial)?);
        }
        let result = self
            .shells
            .get_mut(serial)
            .ok_or_else(|| AuError::code("E_SHELL", "persistent shell was not retained"))?
            .transact(script, deadline);
        if result.is_err() {
            self.shells.remove(serial);
            if self
                .selected
                .as_ref()
                .is_some_and(|endpoint| endpoint.endpoint == serial)
            {
                self.selected = None;
            }
        }
        result
    }

    pub fn shutdown(&mut self) {
        self.shells.clear();
    }
}

fn reader<R: Read + Send + 'static>(
    mut source: R,
    stdout: bool,
    sender: SyncSender<ShellEvent>,
    overflow: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    if matches!(sender.try_send(ShellEvent::Eof), Err(TrySendError::Full(_))) {
                        overflow.store(true, Ordering::Relaxed);
                    }
                    return;
                }
                Ok(count) => {
                    let event = if stdout {
                        ShellEvent::Stdout(buffer[..count].to_vec())
                    } else {
                        ShellEvent::Stderr(buffer[..count].to_vec())
                    };
                    match sender.try_send(event) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            overflow.store(true, Ordering::Relaxed);
                            return;
                        }
                        Err(TrySendError::Disconnected(_)) => return,
                    }
                }
            }
        }
    })
}

fn append_bounded(target: &mut Vec<u8>, bytes: &[u8], label: &str) -> Result<()> {
    if target.len().saturating_add(bytes.len()) > MAX_OUTPUT_BYTES {
        return Err(AuError::code(
            "E_OUTPUT_LIMIT",
            format!("{label} exceeded {MAX_OUTPUT_BYTES} bytes"),
        ));
    }
    target.extend_from_slice(bytes);
    Ok(())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn nonce() -> String {
    let ticks = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{ticks:x}-{:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::find_bytes;

    #[test]
    fn finds_split_safe_frame_marker() {
        assert_eq!(
            find_bytes(b"before\x1eAU:x:1:0\x1fafter", b"\x1eAU:x:1:"),
            Some(6)
        );
    }
}
