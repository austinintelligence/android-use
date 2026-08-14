use crate::api::{Code, Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_ADB_OUTPUT: u64 = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub device: PathBuf,
    pub journal: PathBuf,
    pub artifacts: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let root = if let Some(v) = env::var_os("AU_HOME") {
            PathBuf::from(v)
        } else if cfg!(windows) {
            PathBuf::from(env::var_os("LOCALAPPDATA").ok_or_else(|| Error::new(Code::Io, "LOCALAPPDATA is not set"))?).join("AndroidUse")
        } else {
            PathBuf::from(
                env::var_os("XDG_STATE_HOME")
                    .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state").into_os_string()))
                    .ok_or_else(|| Error::new(Code::Io, "no state directory"))?,
            )
            .join("android-use")
        };
        Self::at(root)
    }
    pub(crate) fn at(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        let artifacts = root.join("artifacts");
        fs::create_dir_all(&artifacts)?;
        Ok(Self { device: root.join("device.json"), journal: root.join("operations.jsonl"), artifacts, root })
    }
}

#[derive(Debug, Clone)]
pub struct Adb {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Binding {
    hardware: String,
    endpoint: String,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub endpoint: Box<str>,
    pub hardware: Box<str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdbDevice {
    pub endpoint: Box<str>,
    pub state: Box<str>,
}

impl Adb {
    pub fn discover() -> Result<Self> {
        if let Some(v) = env::var_os("AU_ADB") {
            let p = PathBuf::from(v);
            if p.is_file() {
                return Ok(Self { path: p });
            }
        }
        if let Some(v) = env::var_os("ANDROID_SDK_ROOT").or_else(|| env::var_os("ANDROID_HOME")) {
            let p = PathBuf::from(v).join("platform-tools").join(if cfg!(windows) { "adb.exe" } else { "adb" });
            if p.is_file() {
                return Ok(Self { path: p });
            }
        }
        if cfg!(windows) {
            if let Some(v) = env::var_os("LOCALAPPDATA") {
                let p = PathBuf::from(v).join("Android/SDK/platform-tools/adb.exe");
                if p.is_file() {
                    return Ok(Self { path: p });
                }
            }
        }
        if run_bounded(Path::new(if cfg!(windows) { "adb.exe" } else { "adb" }), &["version"], Duration::from_secs(3)).is_ok() {
            return Ok(Self { path: PathBuf::from(if cfg!(windows) { "adb.exe" } else { "adb" }) });
        }
        Err(Error::new(Code::Device, "adb was not found; set AU_ADB or ANDROID_SDK_ROOT"))
    }
    pub(crate) fn run(&self, endpoint: Option<&str>, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
        let mut all = Vec::with_capacity(args.len() + 2);
        if let Some(s) = endpoint {
            all.extend(["-s", s]);
        }
        all.extend_from_slice(args);
        run_bounded(&self.path, &all, timeout)
    }
    pub fn devices(&self) -> Result<Vec<Box<str>>> {
        Ok(self.devices_all()?.into_iter().filter(|d| d.state.as_ref() == "device").map(|d| d.endpoint).collect())
    }
    pub fn devices_all(&self) -> Result<Vec<AdbDevice>> {
        let out = self.run(None, &["devices"], Duration::from_secs(5))?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .skip(1)
            .filter_map(|line| {
                let mut p = line.split_whitespace();
                let endpoint = p.next()?;
                let state = p.next()?;
                if endpoint.len() > 256 || state.len() > 32 {
                    return None;
                }
                Some(AdbDevice { endpoint: endpoint.into(), state: state.into() })
            })
            .collect())
    }
    pub fn enroll(&self, paths: &Paths, endpoint: &str) -> Result<Device> {
        let hardware = self.hardware(endpoint)?;
        let binding = Binding { hardware: hardware.to_string(), endpoint: endpoint.into() };
        atomic(&paths.device, &serde_json::to_vec(&binding).map_err(|e| Error::new(Code::Io, e.to_string()))?)?;
        Ok(Device { endpoint: endpoint.into(), hardware })
    }
    pub fn resolve(&self, paths: &Paths) -> Result<Device> {
        let bytes = fs::read(&paths.device).map_err(|_| Error::new(Code::Identity, "no device is enrolled; run au enroll ENDPOINT"))?;
        let binding: Binding = serde_json::from_slice(&bytes).map_err(|_| Error::new(Code::Identity, "device binding is corrupt"))?;
        let mut devices = self.devices()?;
        devices.sort_by_key(|s| {
            if !s.contains(':') {
                0
            } else if s.as_ref() == binding.endpoint {
                1
            } else {
                2
            }
        });
        for endpoint in devices {
            if let Ok(hardware) = self.hardware(&endpoint) {
                if hardware.as_ref() == binding.hardware {
                    return Ok(Device { endpoint, hardware });
                }
            }
        }
        Err(Error::new(Code::Identity, "no connected transport matches the enrolled hardware serial"))
    }
    pub fn resolve_or_enroll(&self, paths: &Paths) -> Result<(Device, bool)> {
        if paths.device.is_file() {
            return Ok((self.resolve(paths)?, false));
        }
        let ready: Vec<_> = self.devices_all()?.into_iter().filter(|d| d.state.as_ref() == "device").collect();
        let one = match ready.as_slice() {
            [one] => one,
            [] => return Err(Error::new(Code::Device, "connect and unlock one Android device, then run au setup again")),
            _ => return Err(Error::new(Code::Ambiguous, "more than one Android device is connected; use au enroll ENDPOINT")),
        };
        Ok((self.enroll(paths, &one.endpoint)?, true))
    }
    pub fn hardware(&self, endpoint: &str) -> Result<Box<str>> {
        for key in ["ro.serialno", "ro.boot.serialno"] {
            let out = self.run(Some(endpoint), &["shell", "getprop", key], Duration::from_secs(4))?;
            let s = String::from_utf8_lossy(&out).trim().to_string();
            if !s.is_empty() && s.len() <= 128 {
                return Ok(s.into_boxed_str());
            }
        }
        Err(Error::new(Code::Identity, "device did not expose a hardware serial"))
    }
    pub fn package_installed(&self, d: &Device, package: &str) -> Result<bool> {
        let out = self.run(Some(&d.endpoint), &["shell", "pm", "path", package], Duration::from_secs(5))?;
        Ok(String::from_utf8_lossy(&out).lines().any(|line| line.starts_with("package:")))
    }
    pub fn secure_setting_contains(&self, d: &Device, key: &str, needle: &str) -> Result<bool> {
        let out = self.run(Some(&d.endpoint), &["shell", "settings", "get", "secure", key], Duration::from_secs(5))?;
        Ok(String::from_utf8_lossy(&out).contains(needle))
    }
    pub fn accessibility_enabled(&self, d: &Device) -> Result<bool> {
        self.secure_setting_contains(d, "enabled_accessibility_services", "dev.codex.aubridge")
    }
    pub fn open_accessibility_settings(&self, d: &Device) -> Result<()> {
        self.run(Some(&d.endpoint), &["shell", "am", "start", "-a", "android.settings.ACCESSIBILITY_SETTINGS"], Duration::from_secs(8))?;
        Ok(())
    }
    pub fn wait_for_accessibility(&self, d: &Device, timeout: Duration) -> Result<bool> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if self.accessibility_enabled(d).unwrap_or(false) {
                return Ok(true);
            }
            thread::sleep(Duration::from_millis(500));
        }
        Ok(false)
    }
    pub fn notifications_enabled(&self, d: &Device) -> Result<bool> {
        self.secure_setting_contains(d, "enabled_notification_listeners", "dev.codex.aubridge")
    }
    pub fn browser_installed(&self, d: &Device) -> Result<bool> {
        self.package_installed(d, "com.android.chrome")
    }
    pub(crate) fn start_helper(&self, d: &Device) -> Result<()> {
        self.run(Some(&d.endpoint), &["shell", "am", "start-foreground-service", "-n", "dev.codex.aubridge/.BridgeService"], Duration::from_secs(8))?;
        Ok(())
    }
    pub(crate) fn forward(&self, d: &Device, remote: &str) -> Result<u16> {
        if !matches!(remote, "localabstract:aubridge-v3" | "localabstract:aubridge-bootstrap-v3" | "localabstract:chrome_devtools_remote") {
            return Err(Error::new(Code::Protocol, "refusing unknown helper socket"));
        }
        let out = self.run(Some(&d.endpoint), &["forward", "tcp:0", remote], Duration::from_secs(5))?;
        String::from_utf8_lossy(&out).trim().parse::<u16>().map_err(|_| Error::new(Code::Helper, "adb did not allocate a forward"))
    }
    pub(crate) fn remove_forward(&self, d: &Device, port: u16) {
        let local = format!("tcp:{port}");
        let _ = self.run(Some(&d.endpoint), &["forward", "--remove", &local], Duration::from_secs(3));
    }
    pub fn install(&self, d: &Device, apk: &Path) -> Result<()> {
        let p = apk.to_str().ok_or_else(|| Error::new(Code::Args, "APK path is not UTF-8"))?;
        self.run(Some(&d.endpoint), &["install", "-r", "-d", "-t", p], Duration::from_secs(120))?;
        Ok(())
    }
    pub fn uninstall_helper(&self, d: &Device) -> Result<()> {
        self.run(Some(&d.endpoint), &["uninstall", "dev.codex.aubridge"], Duration::from_secs(30))?;
        Ok(())
    }
}

fn run_bounded(program: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::new(Code::Device, format!("failed to start adb: {e}")))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut b = Vec::new();
        stdout.take(MAX_ADB_OUTPUT + 1).read_to_end(&mut b).map(|_| b)
    });
    let err = thread::spawn(move || {
        let mut b = Vec::new();
        stderr.take(MAX_ADB_OUTPUT + 1).read_to_end(&mut b).map(|_| b)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err(Error::new(Code::Timeout, "adb command timed out"));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = out.join().map_err(|_| Error::new(Code::Io, "adb stdout reader failed"))??;
    let stderr = err.join().map_err(|_| Error::new(Code::Io, "adb stderr reader failed"))??;
    if stdout.len() as u64 > MAX_ADB_OUTPUT || stderr.len() as u64 > MAX_ADB_OUTPUT {
        return Err(Error::new(Code::Bounds, "adb output exceeded 256 KiB"));
    }
    if !status.success() {
        return Err(Error::new(Code::Device, format!("adb command failed with {status}")));
    }
    Ok(stdout)
}

pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return Err(Error::new(Code::Io, "refusing to replace a symbolic link"));
    }
    let suffix = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let tmp = path.with_extension(format!("tmp-{suffix}"));
    let backup = path.with_extension(format!("bak-{suffix}"));
    {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        std::io::Write::write_all(&mut f, bytes)?;
        f.sync_all()?;
    }
    if path.exists() {
        fs::rename(path, &backup)?;
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::rename(&backup, path);
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        fs::remove_file(backup)?;
    } else {
        fs::rename(tmp, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_scoped() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::at(d.path().to_path_buf()).unwrap();
        assert!(p.root.starts_with(d.path()));
    }
}
