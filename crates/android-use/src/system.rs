use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::adb::Adb;
use crate::config::{atomic_write, AppPaths};
use crate::error::{AuError, Result};
use crate::files::{self, reserve_output, Artifact};
use crate::helper;
use crate::process::text;

pub fn execute(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    hardware_identity: &str,
    command: &str,
    args: &[String],
    options: SystemOptions<'_>,
) -> Result<Value> {
    match command {
        "clip" => clip(adb, serial, args),
        "notif" => notification(adb, paths, serial, args),
        "file" => file(adb, paths, serial, args, options),
        "prop" => property(adb, serial, args),
        "settings" => settings(adb, serial, args),
        "sys" => dumpsys(adb, serial, args),
        "log" => log(adb, serial, args),
        "ps" => processes(adb, serial),
        "fwd" => forward(adb, paths, serial, hardware_identity, args),
        "rev" => reverse(adb, paths, serial, hardware_identity, args),
        _ => Err(AuError::code(
            "E_ARGS",
            format!("unknown system command {command}"),
        )),
    }
}

#[derive(Clone, Copy)]
pub struct SystemOptions<'a> {
    pub output: Option<&'a Path>,
    pub force: bool,
}

fn clip(adb: &Adb, serial: &str, args: &[String]) -> Result<Value> {
    match args.first().map(String::as_str).unwrap_or("get") {
        "get" => {
            let result = adb.device(
                serial,
                &[
                    "shell".into(),
                    "cmd".into(),
                    "clipboard".into(),
                    "get".into(),
                ],
            )?;
            Ok(json!({"text":limit(text(&result.stdout), 16000)}))
        }
        "set" => {
            let value = required(args, 1, "clip set TEXT")?;
            adb.device(
                serial,
                &[
                    "shell".into(),
                    "cmd".into(),
                    "clipboard".into(),
                    "set".into(),
                    value.into(),
                ],
            )?;
            Ok(json!({"set":true}))
        }
        operation => Err(AuError::code(
            "E_ARGS",
            format!("unknown clip operation {operation}"),
        )),
    }
}

fn notification(adb: &Adb, paths: &AppPaths, serial: &str, args: &[String]) -> Result<Value> {
    let operation = args.first().map(String::as_str).unwrap_or("ls");
    helper::call(
        adb,
        paths,
        serial,
        &format!("notification.{operation}"),
        json!({"args":&args[1..]}),
    )
}

fn file(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    options: SystemOptions<'_>,
) -> Result<Value> {
    match args.first().map(String::as_str).unwrap_or("ls") {
        "ls" => {
            let remote = required(args, 1, "file ls REMOTE")?;
            let result = adb.device(
                serial,
                &["shell".into(), "ls".into(), "-la".into(), remote.into()],
            )?;
            Ok(json!({"path":remote,"text":text(&result.stdout)}))
        }
        "mkdir" => {
            let remote = required(args, 1, "file mkdir REMOTE")?;
            adb.device(
                serial,
                &["shell".into(), "mkdir".into(), "-p".into(), remote.into()],
            )?;
            Ok(json!({"path":remote,"created":true}))
        }
        "rm" => {
            let remote = required(args, 1, "file rm REMOTE")?;
            adb.device(
                serial,
                &["shell".into(), "rm".into(), "-f".into(), remote.into()],
            )?;
            Ok(json!({"path":remote,"removed":true}))
        }
        "push" => {
            let local = PathBuf::from(required(args, 1, "file push LOCAL REMOTE")?);
            let remote = required(args, 2, "file push LOCAL REMOTE")?;
            files::push(adb, serial, &local, remote)?;
            Ok(json!({"local":local,"remote":remote,"pushed":true}))
        }
        "pull" => {
            let remote = required(args, 1, "file pull REMOTE [LOCAL]")?;
            let destination = match args.get(2) {
                Some(path) => PathBuf::from(path),
                None => reserve_output(paths, options.output, "pull", "bin", options.force)?,
            };
            files::pull(adb, serial, remote, &destination, options.force)?;
            let artifact = artifact(&destination)?;
            Ok(
                json!({"path":artifact.path,"bytes":artifact.bytes,"sha256":artifact.sha256,"remote":remote}),
            )
        }
        "cat" => {
            let remote = required(args, 1, "file cat REMOTE")?;
            if let Some(requested) = options.output {
                let destination =
                    reserve_output(paths, Some(requested), "file", "bin", options.force)?;
                let artifact = files::dump_to_file(
                    adb,
                    serial,
                    &["exec-out".into(), "cat".into(), remote.into()],
                    destination,
                )?;
                Ok(
                    json!({"path":artifact.path,"bytes":artifact.bytes,"sha256":artifact.sha256,"remote":remote}),
                )
            } else {
                let result = adb.device(serial, &["shell".into(), "cat".into(), remote.into()])?;
                Ok(
                    json!({"path":remote,"text":limit(text(&result.stdout), 16_000),"truncated":result.stdout.truncated}),
                )
            }
        }
        operation => Err(AuError::code(
            "E_ARGS",
            format!("unknown file operation {operation}"),
        )),
    }
}

fn artifact(path: &Path) -> Result<Artifact> {
    use sha2::{Digest, Sha256};
    let bytes = fs::metadata(path)?.len();
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 16 * 1024];
    let mut hasher = Sha256::new();
    loop {
        let count = file.read(&mut buffer)?;
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

fn property(adb: &Adb, serial: &str, args: &[String]) -> Result<Value> {
    let name = args.first().map(String::as_str).unwrap_or("");
    let command = if name.is_empty() {
        vec!["shell".into(), "getprop".into()]
    } else {
        vec!["shell".into(), "getprop".into(), name.into()]
    };
    let result = adb.device(serial, &command)?;
    Ok(json!({"property":name,"text":text(&result.stdout)}))
}

fn settings(adb: &Adb, serial: &str, args: &[String]) -> Result<Value> {
    if args.len() < 3 && args.first().map(String::as_str) != Some("list") {
        return Err(AuError::code(
            "E_ARGS",
            "settings get|put NAMESPACE KEY [VALUE], or settings list NAMESPACE",
        ));
    }
    let mut command = vec!["shell".into(), "settings".into()];
    command.extend(args.iter().cloned());
    let result = adb.device(serial, &command)?;
    Ok(json!({"text":text(&result.stdout)}))
}

fn dumpsys(adb: &Adb, serial: &str, args: &[String]) -> Result<Value> {
    let service = required(args, 0, "sys SERVICE [ARGS…]")?;
    let mut command = vec!["shell".into(), "dumpsys".into(), service.into()];
    command.extend(args.iter().skip(1).cloned());
    let result = adb.device(serial, &command)?;
    Ok(json!({"service":service,"text":limit(text(&result.stdout), 16000)}))
}

fn log(adb: &Adb, serial: &str, args: &[String]) -> Result<Value> {
    let mut command = vec!["logcat".into(), "-d".into(), "-t".into(), "500".into()];
    command.extend(args.iter().cloned());
    let result = adb.device(serial, &command)?;
    Ok(json!({"text":limit(text(&result.stdout), 16000)}))
}

fn processes(adb: &Adb, serial: &str) -> Result<Value> {
    let result = adb.device(serial, &["shell".into(), "ps".into(), "-A".into()])?;
    Ok(json!({"text":limit(text(&result.stdout), 16000)}))
}

fn forward(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    hardware_identity: &str,
    args: &[String],
) -> Result<Value> {
    match args.first().map(String::as_str).unwrap_or("ls") {
        "ls" => {
            let result = adb.device(serial, &["forward".into(), "--list".into()])?;
            Ok(json!({"text":limit(text(&result.stdout), 16000)}))
        }
        "add" => {
            let local = required(args, 1, "fwd add LOCAL REMOTE")?;
            let remote = required(args, 2, "fwd add LOCAL REMOTE")?;
            adb.device(serial, &["forward".into(), local.into(), remote.into()])?;
            if let Err(error) = record_mapping(paths, hardware_identity, "forward", local, remote) {
                let _ = adb.device(serial, &["forward".into(), "--remove".into(), local.into()]);
                return Err(error);
            }
            Ok(json!({"local":local,"remote":remote,"owned":true}))
        }
        "rm" => {
            let local = required(args, 1, "fwd rm LOCAL")?;
            remove_mapping(adb, paths, serial, hardware_identity, "forward", local)?;
            Ok(json!({"local":local,"removed":true}))
        }
        operation => Err(AuError::code(
            "E_ARGS",
            format!("unknown fwd operation {operation}"),
        )),
    }
}

fn reverse(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    hardware_identity: &str,
    args: &[String],
) -> Result<Value> {
    match args.first().map(String::as_str).unwrap_or("ls") {
        "ls" => {
            let result = adb.device(serial, &["reverse".into(), "--list".into()])?;
            Ok(json!({"text":limit(text(&result.stdout), 16000)}))
        }
        "add" => {
            let remote = required(args, 1, "rev add REMOTE LOCAL")?;
            let local = required(args, 2, "rev add REMOTE LOCAL")?;
            adb.device(serial, &["reverse".into(), remote.into(), local.into()])?;
            if let Err(error) = record_mapping(paths, hardware_identity, "reverse", remote, local) {
                let _ = adb.device(
                    serial,
                    &["reverse".into(), "--remove".into(), remote.into()],
                );
                return Err(error);
            }
            Ok(json!({"remote":remote,"local":local,"owned":true}))
        }
        "rm" => {
            let remote = required(args, 1, "rev rm REMOTE")?;
            remove_mapping(adb, paths, serial, hardware_identity, "reverse", remote)?;
            Ok(json!({"remote":remote,"removed":true}))
        }
        operation => Err(AuError::code(
            "E_ARGS",
            format!("unknown rev operation {operation}"),
        )),
    }
}

fn record_mapping(
    paths: &AppPaths,
    serial: &str,
    direction: &str,
    endpoint: &str,
    peer: &str,
) -> Result<()> {
    let path = mapping_path(paths);
    let mut records = load_mappings(&path)?;
    records.push(json!({"serial":serial,"direction":direction,"endpoint":endpoint,"peer":peer}));
    atomic_write(&path, &serde_json::to_vec(&records)?)
}

fn remove_mapping(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    hardware_identity: &str,
    direction: &str,
    endpoint: &str,
) -> Result<()> {
    let path = mapping_path(paths);
    let records = load_mappings(&path)?;
    let owned = records
        .iter()
        .any(|record| mapping_owned(record, hardware_identity, direction, endpoint));
    if !owned {
        return Err(AuError::code(
            "E_FORWARD",
            format!("refusing to remove untracked {direction} mapping {endpoint}"),
        ));
    }
    adb.device(
        serial,
        &[direction.into(), "--remove".into(), endpoint.into()],
    )?;
    let remaining = records
        .into_iter()
        .filter(|record| !mapping_owned(record, hardware_identity, direction, endpoint))
        .collect::<Vec<_>>();
    atomic_write(&path, &serde_json::to_vec(&remaining)?)
}

fn mapping_path(paths: &AppPaths) -> PathBuf {
    paths.state.join("mappings.json")
}

fn mapping_owned(record: &Value, hardware_identity: &str, direction: &str, endpoint: &str) -> bool {
    record.get("serial").and_then(Value::as_str) == Some(hardware_identity)
        && record.get("direction").and_then(Value::as_str) == Some(direction)
        && record.get("endpoint").and_then(Value::as_str) == Some(endpoint)
}

fn load_mappings(path: &PathBuf) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn required<'a>(args: &'a [String], index: usize, usage: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| AuError::code("E_ARGS", usage))
}

fn limit(text: String, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::mapping_owned;

    #[test]
    fn mapping_ownership_is_transport_independent() {
        let record = json!({
            "serial":"a1b2c3d4",
            "direction":"reverse",
            "endpoint":"tcp:18765",
            "peer":"tcp:18765"
        });
        assert!(mapping_owned(&record, "a1b2c3d4", "reverse", "tcp:18765"));
        assert!(!mapping_owned(
            &record,
            "adb-a1b2c3d4-cMDPBG._adb-tls-connect._tcp",
            "reverse",
            "tcp:18765"
        ));
    }
}
