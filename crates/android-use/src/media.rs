use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use crate::adb::Adb;
use crate::config::AppPaths;
use crate::error::{AuError, Result};
use crate::files::{reserve_output, screenshot, Artifact};
use crate::helper;
use crate::process::{run, RunOptions};

pub struct MediaOptions<'a> {
    pub output: Option<&'a Path>,
    pub force: bool,
    pub binary: bool,
}

pub fn execute(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    command: &str,
    args: &[String],
    options: MediaOptions<'_>,
) -> Result<Value> {
    match command {
        "mirror" => mirror(serial, args),
        "screen" => screen(paths, serial, args, options.output, options.force),
        "cam" => camera(
            adb,
            paths,
            serial,
            args,
            options.output,
            options.force,
            options.binary,
        ),
        "mic" => microphone(
            adb,
            paths,
            serial,
            args,
            options.output,
            options.force,
            options.binary,
        ),
        "ss" | "screenshot" => {
            let path = reserve_output(paths, options.output, "screen", "png", options.force)?;
            let artifact = screenshot(adb, serial, path)?;
            Ok(artifact_json(&artifact))
        }
        _ => Err(AuError::code(
            "E_ARGS",
            format!("unknown media command {command}"),
        )),
    }
}

fn mirror(serial: &str, args: &[String]) -> Result<Value> {
    let duration = duration(args, 0, 120, "mirror [SECONDS]")?;
    let scrcpy = find_scrcpy()?;
    let mut command = Command::new(&scrcpy);
    command.args(["--serial", serial, "--time-limit", &duration.to_string()]);
    let result = run(
        &mut command,
        RunOptions {
            deadline: Duration::from_secs(duration + 10),
            ..RunOptions::default()
        },
    )?;
    if !result.status.success() {
        return Err(AuError::code(
            "E_SCRCPY",
            "scrcpy mirror exited unsuccessfully",
        ));
    }
    Ok(json!({"scrcpy":scrcpy.display().to_string(),"seconds":duration,"finished":true}))
}

fn screen(
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    output: Option<&Path>,
    force: bool,
) -> Result<Value> {
    match args.first().map(String::as_str).unwrap_or("record") {
        "record" => {
            let seconds = duration(args, 1, 3, "screen record [SECONDS]")?;
            let destination = reserve_output(paths, output, "screen-record", "mp4", force)?;
            let scrcpy = find_scrcpy()?;
            let mut command = Command::new(&scrcpy);
            command
                .args(["--serial", serial, "--time-limit", &seconds.to_string()])
                .arg("--record")
                .arg(&destination)
                .arg("--no-window");
            let result = run(
                &mut command,
                RunOptions {
                    deadline: Duration::from_secs(seconds + 10),
                    ..RunOptions::default()
                },
            )?;
            if !result.status.success() {
                return Err(AuError::code(
                    "E_SCRCPY",
                    "scrcpy screen recording exited unsuccessfully",
                ));
            }
            let artifact = local_artifact(destination)?;
            Ok(json!({
                "scrcpy":scrcpy.display().to_string(),
                "audio":true,
                "artifact":artifact_json(&artifact)
            }))
        }
        operation => Err(AuError::code(
            "E_ARGS",
            format!("unknown screen operation {operation}"),
        )),
    }
}

fn camera(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    output: Option<&Path>,
    force: bool,
    binary: bool,
) -> Result<Value> {
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => helper::call(adb, paths, serial, "camera.list", json!({})),
        "snap" => {
            let destination = reserve_output(paths, output, "camera", "jpg", force)?;
            let data = helper::call(
                adb,
                paths,
                serial,
                "camera.snapshot",
                json!({"camera":args.get(1)}),
            )?;
            let artifact = pull_private_media(adb, paths, serial, &data, destination, 20)?;
            Ok(artifact_json(&artifact))
        }
        "record" => {
            let seconds = duration(args, 1, 3, "cam record [SECONDS]")?;
            let destination = reserve_output(paths, output, "camera-record", "mp4", force)?;
            let data = helper::call_media(
                adb,
                paths,
                serial,
                "camera.record",
                json!({"seconds":seconds,"camera":args.get(2)}),
                seconds,
            )?;
            let artifact = pull_private_media(
                adb,
                paths,
                serial,
                &data,
                destination,
                seconds.saturating_add(10),
            )?;
            Ok(artifact_json(&artifact))
        }
        "view" => {
            let duration = duration(args, 1, 30, "cam view [SECONDS]")?;
            let scrcpy = find_scrcpy()?;
            let mut command = Command::new(&scrcpy);
            command.args([
                "--serial",
                serial,
                "--video-source",
                "camera",
                "--time-limit",
                &duration.to_string(),
            ]);
            let result = run(
                &mut command,
                RunOptions {
                    deadline: Duration::from_secs(duration + 10),
                    ..RunOptions::default()
                },
            )?;
            if !result.status.success() {
                return Err(AuError::code(
                    "E_SCRCPY",
                    "camera preview exited unsuccessfully",
                ));
            }
            Ok(json!({"seconds":duration,"finished":true}))
        }
        "pipe" => {
            if !binary {
                return Err(AuError::code(
                    "E_BINARY",
                    "cam pipe requires --binary and must not be sent to chat",
                ));
            }
            let seconds = duration(args, 1, 3, "cam pipe [SECONDS] [CAMERA]")?;
            let destination = reserve_output(paths, output, "camera-stream", "mjpeg", force)?;
            let data = helper::call_media(
                adb,
                paths,
                serial,
                "camera.mjpeg",
                json!({"seconds":seconds,"camera":args.get(2)}),
                seconds,
            )?;
            let artifact = pull_private_media(
                adb,
                paths,
                serial,
                &data,
                destination,
                seconds.saturating_add(10),
            )?;
            Ok(json!({
                "binary_path":artifact.path,
                "bytes":artifact.bytes,
                "sha256":artifact.sha256,
                "format":data.get("format"),
                "frames":data.get("frames"),
                "boundary":data.get("boundary")
            }))
        }
        operation => Err(AuError::code(
            "E_ARGS",
            format!("unknown cam operation {operation}"),
        )),
    }
}

fn microphone(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    output: Option<&Path>,
    force: bool,
    binary: bool,
) -> Result<Value> {
    match args.first().map(String::as_str).unwrap_or("cap") {
        "cap" => {
            let seconds = duration(args, 1, 3, "mic cap [SECONDS]")?;
            let destination = reserve_output(paths, output, "microphone", "wav", force)?;
            let data = helper::call_media(
                adb,
                paths,
                serial,
                "microphone.capture",
                json!({"seconds":seconds,"format":"wav"}),
                seconds,
            )?;
            let artifact = pull_private_media(
                adb,
                paths,
                serial,
                &data,
                destination,
                seconds.saturating_add(10),
            )?;
            Ok(artifact_json(&artifact))
        }
        "pipe" => {
            if !binary {
                return Err(AuError::code(
                    "E_BINARY",
                    "mic pipe requires --binary and must not be sent to chat",
                ));
            }
            let seconds = duration(args, 1, 3, "mic pipe [SECONDS]")?;
            let destination = reserve_output(paths, output, "microphone-stream", "pcm", force)?;
            let data = helper::call_media(
                adb,
                paths,
                serial,
                "microphone.pcm",
                json!({"seconds":seconds,"format":"pcm_s16le"}),
                seconds,
            )?;
            let artifact = pull_private_media(
                adb,
                paths,
                serial,
                &data,
                destination,
                seconds.saturating_add(10),
            )?;
            Ok(json!({
                "binary_path":artifact.path,
                "bytes":artifact.bytes,
                "sha256":artifact.sha256,
                "sample_rate":data.get("sample_rate"),
                "channels":data.get("channels"),
                "sample_format":data.get("sample_format")
            }))
        }
        operation => Err(AuError::code(
            "E_ARGS",
            format!("unknown mic operation {operation}"),
        )),
    }
}

fn pull_private_media(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    data: &Value,
    destination: PathBuf,
    timeout_seconds: u64,
) -> Result<Artifact> {
    let file = data.get("file").and_then(Value::as_str).ok_or_else(|| {
        AuError::code(
            "E_MEDIA",
            "helper response did not include private media file",
        )
    })?;
    if file.contains("..")
        || file.starts_with('/')
        || !file.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | '_' | '-' | '.')
        })
    {
        return Err(AuError::code(
            "E_MEDIA",
            "helper returned invalid media path",
        ));
    }
    const CHUNK_BYTES: u64 = 256 * 1024;
    const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
    let expected = data.get("bytes").and_then(Value::as_u64).ok_or_else(|| {
        AuError::code(
            "E_MEDIA",
            "helper response did not include private media size",
        )
    })?;
    if expected == 0 || expected > MAX_ARTIFACT_BYTES {
        return Err(AuError::code(
            "E_OUTPUT_LIMIT",
            "private media size is zero or exceeds the 512 MiB artifact limit",
        ));
    }

    let mut session = helper::HelperSession::open(adb, paths, serial)?;
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(8));
    let opened = session.call_with_timeout(
        "artifact.open",
        json!({"file":file}),
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(30)),
    )?;
    let handle = opened.get("handle").and_then(Value::as_str).unwrap_or("");
    let opened_file = opened.get("file").and_then(Value::as_str).unwrap_or("");
    let opened_bytes = opened.get("total_bytes").and_then(Value::as_u64);
    let expected_sha256 = opened.get("sha256").and_then(Value::as_str).unwrap_or("");
    if handle.len() != 32
        || !handle.bytes().all(|byte| byte.is_ascii_hexdigit())
        || opened_file != file
        || opened_bytes != Some(expected)
        || expected_sha256.len() != 64
        || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AuError::code(
            "E_PROTOCOL",
            "helper returned invalid private artifact snapshot metadata",
        ));
    }
    let transfer = (|| -> Result<Artifact> {
        use sha2::{Digest, Sha256};

        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&destination)?;
        let mut offset = 0u64;
        let mut hasher = Sha256::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AuError::code(
                    "E_TIMEOUT",
                    "private media transfer timed out",
                ));
            }
            let response = session.call_with_timeout(
                "artifact.read",
                json!({"handle":handle,"file":file,"offset":offset,"length":CHUNK_BYTES}),
                remaining.min(Duration::from_secs(5)),
            )?;
            let response_file = response.get("file").and_then(Value::as_str).unwrap_or("");
            let response_handle = response.get("handle").and_then(Value::as_str).unwrap_or("");
            let response_offset = response.get("offset").and_then(Value::as_u64);
            let next = response.get("next_offset").and_then(Value::as_u64);
            let chunk_bytes = response.get("bytes").and_then(Value::as_u64);
            let total = response.get("total_bytes").and_then(Value::as_u64);
            let encoded = response.get("data").and_then(Value::as_str).unwrap_or("");
            let eof = response
                .get("eof")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if response_file != file
                || response_handle != handle
                || response_offset != Some(offset)
                || total != Some(expected)
                || next.is_none()
                || chunk_bytes.is_none()
            {
                return Err(AuError::code(
                    "E_PROTOCOL",
                    "helper returned inconsistent private artifact metadata",
                ));
            }
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| {
                    AuError::code("E_PROTOCOL", "helper returned invalid artifact data")
                })?;
            let next = next.unwrap_or(offset);
            if decoded.len() as u64 != chunk_bytes.unwrap_or(u64::MAX)
                || decoded.len() as u64 > CHUNK_BYTES
                || next != offset.saturating_add(decoded.len() as u64)
                || next > expected
                || (!eof && decoded.is_empty())
                || (eof && next != expected)
            {
                return Err(AuError::code(
                    "E_PROTOCOL",
                    "helper returned an invalid private artifact chunk",
                ));
            }
            output.write_all(&decoded)?;
            hasher.update(&decoded);
            offset = next;
            if eof {
                output.flush()?;
                output.sync_all()?;
                break;
            }
        }
        let actual_sha256 = format!("{:x}", hasher.finalize());
        if actual_sha256 != expected_sha256 {
            return Err(AuError::code(
                "E_HASH",
                "private artifact digest did not match the immutable device snapshot",
            ));
        }
        Ok(Artifact {
            path: destination.display().to_string(),
            bytes: expected,
            sha256: actual_sha256,
        })
    })();

    let cleanup = session
        .call("artifact.delete", json!({"handle":handle,"file":file}))
        .and_then(|value| {
            if value.get("removed").and_then(Value::as_bool) == Some(true) {
                Ok(())
            } else {
                Err(AuError::code(
                    "E_ARTIFACT",
                    "helper did not remove private media",
                ))
            }
        });
    match (transfer, cleanup) {
        (Ok(artifact), Ok(())) => Ok(artifact),
        (Err(error), _) => {
            let _ = fs::remove_file(&destination);
            Err(error)
        }
        (Ok(_), Err(error)) => {
            let _ = fs::remove_file(&destination);
            Err(error)
        }
    }
}

fn local_artifact(path: PathBuf) -> Result<Artifact> {
    use sha2::{Digest, Sha256};
    let bytes = fs::metadata(&path)?.len();
    let mut stream = fs::File::open(&path)?;
    let mut buffer = [0u8; 16 * 1024];
    let mut hasher = Sha256::new();
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Artifact {
        path: path.display().to_string(),
        bytes,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn artifact_json(artifact: &Artifact) -> Value {
    json!({"path":artifact.path,"bytes":artifact.bytes,"sha256":artifact.sha256})
}

fn duration(args: &[String], index: usize, default: u64, usage: &str) -> Result<u64> {
    let seconds = match args.get(index) {
        None => default,
        Some(value) => value.parse().map_err(|_| AuError::code("E_ARGS", usage))?,
    };
    if !(1..=180).contains(&seconds) {
        return Err(AuError::code("E_ARGS", "duration must be 1..180 seconds"));
    }
    Ok(seconds)
}

fn find_scrcpy() -> Result<PathBuf> {
    let executable = if cfg!(windows) {
        "scrcpy.exe"
    } else {
        "scrcpy"
    };
    let mut candidates = vec![env::var_os("AU_INSTALL_ROOT").map(|root| {
        PathBuf::from(root)
            .join("tools")
            .join("scrcpy")
            .join(executable)
    })];
    #[cfg(windows)]
    candidates.push(env::var_os("LOCALAPPDATA").map(|root| {
        PathBuf::from(root).join("Codex/android-use/tools/scrcpy/scrcpy-win64-v4.1/scrcpy.exe")
    }));
    candidates.push(env::var_os("PATH").and_then(|value| {
        env::split_paths(&value)
            .map(|path| path.join(executable))
            .find(|path| path.is_file())
    }));
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
        .ok_or_else(|| AuError::code("E_SCRCPY", "scrcpy 4.1 is not installed or on PATH"))
}

#[cfg(test)]
mod tests {
    use super::duration;

    #[test]
    fn recording_duration_is_finite() {
        let args = vec!["record".into(), "3".into()];
        assert_eq!(duration(&args, 1, 1, "test").expect("duration"), 3);
    }
}
