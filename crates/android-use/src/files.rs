use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::adb::Adb;
use crate::config::AppPaths;
use crate::error::{AuError, Result};
use crate::process::{text, CaptureDestination, ProcessResult};

#[derive(Clone, Debug, Serialize)]
pub struct Artifact {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

pub fn reserve_output(
    paths: &AppPaths,
    requested: Option<&Path>,
    stem: &str,
    extension: &str,
    force: bool,
) -> Result<PathBuf> {
    let path = match requested {
        Some(path) => path.to_path_buf(),
        None => {
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| AuError::code("E_TIME", "system clock before epoch"))?
                .as_millis();
            paths.artifacts.join(format!("{stem}-{millis}.{extension}"))
        }
    };
    if path.exists() {
        if force {
            if path.is_dir() {
                return Err(AuError::code(
                    "E_PATH",
                    format!("refusing to overwrite directory {}", path.display()),
                ));
            }
            fs::remove_file(&path)?;
        } else {
            return Err(AuError::code(
                "E_EXISTS",
                format!(
                    "{} exists; choose another path or pass --force",
                    path.display()
                ),
            ));
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(path)
}

pub fn artifact_from_process(result: &ProcessResult) -> Result<Artifact> {
    let path = result.stdout.path.as_ref().ok_or_else(|| {
        AuError::code("E_ARTIFACT", "command did not stream stdout to an artifact")
    })?;
    Ok(Artifact {
        path: path.display().to_string(),
        bytes: result.stdout.total_bytes,
        sha256: result.stdout.sha256.clone(),
    })
}

pub fn push(adb: &Adb, serial: &str, local: &Path, remote: &str) -> Result<()> {
    if !local.is_file() {
        return Err(AuError::code(
            "E_PATH",
            format!("local file {} does not exist", local.display()),
        ));
    }
    adb.device(
        serial,
        &["push".into(), local.display().to_string(), remote.into()],
    )?;
    Ok(())
}

pub fn pull(adb: &Adb, serial: &str, remote: &str, local: &Path, force: bool) -> Result<()> {
    if local.exists() {
        if force {
            fs::remove_file(local)?;
        } else {
            return Err(AuError::code(
                "E_EXISTS",
                format!("{} exists; pass --force", local.display()),
            ));
        }
    }
    if let Some(parent) = local.parent() {
        fs::create_dir_all(parent)?;
    }
    adb.device(
        serial,
        &["pull".into(), remote.into(), local.display().to_string()],
    )?;
    Ok(())
}

pub fn list(adb: &Adb, serial: &str, remote: &str) -> Result<String> {
    let result = adb.device(
        serial,
        &["shell".into(), "ls".into(), "-la".into(), remote.into()],
    )?;
    Ok(text(&result.stdout))
}

pub fn screenshot(adb: &Adb, serial: &str, path: PathBuf) -> Result<Artifact> {
    let result = adb.device_to_file(
        serial,
        &["exec-out".into(), "screencap".into(), "-p".into()],
        path,
    )?;
    artifact_from_process(&result)
}

pub fn dump_to_file(
    adb: &Adb,
    serial: &str,
    command: &[String],
    path: PathBuf,
) -> Result<Artifact> {
    let result = adb.device_to_file(serial, command, path)?;
    artifact_from_process(&result)
}

pub fn capture_destination_for_binary(path: PathBuf) -> CaptureDestination {
    CaptureDestination::File(path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::reserve_output;
    use crate::config::AppPaths;

    fn paths(root: &std::path::Path) -> AppPaths {
        AppPaths {
            root: root.to_path_buf(),
            config: root.join("config.json"),
            state: root.join("state"),
            artifacts: root.join("artifacts"),
            daemon: root.join("state/daemon.json"),
            forwards: root.join("state/forwards.json"),
            location_journal: root.join("state/location-journal.json"),
        }
    }

    #[test]
    fn explicit_output_does_not_clobber_by_default() {
        let root = tempfile::tempdir().expect("temp");
        let path = root.path().join("existing.png");
        fs::write(&path, b"existing").expect("write");
        let error = reserve_output(&paths(root.path()), Some(&path), "shot", "png", false)
            .expect_err("exists");
        assert_eq!(error.kind(), "E_EXISTS");
    }
}
