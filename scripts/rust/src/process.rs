use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::{AuError, Result};
use crate::trace;
use crate::MAX_OUTPUT_BYTES;

#[derive(Clone, Debug)]
pub enum CaptureDestination {
    Memory,
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub deadline: Duration,
    pub output_limit: usize,
    pub stdout: CaptureDestination,
    pub stderr: CaptureDestination,
    pub cancellation: Arc<AtomicBool>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(8),
            output_limit: MAX_OUTPUT_BYTES,
            stdout: CaptureDestination::Memory,
            stderr: CaptureDestination::Memory,
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Capture {
    pub bytes: Vec<u8>,
    pub total_bytes: u64,
    pub sha256: String,
    pub truncated: bool,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct ProcessResult {
    pub status: ExitStatus,
    pub stdout: Capture,
    pub stderr: Capture,
    pub elapsed: Duration,
}

pub fn run(command: &mut Command, options: RunOptions) -> Result<ProcessResult> {
    let program = Path::new(command.get_program())
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("child");
    let _span = trace::span("child.run", json!({"p":program}));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| AuError::code("E_SPAWN", error.to_string()))?;
    run_child(&mut child, options)
}

pub fn run_child(child: &mut Child, options: RunOptions) -> Result<ProcessResult> {
    let started = Instant::now();
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate(child)?;
            return Err(AuError::code("E_PROCESS", "stdout pipe unavailable"));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate(child)?;
            return Err(AuError::code("E_PROCESS", "stderr pipe unavailable"));
        }
    };
    let stdout_worker = drain(stdout, options.output_limit, options.stdout.clone());
    let stderr_worker = drain(stderr, options.output_limit, options.stderr.clone());
    let status = wait_with_deadline(child, options.deadline, &options.cancellation);
    // A timeout/cancellation kills the child, which closes both pipes. Always join
    // drain workers before returning so neither threads nor temporary file handles
    // outlive a failed ADB/helper invocation.
    let stdout = stdout_worker
        .join()
        .map_err(|_| AuError::code("E_PROCESS", "stdout reader panicked"))?;
    let stderr = stderr_worker
        .join()
        .map_err(|_| AuError::code("E_PROCESS", "stderr reader panicked"))?;
    let status = status?;
    let stdout = stdout?;
    let stderr = stderr?;
    Ok(ProcessResult {
        status,
        stdout,
        stderr,
        elapsed: started.elapsed(),
    })
}

fn wait_with_deadline(
    child: &mut Child,
    deadline: Duration,
    cancellation: &AtomicBool,
) -> Result<ExitStatus> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if cancellation.load(Ordering::Relaxed) {
            terminate(child)?;
            return Err(AuError::code("E_CANCELLED", "child process cancelled"));
        }
        if started.elapsed() >= deadline {
            terminate(child)?;
            return Err(AuError::code(
                "E_TIMEOUT",
                "child process deadline exceeded",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn terminate(child: &mut Child) -> Result<()> {
    match child.kill() {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
        Err(error) => return Err(error.into()),
    }
    let _ = child.wait();
    Ok(())
}

fn drain<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    destination: CaptureDestination,
) -> thread::JoinHandle<Result<Capture>> {
    thread::spawn(move || {
        let mut file = match &destination {
            CaptureDestination::Memory => None,
            CaptureDestination::File(path) => Some(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|error| {
                        AuError::code("E_PATH", format!("create {}: {error}", path.display()))
                    })?,
            ),
        };
        let mut result = Vec::new();
        let mut total_bytes = 0u64;
        let mut truncated = false;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let chunk = &buffer[..read];
            total_bytes += read as u64;
            hasher.update(chunk);
            if let Some(file) = file.as_mut() {
                file.write_all(chunk)?;
            } else if result.len() < limit {
                let keep = (limit - result.len()).min(read);
                result.extend_from_slice(&chunk[..keep]);
                truncated |= keep < read;
            } else {
                truncated = true;
            }
        }
        if let Some(file) = file.as_mut() {
            file.flush()?;
            file.sync_all()?;
        }
        let path = match destination {
            CaptureDestination::Memory => None,
            CaptureDestination::File(path) => Some(path),
        };
        Ok(Capture {
            bytes: result,
            total_bytes,
            sha256: format!("{:x}", hasher.finalize()),
            truncated,
            path,
        })
    })
}

pub fn text(capture: &Capture) -> String {
    String::from_utf8_lossy(&capture.bytes).trim().to_owned()
}

pub fn execute_for_test(
    command: &str,
    arguments: &[&str],
    deadline: Duration,
) -> Result<ProcessResult> {
    let mut child = Command::new(command);
    child.args(arguments);
    run(
        &mut child,
        RunOptions {
            deadline,
            ..RunOptions::default()
        },
    )
}

pub fn stream_to_file_command(
    command: &mut Command,
    output_path: PathBuf,
    deadline: Duration,
) -> Result<ProcessResult> {
    run(
        command,
        RunOptions {
            deadline,
            stdout: CaptureDestination::File(output_path),
            ..RunOptions::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;
    use std::time::Duration;

    use super::{run, stream_to_file_command, CaptureDestination, RunOptions};

    #[test]
    fn large_output_is_capped() {
        let mut command = Command::new("cmd");
        let payload = "1234567890".repeat(200);
        command.args(["/c", &format!("echo {payload}")]);
        let result = run(
            &mut command,
            RunOptions {
                deadline: Duration::from_secs(15),
                output_limit: 100,
                stdout: CaptureDestination::Memory,
                ..RunOptions::default()
            },
        )
        .expect("run");
        assert!(result.stdout.truncated);
        assert_eq!(result.stdout.bytes.len(), 100);
    }

    #[test]
    fn deadline_terminates_hung_child() {
        let mut command = Command::new("cmd");
        command.args(["/c", "ping -n 5 127.0.0.1 > nul"]);
        let error = run(
            &mut command,
            RunOptions {
                deadline: Duration::from_millis(30),
                ..RunOptions::default()
            },
        )
        .expect_err("timeout");
        assert_eq!(error.kind(), "E_TIMEOUT");
    }

    #[test]
    fn large_output_can_stream_to_a_non_clobbering_file() {
        let root = tempfile::tempdir().expect("temp");
        let path = root.path().join("large.txt");
        let mut command = Command::new("cmd");
        command.args(["/c", "for /L %i in (1,1,1000) do @echo streamed-data"]);
        let result = stream_to_file_command(&mut command, path.clone(), Duration::from_secs(5))
            .expect("stream file");
        assert!(result.stdout.bytes.is_empty());
        assert_eq!(result.stdout.path.as_deref(), Some(path.as_path()));
        assert_eq!(
            result.stdout.total_bytes,
            fs::metadata(path).expect("file").len()
        );
        assert!(!result.stdout.sha256.is_empty());
    }
}
