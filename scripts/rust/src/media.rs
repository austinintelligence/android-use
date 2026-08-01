use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};

use crate::adb::Adb;
use crate::config::AppPaths;
use crate::error::{AuError, Result};
use crate::files::{artifact_from_process, reserve_output, screenshot, Artifact};
use crate::helper::{self, HELPER_PACKAGE};
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
            let artifact = pull_private_media(adb, serial, &data, destination, 20)?;
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
            let artifact =
                pull_private_media(adb, serial, &data, destination, seconds.saturating_add(10))?;
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
            let artifact =
                pull_private_media(adb, serial, &data, destination, seconds.saturating_add(10))?;
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
            let artifact =
                pull_private_media(adb, serial, &data, destination, seconds.saturating_add(10))?;
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
            let artifact =
                pull_private_media(adb, serial, &data, destination, seconds.saturating_add(10))?;
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
    let media_adb = adb.with_timeout(Duration::from_secs(timeout_seconds.max(8)));
    let result = media_adb.device_to_file(
        serial,
        &[
            "exec-out".into(),
            "run-as".into(),
            HELPER_PACKAGE.into(),
            "cat".into(),
            format!("files/{file}"),
        ],
        destination,
    );
    let cleanup = media_adb.device(
        serial,
        &[
            "shell".into(),
            "run-as".into(),
            HELPER_PACKAGE.into(),
            "rm".into(),
            "-f".into(),
            format!("files/{file}"),
        ],
    );
    match result {
        Ok(result) => {
            cleanup?;
            artifact_from_process(&result)
        }
        Err(error) => {
            let _ = cleanup;
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
    let candidates = [
        env::var_os("LOCALAPPDATA").map(|root| {
            PathBuf::from(root).join("Codex/android-use/tools/scrcpy/scrcpy-win64-v4.1/scrcpy.exe")
        }),
        env::var_os("PATH").and_then(|value| {
            env::split_paths(&value)
                .map(|path| path.join("scrcpy.exe"))
                .find(|path| path.is_file())
        }),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
        .ok_or_else(|| AuError::code("E_SCRCPY", "scrcpy 4.1 is not installed"))
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
