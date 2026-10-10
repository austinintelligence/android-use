//! `au emulator …`: create, start, stop, and destroy Android emulators that belong to Android Use,
//! each bound to a named target (`au --target NAME …`).
//!
//! Only emulators created here are touched, and only under Android Use's own AVD folder
//! (`<state>/emulators/avd`). The SDK and system images are never downloaded or bundled: they come
//! from the user's Android SDK (their license), and `doctor` prints the install command when one is
//! missing. Accessibility is enabled on the emulator the way it is on a phone, through Android's
//! own screen; nothing here writes a setting that would skip that consent.
use crate::{
    api::{Code, Error, Result},
    device::{Adb, Paths},
};
use serde_json::{json, Value};
use std::{
    env, fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// First console port; each instance takes an even console port and the odd one above it for adb.
const FIRST_PORT: u16 = 5554;
const LAST_PORT: u16 = 5584;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// `android-36;google_apis;arm64-v8a`, as `sdkmanager` names it.
    pub id: String,
    pub api: u32,
    pub tag: String,
    pub abi: String,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn sdk_root() -> Result<PathBuf> {
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    let candidates = [
        env::var_os("ANDROID_HOME").map(PathBuf::from),
        env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from),
        Some(home.join("Library/Android/sdk")),
        Some(home.join("Android/Sdk")),
        env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Android/Sdk")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|dir| dir.join("platform-tools").is_dir() || dir.join("emulator").is_dir())
        .ok_or_else(|| Error::new(Code::Device, "no Android SDK found; install Android Studio or the command-line tools, or set ANDROID_HOME"))
}

fn emulator_binary(sdk: &Path) -> Result<PathBuf> {
    let path = sdk.join("emulator").join(if cfg!(windows) { "emulator.exe" } else { "emulator" });
    path.is_file().then_some(path).ok_or_else(|| Error::new(Code::Device, "the Android emulator is not installed; run: sdkmanager \"emulator\""))
}

/// System images installed in the SDK, as `sdkmanager` ids.
pub fn images(sdk: &Path) -> Vec<Image> {
    let mut out = Vec::new();
    let read = |dir: &Path| fs::read_dir(dir).map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect::<Vec<_>>()).unwrap_or_default();
    for api_dir in read(&sdk.join("system-images")) {
        let api = api_dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let Some(number) = api.strip_prefix("android-").and_then(|n| n.split(['-', '.']).next()).and_then(|n| n.parse().ok()) else { continue };
        for tag_dir in read(&api_dir) {
            for abi_dir in read(&tag_dir) {
                if !abi_dir.join("system.img").is_file() && !abi_dir.join("source.properties").is_file() {
                    continue;
                }
                let (tag, abi) = (tag_dir.file_name(), abi_dir.file_name());
                if let (Some(tag), Some(abi)) = (tag.and_then(|n| n.to_str()), abi.and_then(|n| n.to_str())) {
                    out.push(Image { id: format!("{api};{tag};{abi}"), api: number, tag: tag.into(), abi: abi.into() });
                }
            }
        }
    }
    out.sort_by(|a, b| b.api.cmp(&a.api).then_with(|| a.id.cmp(&b.id)));
    out
}

/// The ABI this computer can run at speed.
fn host_abi() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64-v8a"
    } else {
        "x86_64"
    }
}

/// The image to use: the one named, else the newest `google_apis` image for this computer (it
/// carries Chrome; Play images are locked down and `default` images have no browser).
pub fn pick(images: &[Image], wanted: Option<&str>) -> Result<Image> {
    if let Some(id) = wanted {
        return images.iter().find(|image| image.id == id).cloned().ok_or_else(|| Error::new(Code::Args, format!("image {id} is not installed; run au emulator images")));
    }
    images.iter().filter(|image| image.tag == "google_apis" && image.abi == host_abi()).max_by_key(|image| image.api).cloned().ok_or_else(|| {
        Error::new(Code::Device, format!("no google_apis image for {} is installed; run: sdkmanager \"system-images;android-36;google_apis;{}\"", host_abi(), host_abi()))
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Profile {
    Small,
    Standard,
}

/// `<avd>.ini` and `config.ini` for an AVD written under Android Use's own AVD folder, so no JDK or
/// `avdmanager` is needed.
fn avd_files(name: &str, image: &Image, profile: Profile, avd_home: &Path) -> (String, String) {
    let (ram, cores, width, height, density) = match profile {
        Profile::Small => (2048, 2, 720, 1600, 320),
        Profile::Standard => (4096, 4, 1080, 2400, 420),
    };
    let ini = format!("avd.ini.encoding=UTF-8\npath={}\npath.rel=avd/{name}.avd\ntarget=android-{}\n", avd_home.join(format!("{name}.avd")).display(), image.api);
    let config = format!(
        "AvdId={name}\navd.ini.displayname={name}\nabi.type={abi}\nhw.cpu.arch={arch}\nhw.cpu.ncore={cores}\nhw.ramSize={ram}\nhw.lcd.width={width}\nhw.lcd.height={height}\nhw.lcd.density={density}\nhw.keyboard=yes\nhw.gpu.enabled=yes\nhw.gpu.mode=auto\nhw.audioInput=no\ndisk.dataPartition.size=6G\nimage.sysdir.1=system-images/android-{api}/{tag}/{abi}/\ntag.id={tag}\nPlayStore.enabled=false\n",
        abi = image.abi,
        arch = if image.abi.starts_with("arm64") { "arm64" } else { "x86_64" },
        api = image.api,
        tag = image.tag,
    );
    (ini, config)
}

fn launch_args(name: &str, console: u16, window: bool) -> Vec<String> {
    let mut args: Vec<String> = ["-avd", name, "-ports"].map(str::to_owned).to_vec();
    args.push(format!("{console},{}", console + 1));
    args.extend(["-no-audio", "-no-boot-anim", "-gpu", "auto", "-no-snapshot-save"].map(str::to_owned));
    if !window {
        args.push("-no-window".into());
    }
    args
}

/// The first console port whose pair is neither claimed by an instance nor busy on this computer.
fn free_ports(taken: &[u16]) -> Option<u16> {
    (FIRST_PORT..=LAST_PORT).step_by(2).find(|port| !taken.contains(port) && TcpListener::bind(("127.0.0.1", *port)).is_ok() && TcpListener::bind(("127.0.0.1", port + 1)).is_ok())
}

struct Layout {
    base: PathBuf,
}

impl Layout {
    fn avd_home(&self) -> PathBuf {
        self.base.join("emulators/avd")
    }
    fn target(&self, name: &str) -> PathBuf {
        self.base.join("targets").join(name)
    }
}

fn layout() -> Result<Layout> {
    Ok(Layout { base: Paths::base()? })
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Console ports claimed by running instances of any target.
fn claimed_ports(l: &Layout) -> Vec<u16> {
    let entries = fs::read_dir(l.base.join("targets")).map(|dir| dir.flatten().map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default();
    entries.iter().filter_map(|dir| read_json(&dir.join("emulator.json"))?.get("console")?.as_u64().and_then(|n| u16::try_from(n).ok())).collect()
}

pub fn doctor() -> Result<Value> {
    let sdk = sdk_root();
    let (emulator, found) = match &sdk {
        Ok(dir) => (emulator_binary(dir).is_ok(), images(dir)),
        Err(_) => (false, Vec::new()),
    };
    let usable: Vec<_> = found.iter().filter(|image| image.abi == host_abi()).collect();
    let mut fixes = Vec::new();
    if sdk.is_err() {
        fixes.push("install Android Studio or the Android command-line tools".to_owned());
    } else if !emulator {
        fixes.push("sdkmanager \"emulator\"".into());
    }
    if usable.iter().all(|image| image.tag != "google_apis") {
        fixes.push(format!("sdkmanager \"system-images;android-36;google_apis;{}\"", host_abi()));
    }
    Ok(
        json!({"ok": u8::from(fixes.is_empty()), "sdk": sdk.ok().map(|d| d.display().to_string()), "emulator": emulator, "abi": host_abi(), "images": found.len(), "usable_images": usable.len(), "fix": fixes}),
    )
}

pub fn list_images() -> Result<Value> {
    let sdk = sdk_root()?;
    Ok(json!({"ok":1,"abi":host_abi(),"images":images(&sdk).iter().map(|i| json!({"id":i.id,"api":i.api,"tag":i.tag,"abi":i.abi,"usable":i.abi==host_abi()})).collect::<Vec<_>>()}))
}

pub fn create(name: &str, image: Option<&str>, profile: Profile) -> Result<Value> {
    if !valid_name(name) {
        return Err(Error::new(Code::Args, "emulator names use lowercase letters, digits, and dashes (at most 32)"));
    }
    let sdk = sdk_root()?;
    let image = pick(&images(&sdk), image)?;
    let l = layout()?;
    let avd = l.avd_home().join(format!("{name}.avd"));
    if avd.exists() {
        return Err(Error::new(Code::Args, format!("{name} already exists; au emulator destroy {name} --yes first")));
    }
    let (ini, config) = avd_files(name, &image, profile, &l.avd_home());
    fs::create_dir_all(&avd)?;
    fs::write(l.avd_home().join(format!("{name}.ini")), ini)?;
    fs::write(avd.join("config.ini"), config)?;
    fs::create_dir_all(l.target(name))?;
    fs::write(l.target(name).join("target.json"), json!({"schema":1,"name":name,"kind":"emulator","avd":name,"image":image.id}).to_string())?;
    Ok(json!({"ok":1,"name":name,"image":image.id,"next":format!("au emulator start {name}")}))
}

fn adb_state(adb: &Adb, endpoint: &str) -> Option<String> {
    adb.devices_all().ok()?.into_iter().find(|d| d.endpoint.as_ref() == endpoint).map(|d| d.state.to_string())
}

/// Whether the device on `endpoint` is this target's emulator. `emulator.json` can outlive the
/// process (a crash or reboot), and a port can be taken by another emulator, so nothing may act on
/// the recorded endpoint (kill it, report it, bind it) without this check.
fn is_ours(adb: &Adb, dir: &Path, endpoint: &str) -> bool {
    let instance = fs::read_to_string(dir.join("instance")).unwrap_or_default();
    !instance.trim().is_empty() && adb.hardware(endpoint).is_ok_and(|id| *id == *format!("au-emulator-{}", instance.trim()))
}

fn require_target(l: &Layout, name: &str) -> Result<PathBuf> {
    let dir = l.target(name);
    if valid_name(name) && read_json(&dir.join("target.json")).is_some_and(|t| t["kind"] == "emulator") {
        Ok(dir)
    } else {
        Err(Error::new(Code::Args, format!("{name} is not an emulator created by au; see au emulator list")))
    }
}

pub fn start(name: &str, window: bool) -> Result<Value> {
    let l = layout()?;
    let dir = require_target(&l, name)?;
    let adb = Adb::discover()?;
    if let Some(state) = read_json(&dir.join("emulator.json"))
        .and_then(|i| i["endpoint"].as_str().map(str::to_owned))
        .and_then(|endpoint| adb_state(&adb, &endpoint).map(|s| (endpoint, s)))
        .filter(|(endpoint, _)| is_ours(&adb, &dir, endpoint))
    {
        return Ok(json!({"ok":1,"name":name,"endpoint":state.0,"state":state.1,"started":0}));
    }
    let emulator = emulator_binary(&sdk_root()?)?;
    // Port choice and the spawn happen under one lock, so two starts at once cannot pick one pair.
    fs::create_dir_all(l.base.join("emulators"))?;
    let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(l.base.join("emulators/ports.lock"))?;
    lock.lock()?;
    let mut taken = claimed_ports(&l);
    taken.extend(adb.devices_all()?.iter().filter_map(|d| d.endpoint.strip_prefix("emulator-").and_then(|p| p.parse::<u16>().ok())));
    let console = free_ports(&taken).ok_or_else(|| Error::new(Code::Bounds, "no free emulator ports (limit: 16 instances)"))?;
    let log = fs::File::create(dir.join("emulator.log"))?;
    let mut command = Command::new(&emulator);
    command.args(launch_args(name, console, window)).env("ANDROID_AVD_HOME", l.avd_home()).stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0200);
    let mut child = command.spawn().map_err(|e| Error::new(Code::Device, format!("failed to start the emulator: {e}")))?;
    let endpoint = format!("emulator-{console}");
    // The identity belongs to the target, not to one boot: enrollment records it, so a restart
    // on another port must write the same value back.
    let instance = match fs::read_to_string(dir.join("instance")).map(|s| s.trim().to_owned()) {
        Ok(known) if !known.is_empty() => known,
        _ => {
            let mut bytes = [0u8; 8];
            getrandom::fill(&mut bytes).map_err(|_| Error::new(Code::Io, "no random bytes"))?;
            let fresh = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
            fs::write(dir.join("instance"), &fresh)?;
            fresh
        }
    };
    fs::write(dir.join("emulator.json"), json!({"endpoint":endpoint,"console":console,"adb":console + 1,"pid":child.id(),"instance":instance}).to_string())?;
    let appeared = Instant::now() + Duration::from_secs(90);
    while adb_state(&adb, &endpoint).as_deref() != Some("device") {
        if child.try_wait()?.is_some() || Instant::now() > appeared {
            // A timed-out emulator must not come up later on a port nothing claims.
            let _ = child.kill();
            let _ = fs::remove_file(dir.join("emulator.json"));
            let tail = fs::read_to_string(dir.join("emulator.log")).unwrap_or_default().lines().rev().take(8).collect::<Vec<_>>().join(" | ");
            return Err(Error::plain(Code::Device, format!("the emulator did not come up; last log lines: {tail}")));
        }
        thread::sleep(Duration::from_secs(1));
    }
    drop(lock);
    adb.wait_boot_completed(&endpoint, Duration::from_secs(240))?;
    // The marker identifies this instance from now on, whatever port it gets next time.
    adb.run(Some(&endpoint), &["shell", "settings", "put", "global", "android_use_instance", &instance], Duration::from_secs(10))?;
    Ok(json!({"ok":1,"name":name,"endpoint":endpoint,"started":1,"next":format!("au --target {name} setup")}))
}

pub fn stop(name: &str) -> Result<Value> {
    let l = layout()?;
    let dir = require_target(&l, name)?;
    let Some(instance) = read_json(&dir.join("emulator.json")) else { return Ok(json!({"ok":1,"name":name,"stopped":0})) };
    let endpoint = instance["endpoint"].as_str().unwrap_or_default().to_owned();
    let adb = Adb::discover()?;
    if adb_state(&adb, &endpoint).is_some() {
        if !is_ours(&adb, &dir, &endpoint) {
            // The recorded port now belongs to something else; leave it alone.
            let _ = fs::remove_file(dir.join("emulator.json"));
            return Ok(json!({"ok":1,"name":name,"stopped":0,"note":"the recorded port is not this emulator; nothing was stopped"}));
        }
        adb.run(Some(&endpoint), &["emu", "kill"], Duration::from_secs(10))?;
        let end = Instant::now() + Duration::from_secs(20);
        while adb_state(&adb, &endpoint).is_some() {
            if Instant::now() > end {
                return Err(Error::new(Code::Timeout, format!("{endpoint} is still shutting down; try again, or close it yourself (pid {})", instance["pid"])));
            }
            thread::sleep(Duration::from_millis(500));
        }
    }
    let _ = fs::remove_file(dir.join("emulator.json"));
    Ok(json!({"ok":1,"name":name,"stopped":1}))
}

pub fn status(name: &str) -> Result<Value> {
    let l = layout()?;
    let dir = require_target(&l, name)?;
    let instance = read_json(&dir.join("emulator.json"));
    let state = instance.as_ref().and_then(|i| i["endpoint"].as_str()).and_then(|endpoint| {
        let adb = Adb::discover().ok()?;
        adb_state(&adb, endpoint).filter(|_| is_ours(&adb, &dir, endpoint))
    });
    Ok(json!({"ok":1,"name":name,"running":u8::from(state.is_some()),"endpoint":instance.as_ref().and_then(|i| i["endpoint"].as_str()),"state":state}))
}

/// `au emulator snapshot NAME save|load TAG`: a named restore point of the running emulator.
pub fn snapshot(name: &str, action: &str, tag: &str) -> Result<Value> {
    if tag.is_empty() || tag.len() > 32 || !tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(Error::new(Code::Args, "a snapshot tag is 1-32 letters, digits, - or _"));
    }
    if !matches!(action, "save" | "load") {
        return Err(Error::new(Code::Args, "use au emulator snapshot NAME save TAG or load TAG"));
    }
    let l = layout()?;
    let dir = require_target(&l, name)?;
    let endpoint = read_json(&dir.join("emulator.json"))
        .and_then(|i| i["endpoint"].as_str().map(str::to_owned))
        .ok_or_else(|| Error::new(Code::Device, format!("{name} is not running; au emulator start {name}")))?;
    let out = Adb::discover()?.run(Some(&endpoint), &["emu", "avd", "snapshot", action, tag], Duration::from_secs(120))?;
    let text = String::from_utf8_lossy(&out);
    if text.lines().any(|line| line.starts_with("KO")) {
        return Err(Error::new(Code::Device, format!("the emulator refused to {action} snapshot {tag}: {}", text.trim())));
    }
    Ok(json!({"ok":1,"name":name,"snapshot":tag,"action":action}))
}

pub fn list() -> Result<Value> {
    let l = layout()?;
    let names = fs::read_dir(l.base.join("targets")).map(|dir| dir.flatten().filter_map(|e| e.file_name().into_string().ok()).collect::<Vec<_>>()).unwrap_or_default();
    let mut rows: Vec<Value> = names.iter().filter(|name| require_target(&l, name).is_ok()).filter_map(|name| status(name).ok()).collect();
    rows.sort_by_key(|row| row["name"].as_str().unwrap_or_default().to_owned());
    Ok(json!({"ok":1,"emulators":rows}))
}

pub fn destroy(name: &str, yes: bool) -> Result<Value> {
    let l = layout()?;
    let dir = require_target(&l, name)?;
    if !yes {
        return Err(Error::new(Code::Args, format!("this deletes the emulator {name}, its data, and its target state; pass --yes to confirm")));
    }
    stop(name)?;
    let _ = fs::remove_dir_all(l.avd_home().join(format!("{name}.avd")));
    let _ = fs::remove_file(l.avd_home().join(format!("{name}.ini")));
    fs::remove_dir_all(dir)?;
    Ok(json!({"ok":1,"name":name,"destroyed":1}))
}

/// The endpoint of the running emulator behind this target, if it is one (used to enroll it).
pub fn running_endpoint(paths: &Paths) -> Option<String> {
    read_json(&paths.root.join("emulator.json"))?.get("endpoint")?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(api: u32, tag: &str, abi: &str) -> Image {
        Image { id: format!("android-{api};{tag};{abi}"), api, tag: tag.into(), abi: abi.into() }
    }

    #[test]
    fn names_are_restricted_and_images_are_picked_for_this_computer() {
        assert!(valid_name("pixel-a") && valid_name("a1"));
        assert!(!valid_name("") && !valid_name("A") && !valid_name("a b") && !valid_name("../x") && !valid_name(&"a".repeat(33)));
        let found =
            [image(34, "google_apis", host_abi()), image(36, "google_apis_playstore", host_abi()), image(36, "google_apis", "other-abi"), image(36, "google_apis", host_abi())];
        assert_eq!(pick(&found, None).unwrap().api, 36, "newest google_apis for this computer's ABI");
        assert_eq!(pick(&found, None).unwrap().abi, host_abi());
        assert!(pick(&found, Some("android-99;x;y")).is_err());
        assert!(pick(&[image(36, "google_apis_playstore", host_abi())], None).is_err(), "Play images are not picked");
    }

    #[test]
    fn images_are_read_from_the_sdk_layout() {
        let dir = tempfile::tempdir().unwrap();
        let abi = dir.path().join("system-images/android-36/google_apis/arm64-v8a");
        fs::create_dir_all(&abi).unwrap();
        fs::write(abi.join("source.properties"), "x").unwrap();
        fs::create_dir_all(dir.path().join("system-images/android-36/google_apis/empty")).unwrap();
        assert_eq!(images(dir.path()), [image(36, "google_apis", "arm64-v8a")]);
    }

    #[test]
    fn avd_files_point_at_the_image_and_the_private_folder() {
        let home = Path::new("/state/emulators/avd");
        let (ini, config) = avd_files("pixel-a", &image(36, "google_apis", "arm64-v8a"), Profile::Small, home);
        assert!(ini.contains(&format!("path={}", home.join("pixel-a.avd").display())) && ini.contains("target=android-36"));
        assert!(config.contains("image.sysdir.1=system-images/android-36/google_apis/arm64-v8a/") && config.contains("hw.ramSize=2048") && config.contains("hw.cpu.arch=arm64"));
        assert!(avd_files("x", &image(34, "google_apis", "x86_64"), Profile::Standard, Path::new("/a")).1.contains("hw.lcd.height=2400"));
    }

    #[test]
    fn launch_arguments_come_only_from_validated_fields() {
        let args = launch_args("pixel-a", 5560, false);
        assert_eq!(&args[..4], ["-avd", "pixel-a", "-ports", "5560,5561"]);
        assert!(args.contains(&"-no-window".to_owned()) && args.contains(&"-no-snapshot-save".to_owned()));
        assert!(!launch_args("pixel-a", 5560, true).contains(&"-no-window".to_owned()));
    }

    #[test]
    fn ports_skip_claimed_pairs_and_pairs_in_use() {
        let busy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        assert!(free_ports(&[FIRST_PORT]).is_none_or(|port| port != FIRST_PORT));
        let _ = busy;
        assert_eq!(free_ports(&(FIRST_PORT..=LAST_PORT).step_by(2).collect::<Vec<_>>()), None, "every pair claimed");
    }

    #[test]
    fn snapshot_tags_and_actions_are_checked_before_anything_runs() {
        for tag in ["", "has space", "semi;colon", &"x".repeat(33)] {
            assert_eq!(snapshot("any", "save", tag).unwrap_err().code, Code::Args, "{tag:?}");
        }
        assert_eq!(snapshot("any", "delete", "ok_tag-1").unwrap_err().code, Code::Args);
    }
}
