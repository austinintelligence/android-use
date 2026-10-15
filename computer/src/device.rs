use crate::api::{Code, Error, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
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
    /// The state folder shared by every target (`AU_HOME`, else the platform default).
    pub fn base() -> Result<PathBuf> {
        Ok(if let Some(v) = env::var_os("AU_HOME") {
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
        })
    }
    /// State for the selected target: `<base>/targets/NAME` with `--target NAME` (or `AU_TARGET`),
    /// else the base folder itself, so nothing moves for anyone not using targets.
    pub fn discover() -> Result<Self> {
        let base = Self::base()?;
        let located = Self::locate()?;
        if located.root != base {
            private_dir(&base)?;
            private_dir(&base.join("targets"))?;
        }
        Self::at(located.root)
    }
    pub(crate) fn at(root: PathBuf) -> Result<Self> {
        let paths = Self::layout(root);
        private_dir(&paths.root)?;
        private_dir(&paths.artifacts)?;
        expire(&paths.artifacts, ARTIFACT_TTL);
        Ok(paths)
    }
    /// The same folders as `discover`, without creating any of them, for a read-only report.
    pub fn locate() -> Result<Self> {
        let base = Self::base()?;
        let root = match env::var("AU_TARGET").ok().filter(|name| !name.is_empty()) {
            Some(name) if crate::emulator::valid_name(&name) => base.join("targets").join(name),
            Some(_) => return Err(Error::new(Code::Args, "target names use lowercase letters, digits, and dashes (at most 32)")),
            None => base,
        };
        Ok(Self::layout(root))
    }
    fn layout(root: PathBuf) -> Self {
        let artifacts = root.join("artifacts");
        Self { device: root.join("device.json"), journal: root.join("operations.jsonl"), artifacts, root }
    }
    /// Takes the exclusive lock that keeps two `au serve` processes off one target; the lock is
    /// released when the returned file is dropped or the process ends, even by a crash.
    pub fn lock_for_serving(&self) -> Result<fs::File> {
        let file = private_file().create(true).truncate(false).write(true).open(self.root.join("lock"))?;
        file.try_lock().map_err(|_| Error::new(Code::Device, "another au serve is already driving this target; stop it, or use a different --target"))?;
        Ok(file)
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

/// Picks the connected transport for an enrolled device. The enrolled endpoint wins when it still
/// reports the enrolled serial. Otherwise exactly one matching transport is accepted; two or more
/// (emulators and cloned devices can share a serial) are refused rather than guessed, so an action
/// is never sent to the wrong device.
fn choose_endpoint(binding: &Binding, connected: &[(Box<str>, Box<str>)]) -> Result<Box<str>> {
    let same: Vec<_> = connected.iter().filter(|(_, hardware)| hardware.as_ref() == binding.hardware).collect();
    if let Some((endpoint, _)) = same.iter().find(|(endpoint, _)| endpoint.as_ref() == binding.endpoint) {
        return Ok(endpoint.clone());
    }
    match same.as_slice() {
        [] => Err(Error::new(Code::Identity, "no connected transport matches the enrolled hardware serial")),
        [(endpoint, _)] => Ok(endpoint.clone()),
        _ => Err(Error::new(Code::Ambiguous, "more than one connected device reports the enrolled serial; run au enroll ENDPOINT for the one to use")),
    }
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
        let mut found = Vec::new();
        for endpoint in self.devices()? {
            if let Ok(hardware) = self.hardware(&endpoint) {
                found.push((endpoint, hardware));
            }
        }
        let endpoint = choose_endpoint(&binding, &found)?;
        let hardware = binding.hardware.clone().into_boxed_str();
        Ok(Device { endpoint, hardware })
    }
    pub fn resolve_or_enroll(&self, paths: &Paths) -> Result<(Device, bool)> {
        if paths.device.is_file() {
            return Ok((self.resolve(paths)?, false));
        }
        // A target made by `au emulator create` enrolls its own running emulator.
        // The recorded endpoint can be stale (a crash or reboot leaves it behind), so the device on
        // it must still carry this target's identifier before it is bound.
        if let Some(endpoint) = crate::emulator::running_endpoint(paths) {
            let expected = fs::read_to_string(paths.root.join("instance")).unwrap_or_default();
            if expected.trim().is_empty() || *self.hardware(&endpoint)? != *format!("au-emulator-{}", expected.trim()) {
                return Err(Error::new(Code::Identity, "the device on this emulator's recorded port is not that emulator; run au emulator start again"));
            }
            return Ok((self.enroll(paths, &endpoint)?, true));
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
        // An emulator made by `au emulator` carries its own identity, because serials are not
        // guaranteed unique across instances and ports move between starts.
        if endpoint.starts_with("emulator-") {
            let out = self.run(Some(endpoint), &["shell", "settings", "get", "global", "android_use_instance"], Duration::from_secs(4)).unwrap_or_default();
            let marker = String::from_utf8_lossy(&out).trim().to_owned();
            if !marker.is_empty() && marker != "null" && marker.len() <= 64 {
                return Ok(format!("au-emulator-{marker}").into());
            }
        }
        for key in ["ro.serialno", "ro.boot.serialno"] {
            let out = self.run(Some(endpoint), &["shell", "getprop", key], Duration::from_secs(4))?;
            let s = String::from_utf8_lossy(&out).trim().to_string();
            if !s.is_empty() && s.len() <= 128 {
                return Ok(s.into_boxed_str());
            }
        }
        Err(Error::new(Code::Identity, "device did not expose a hardware serial"))
    }
    /// Waits for Android to finish booting: a device that is already in the `device` state can
    /// still be starting its services, and installing or enabling anything then fails oddly.
    pub fn wait_boot_completed(&self, endpoint: &str, timeout: Duration) -> Result<()> {
        let started = Instant::now();
        loop {
            let out = self.run(Some(endpoint), &["shell", "getprop", "sys.boot_completed"], Duration::from_secs(4)).unwrap_or_default();
            if String::from_utf8_lossy(&out).trim() == "1" {
                return Ok(());
            }
            if started.elapsed() >= timeout {
                return Err(Error::new(Code::Device, "the device is still starting; wait for it to finish booting, then run au setup again"));
            }
            thread::sleep(Duration::from_millis(1000));
        }
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
    /// `resume` turns a helper that was stopped from its notification back on; the host's
    /// own reconnects pass false, so Stop is not undone behind the user's back.
    pub(crate) fn start_helper(&self, d: &Device, resume: bool) -> Result<()> {
        let mut args = vec!["shell", "am", "start-foreground-service", "-n", "dev.codex.aubridge/.BridgeService"];
        if resume {
            args.extend(["--ez", "resume", "true"]);
        }
        self.run(Some(&d.endpoint), &args, Duration::from_secs(8))?;
        Ok(())
    }
    /// The helper's per-process key. Only the shell user can read it, so `au` can check that
    /// whatever answers on the helper socket is the helper (see `link::prove`).
    pub(crate) fn helper_key(&self, d: &Device) -> Result<Vec<u8>> {
        let out = self.run(Some(&d.endpoint), &["shell", "content", "query", "--uri", "content://dev.codex.aubridge.key"], Duration::from_secs(8))?;
        parse_key(&String::from_utf8_lossy(&out))
    }
    /// Opens a stream to an abstract socket on the device by speaking to the adb server directly
    /// (`host:transport:SERIAL`, then `localabstract:NAME`), the protocol `adb` itself uses. No
    /// `adb` process is spawned and no port forward exists to clean up.
    pub(crate) fn open_stream(&self, d: &Device, remote: &str) -> Result<TcpStream> {
        if !matches!(remote, "localabstract:aubridge-v3" | "localabstract:aubridge-bootstrap-v3" | "localabstract:chrome_devtools_remote") {
            return Err(Error::new(Code::Protocol, "refusing unknown device socket"));
        }
        let port = env::var("ANDROID_ADB_SERVER_PORT").ok().and_then(|value| value.parse().ok()).unwrap_or(5037);
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = match TcpStream::connect_timeout(&address, Duration::from_secs(1)) {
            Ok(stream) => stream,
            Err(_) => {
                // No adb server yet: starting one is what any `adb` command would do first.
                // The daemon can hold the output pipe open; the connect below is the real check.
                if let Err(error) = self.run(None, &["start-server"], Duration::from_secs(10)) {
                    if error.code != Code::Timeout {
                        return Err(error);
                    }
                }
                TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|e| Error::new(Code::Device, format!("cannot reach the adb server: {e}")))?
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        for request in [format!("host:transport:{}", d.endpoint), remote.to_owned()] {
            stream.write_all(format!("{:04x}{request}", request.len()).as_bytes())?;
            let mut status = [0u8; 4];
            stream.read_exact(&mut status)?;
            if &status != b"OKAY" {
                let mut length = [0u8; 4];
                let reason = stream.read_exact(&mut length).ok().and_then(|()| usize::from_str_radix(&String::from_utf8_lossy(&length), 16).ok()).filter(|n| *n <= 512).map(|n| {
                    let mut text = vec![0u8; n];
                    let _ = stream.read_exact(&mut text);
                    String::from_utf8_lossy(&text).into_owned()
                });
                return Err(Error::new(Code::Helper, format!("adb refused {request}: {}", reason.unwrap_or_default())));
            }
        }
        Ok(stream)
    }
    /// Installs or upgrades the helper, never downgrading (no `-d`). A release `au` installs only its
    /// own embedded helper or an APK signed by the pinned release key; `-t` is for dev builds alone.
    pub fn install(&self, d: &Device, apk: &Path, embedded: bool) -> Result<()> {
        let p = apk.to_str().ok_or_else(|| Error::new(Code::Args, "APK path is not UTF-8"))?;
        let dev = cfg!(debug_assertions) || env::var_os("AU_DEV_APK").is_some();
        if !(embedded || dev || crate::apk::trusted(&std::fs::read(apk)?)) {
            return Err(Error::new(Code::Args, "the helper APK is not signed by the Android Use release key"));
        }
        let flags: &[&str] = if dev { &["install", "-r", "-t"] } else { &["install", "-r"] };
        self.run(Some(&d.endpoint), &[flags, &[p]].concat(), Duration::from_secs(120)).map_err(|e| {
            let downgrade = e.message.contains("VERSION_DOWNGRADE");
            if downgrade {
                Error::new(e.code, "a newer helper is installed; run au uninstall first to go back to an older one")
            } else {
                e
            }
        })?;
        Ok(())
    }
    /// The installed helper's `versionCode`, or `None` when it is not installed.
    pub fn helper_version_code(&self, d: &Device) -> Result<Option<u64>> {
        let out = self.run(Some(&d.endpoint), &["shell", "dumpsys", "package", "dev.codex.aubridge"], Duration::from_secs(10))?;
        Ok(String::from_utf8_lossy(&out).split_whitespace().find_map(|w| w.strip_prefix("versionCode=")?.parse().ok()))
    }
    pub fn uninstall_helper(&self, d: &Device) -> Result<()> {
        self.run(Some(&d.endpoint), &["uninstall", "dev.codex.aubridge"], Duration::from_secs(30))?;
        Ok(())
    }
}

/// Reads `Row: 0 key=BASE64` from `content query`.
fn parse_key(output: &str) -> Result<Vec<u8>> {
    let fail = || Error::new(Code::Helper, "cannot read the helper key; update the helper with `au update`");
    let value = output.lines().find_map(|line| line.split_once("key=")).map(|(_, v)| v.trim()).ok_or_else(fail)?;
    let key = base64::engine::general_purpose::STANDARD.decode(value).map_err(|_| fail())?;
    if key.len() < 16 {
        return Err(fail());
    }
    Ok(key)
}

const READER_GRACE: Duration = Duration::from_secs(2);

/// Reads a child pipe on its own thread; the reader is abandoned if the pipe never closes.
fn read_pipe(pipe: impl Read + Send + 'static) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut b = Vec::new();
        let _ = tx.send(pipe.take(MAX_ADB_OUTPUT + 1).read_to_end(&mut b).map(|_| b));
    });
    rx
}

/// Waits for a child that inherits the terminal; past `timeout` it is killed.
pub fn wait_deadline(child: &mut std::process::Child, timeout: Duration) -> Result<std::process::ExitStatus> {
    let end = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::new(Code::Timeout, "the command did not finish in time and was stopped"));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn run_bounded(program: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
    let name = program.file_stem().map_or_else(|| "command".into(), |stem| stem.to_string_lossy().into_owned());
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::new(Code::Device, format!("failed to start {name}: {e}")))?;
    let (out, err) = (read_pipe(child.stdout.take().unwrap()), read_pipe(child.stderr.take().unwrap()));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::new(Code::Timeout, format!("{name} command timed out")));
        }
        thread::sleep(Duration::from_millis(10));
    };
    // A grandchild (such as the adb server) can keep a pipe open after the command exits; wait for
    // the output a short grace, never past the deadline, then abandon the reader.
    let collect = |rx: mpsc::Receiver<std::io::Result<Vec<u8>>>, pipe: &str| match rx.recv_timeout(READER_GRACE.min(deadline.saturating_duration_since(Instant::now()))) {
        Ok(read) => Ok(read?),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::new(Code::Timeout, format!("{name} {pipe} stayed open after the command ended; the command was abandoned"))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::new(Code::Io, format!("{name} {pipe} reader failed"))),
    };
    let (stdout, stderr) = (collect(out, "stdout")?, collect(err, "stderr")?);
    if stdout.len() as u64 > MAX_ADB_OUTPUT || stderr.len() as u64 > MAX_ADB_OUTPUT {
        return Err(Error::new(Code::Bounds, format!("{name} output exceeded 256 KiB")));
    }
    if !status.success() {
        let detail = sanitize_stderr(&stderr);
        return Err(Error::new(Code::Device, format!("{name} command failed with {status}: {detail}")));
    }
    Ok(stdout)
}

fn sanitize_stderr(bytes: &[u8]) -> String {
    let raw: String = String::from_utf8_lossy(bytes).chars().map(|c| if c.is_control() && !matches!(c, '\t' | '\n' | '\r') { ' ' } else { c }).collect();
    let mut parts = Vec::new();
    let mut previous = None;
    for word in raw.split_whitespace().take(32) {
        let lower = word.to_ascii_lowercase();
        let safe = if lower.contains("token") || lower.contains("secret") || lower.contains("password") || word.len() > 160 || word.contains(":\\") || word.contains("/") {
            "<redacted>"
        } else {
            word
        };
        if previous != Some(safe) {
            parts.push(safe);
            previous = Some(safe);
        }
    }
    let text = parts.join(" ");
    if text.is_empty() {
        "no diagnostic".into()
    } else {
        text.chars().take(512).collect()
    }
}

/// Artifacts older than this are deleted when a target's state opens.
const ARTIFACT_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// Options for a new host-state file: mode 0600 from creation on Unix, so no umask window exists.
pub(crate) fn private_file() -> fs::OpenOptions {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut options = fs::OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

/// Creates `path` and any missing parents with mode 0700, then tightens it and the files au
/// keeps in it (all of them in `artifacts/`). Other entries are never touched: `AU_HOME` is a
/// folder the user chose and may hold their own files.
pub(crate) fn private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)?;
    tighten(path);
    Ok(())
}

/// The files au writes into a state folder.
#[cfg(unix)]
fn ours(name: &str) -> bool {
    name.starts_with("operations.")
        || ["device.json", "lock", "screen-space.json", "aubridge.apk", "target.json", "instance", "emulator.json", "emulator.log", "ports.lock"].contains(&name)
}

/// Makes a folder 0700 and its au-written files 0600 when this user owns them. Anything owned by
/// someone else is left alone with a warning, so a shared `AU_HOME` is never chmod-ed out from
/// under them. Ownership is learned from a file created inside the folder itself.
#[cfg(unix)]
fn tighten(dir: &Path) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let probe = dir.join(format!(".au-uid-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()));
    let me = private_file().write(true).create_new(true).open(&probe).and_then(|file| file.metadata()).map(|meta| meta.uid()).ok();
    let _ = fs::remove_file(probe);
    let all = dir.file_name().is_some_and(|name| name == "artifacts");
    let fix = |path: &Path, mode: u32| {
        let Ok(meta) = fs::symlink_metadata(path) else { return };
        if meta.file_type().is_symlink() || meta.mode() & 0o077 == 0 {
            return;
        }
        if Some(meta.uid()) == me && fs::set_permissions(path, fs::Permissions::from_mode(mode)).is_ok() {
            return;
        }
        eprintln!("au: {} is readable by other users and is not yours to change; make it private", path.display());
    };
    fix(dir, 0o700);
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_file()) && (all || entry.file_name().to_str().is_some_and(ours)) {
            fix(&entry.path(), 0o600);
        }
    }
}
#[cfg(not(unix))]
fn tighten(_: &Path) {}

/// Deletes regular files in `dir` last modified longer ago than `ttl`.
pub(crate) fn expire(dir: &Path, ttl: Duration) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = entry.metadata().ok().filter(|meta| meta.is_file()).and_then(|meta| meta.modified().ok()?.elapsed().ok()).is_some_and(|age| age > ttl);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return Err(Error::new(Code::Io, "refusing to replace a symbolic link"));
    }
    let suffix = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let tmp = path.with_extension(format!("tmp-{suffix}"));
    let backup = path.with_extension(format!("bak-{suffix}"));
    {
        let mut f = private_file().write(true).create_new(true).open(&tmp)?;
        std::io::Write::write_all(&mut f, bytes)?;
        f.sync_all()?;
    }
    // On Unix a rename replaces the target in one step, so a crash leaves either
    // the old or the new file. Platforms that cannot rename over an existing
    // file move the old one aside first; that path can briefly leave no file.
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_) if path.exists() => {
            fs::rename(path, &backup)?;
            if let Err(e) = fs::rename(&tmp, path) {
                let _ = fs::rename(&backup, path);
                let _ = fs::remove_file(&tmp);
                return Err(e.into());
            }
            fs::remove_file(backup)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            Err(error.into())
        }
    }
}

/// Restores the newest file an interrupted `atomic` moved aside, if any.
pub fn recover(path: &Path) -> Result<()> {
    let (false, Some(dir), Some(stem)) = (path.exists(), path.parent(), path.file_stem().and_then(|stem| stem.to_str())) else { return Ok(()) };
    let nanos = |name: &str| name.strip_prefix(&format!("{stem}.bak-"))?.rsplit('-').next()?.parse::<u128>().ok();
    if let Some((_, name)) = fs::read_dir(dir)?.filter_map(|entry| entry.ok()?.file_name().into_string().ok()).filter_map(|name| Some((nanos(&name)?, name))).max() {
        fs::rename(dir.join(name), path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> Binding {
        Binding { hardware: "SER1".into(), endpoint: "emulator-5554".into() }
    }

    #[test]
    fn the_helper_key_is_parsed_from_content_query_output() {
        assert_eq!(parse_key("Row: 0 key=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n").unwrap().len(), 32);
        for bad in ["No result found.\n", "", "Row: 0 key=!!!\n", "Row: 0 key=AAAA\n"] {
            assert!(parse_key(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_pipe_held_open_after_the_child_exits_returns_within_the_deadline() {
        let start = Instant::now();
        let error = run_bounded(Path::new("sh"), &["-c", "sleep 5 & exit 0"], Duration::from_secs(10)).unwrap_err();
        assert!(error.code == Code::Timeout && start.elapsed() < Duration::from_secs(8), "{error:?}");
    }

    #[test]
    fn a_shared_serial_is_never_guessed() {
        let pair = |a: &str, b: &str| (Box::<str>::from(a), Box::<str>::from(b));
        // The enrolled endpoint wins even when another transport reports the same serial.
        assert_eq!(choose_endpoint(&binding(), &[pair("emulator-5556", "SER1"), pair("emulator-5554", "SER1")]).unwrap().as_ref(), "emulator-5554");
        // Moved to another transport: one match is accepted, two are refused.
        assert_eq!(choose_endpoint(&binding(), &[pair("10.0.0.5:5555", "SER1"), pair("other", "SER2")]).unwrap().as_ref(), "10.0.0.5:5555");
        let error = choose_endpoint(&binding(), &[pair("emulator-5556", "SER1"), pair("emulator-5558", "SER1")]).unwrap_err();
        assert_eq!(error.code, Code::Ambiguous);
        assert_eq!(choose_endpoint(&binding(), &[pair("x", "SER2")]).unwrap_err().code, Code::Identity);
    }

    #[test]
    fn paths_are_scoped() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::at(d.path().to_path_buf()).unwrap();
        assert!(p.root.starts_with(d.path()));
    }

    #[test]
    fn stderr_is_bounded_and_sanitized() {
        let value = sanitize_stderr(b"\x1b[31mfailed token=secret C:\\Users\\person\\secret\x1b[0m failed failed");
        assert!(!value.contains('\x1b'));
        assert!(!value.to_ascii_lowercase().contains("token"));
        assert!(!value.contains("C:\\"));
        assert!(value.len() <= 512);
    }
}

#[cfg(all(test, unix))]
mod private_state_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn host_state_is_private_and_old_artifacts_expire() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("state");
        fs::create_dir_all(root.join("artifacts")).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        let (old, fresh) = (root.join("artifacts/old.png"), root.join("artifacts/fresh.png"));
        fs::write(&old, "x").unwrap();
        fs::write(&fresh, "x").unwrap();
        fs::write(root.join("device.json"), "{}").unwrap();
        fs::write(root.join("gradlew"), "#!/bin/sh").unwrap();
        fs::set_permissions(root.join("gradlew"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::File::options().write(true).open(&old).unwrap().set_modified(std::time::SystemTime::now() - ARTIFACT_TTL - Duration::from_secs(60)).unwrap();

        let paths = Paths::at(root.clone()).unwrap();
        assert_eq!((mode(&root), mode(&paths.artifacts), mode(&root.join("device.json")), mode(&fresh)), (0o700, 0o700, 0o600, 0o600));
        assert!(!old.exists() && fresh.exists());
        assert_eq!(mode(&root.join("gradlew")), 0o755, "files au did not write are left alone");

        atomic(&root.join("new.json"), b"{}").unwrap();
        paths.lock_for_serving().unwrap();
        assert_eq!((mode(&root.join("new.json")), mode(&root.join("lock"))), (0o600, 0o600));
    }
}
