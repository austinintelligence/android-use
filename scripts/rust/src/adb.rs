use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::error::{AuError, Result};
use crate::process::{run, text, CaptureDestination, ProcessResult, RunOptions};

#[derive(Clone, Debug)]
pub struct Adb {
    path: PathBuf,
    timeout: Duration,
}

impl Adb {
    pub fn from_config(config: &Config, timeout_ms: u64) -> Result<Self> {
        let path = locate_adb(config)?;
        Ok(Self {
            path,
            timeout: Duration::from_millis(timeout_ms),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn with_timeout(&self, timeout: Duration) -> Self {
        Self {
            path: self.path.clone(),
            timeout,
        }
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

pub fn shell_script_args(script: &str) -> Vec<String> {
    vec!["shell".into(), script.to_owned()]
}

pub fn locate_adb(config: &Config) -> Result<PathBuf> {
    let candidates = [
        config.adb_path.clone(),
        env::var_os("LOCALAPPDATA").map(|root| {
            PathBuf::from(root)
                .join("Codex")
                .join("android-use")
                .join("platform-tools")
                .join("adb.exe")
        }),
        env::var_os("LOCALAPPDATA").map(|root| {
            PathBuf::from(root)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join("adb.exe")
        }),
        env::var_os("LOCALAPPDATA").map(|root| {
            PathBuf::from(root)
                .join("Codex")
                .join("android-agent-display")
                .join("platform-tools")
                .join("adb.exe")
        }),
    ];
    if let Some(path) = candidates.into_iter().flatten().find(|path| path.is_file()) {
        return Ok(path);
    }
    if let Some(path) = find_on_path("adb.exe") {
        return Ok(path);
    }
    Err(AuError::code(
        "E_ADB",
        "adb.exe not found; install Android platform-tools or configure adb_path",
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
    use super::{fixed_shell_command, shell_quote, shell_script_args};

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
}
