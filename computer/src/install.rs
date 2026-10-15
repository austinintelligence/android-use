use crate::{
    api::{Code, Error, Result},
    device::{Adb, Paths},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn next_step(kind: &str, title: &str, steps: &[&str], resume: &str, command: Option<&str>) -> Value {
    let mut value = json!({"kind":kind,"title":title,"steps":steps,"resume":resume});
    if let Some(command) = command {
        value["command"] = json!(command);
    }
    value
}

fn connect_device_step() -> Value {
    next_step(
        "user",
        "Connect and authorize one Android device",
        &[
            "Unlock the phone or tablet.",
            "Use a USB cable that carries data, not charge only.",
            "Open Settings → About phone and tap Build number seven times if Developer options is not visible.",
            "Open Developer options and turn on USB debugging.",
            "Reconnect the device and tap Allow on the Android USB debugging prompt.",
        ],
        "Run au doctor --json, then au setup --json.",
        None,
    )
}

fn approve_usb_step() -> Value {
    next_step(
        "user",
        "Approve USB debugging on Android",
        &[
            "Unlock the phone or tablet and look for the Allow USB debugging? prompt.",
            "Tap Allow. Choose Always allow only if this is your computer.",
            "Leave the device connected while Android Use checks it again.",
        ],
        "Run au doctor --json, then au setup --json.",
        None,
    )
}

fn choose_device_step() -> Value {
    next_step(
        "user",
        "Choose the Android device to enroll",
        &[
            "Disconnect extra Android devices, or inspect the endpoints printed by au devices.",
            "Tell the agent which endpoint belongs to the device you want to control.",
            "The agent will run au enroll ENDPOINT and verify the hardware identity before continuing.",
        ],
        "Run au setup --json after one device is selected.",
        None,
    )
}

fn reconnect_device_step() -> Value {
    next_step(
        "user",
        "Reconnect the enrolled Android device",
        &[
            "Connect the same phone or tablet that was enrolled before.",
            "Unlock it and keep USB debugging enabled.",
            "If Android asks again, approve USB debugging for this computer.",
        ],
        "Run au doctor --json, then au setup --json.",
        None,
    )
}

fn install_helper_step() -> Value {
    next_step(
        "agent",
        "Install the Android Use helper",
        &[
            "The device is authorized. Android Use can install its bundled helper without changing unrelated apps.",
            "Release builds of au carry the helper; with a development build keep aubridge.apk beside au.",
        ],
        "Run au setup --json.",
        Some("au setup --json"),
    )
}

fn enable_accessibility_step() -> Value {
    next_step(
        "user",
        "Enable Android Use Accessibility",
        &[
            "On Android, open Settings → Accessibility → Android-Use.",
            "Turn Android-Use on and approve Android's warning.",
            "Leave the device unlocked while Android Use verifies the service.",
        ],
        "Run au setup --json, or run au doctor --json to verify readiness.",
        None,
    )
}

fn ready_step() -> Value {
    next_step(
        "ready",
        "Android Use is ready for an agent",
        &[
            "Register the MCP server with an agent: au install claude (or codex, cursor, gemini, vscode, claude-desktop, opencode), then reload the agent.",
            "Verify with android_read status, then android_read screen, without changing the device.",
            "Shell-only agents can use the same commands directly: au read screen, au act 'tap \"LABEL\"'.",
        ],
        "Start the local server with au serve --mcp.",
        Some("au serve --mcp"),
    )
}

fn platform_tools_step() -> Value {
    next_step(
        "computer",
        "Install or locate Android platform tools",
        &[
            "Use a trusted Android SDK platform-tools installation if one already exists.",
            "Otherwise install the official Android SDK Platform-Tools from https://developer.android.com/tools/releases/platform-tools, or set AU_ADB to the full path of adb.",
            "Keep ADB local; Android Use uses it to reach only the enrolled device.",
        ],
        "Run au doctor --json again.",
        None,
    )
}

pub fn enroll(endpoint: &str) -> Result<Value> {
    let paths = Paths::discover()?;
    let adb = Adb::discover()?;
    let _ = adb.enroll(&paths, endpoint)?;
    Ok(json!({"ok":1,"enrolled":1}))
}
pub fn devices() -> Result<Value> {
    let adb = Adb::discover()?;
    let devices = adb.devices_all()?.into_iter().map(|d| json!({"endpoint":d.endpoint,"state":d.state})).collect::<Vec<_>>();
    Ok(json!({"ok":1,"devices":devices}))
}
pub fn setup(apk: Option<&Path>, wait_for_approval: bool) -> Result<Value> {
    let paths = Paths::discover()?;
    let adb = Adb::discover()?;
    let (device, enrolled_now) = adb.resolve_or_enroll(&paths)?;
    let (apk, embedded) = apk_to_install(apk, &paths)?;
    adb.wait_boot_completed(&device.endpoint, std::time::Duration::from_secs(120))?;
    adb.install(&device, &apk, embedded)?;
    adb.start_helper(&device, true)?;
    let installed = adb.package_installed(&device, "dev.codex.aubridge")?;
    let mut accessibility = adb.accessibility_enabled(&device).unwrap_or(false);
    let mut settings_opened = false;
    if !accessibility {
        adb.open_accessibility_settings(&device)?;
        settings_opened = true;
        if wait_for_approval {
            accessibility = adb.wait_for_accessibility(&device, std::time::Duration::from_secs(60))?;
        }
    }
    let phase = if !installed {
        "install_helper"
    } else if !accessibility {
        "enable_accessibility"
    } else {
        "ready"
    };
    let step = if !installed {
        install_helper_step()
    } else if !accessibility {
        enable_accessibility_step()
    } else {
        ready_step()
    };
    Ok(
        json!({"ok":1,"installed":installed as u8,"enrolled":enrolled_now as u8,"device":device.hardware,"accessibility":accessibility as u8,"settings_opened":settings_opened as u8,"ready":(installed&&accessibility) as u8,"phase":phase,"next":(!installed).then_some("Android Use could not confirm the helper after installation; run au setup again.").or((!accessibility).then_some("Android Use opened Accessibility settings on your device. Tap Android-Use, turn it on, then run au setup again.")),"next_step":step}),
    )
}
pub fn doctor() -> Result<Value> {
    let paths = Paths::discover()?;
    let journal_bytes = fs::metadata(&paths.journal).map(|m| m.len()).unwrap_or(0);
    let adb = match Adb::discover() {
        Ok(adb) => adb,
        Err(e) => {
            return Ok(
                json!({"ok":0,"ready":0,"phase":"install_platform_tools","checks":{"computer":{"state":"broken","message":"Android device tools were not found. Install Android platform tools or set AU_ADB."}},"next_step":platform_tools_step(),"error":e.code.wire()}),
            )
        }
    };
    let all = adb.devices_all()?;
    let ready: Vec<_> = all.iter().filter(|d| d.state.as_ref() == "device").collect();
    let device_states = all.iter().map(|d| json!({"endpoint":d.endpoint,"state":d.state})).collect::<Vec<_>>();
    let mut device = None;
    let enrolled = paths.device.is_file();
    if enrolled {
        match adb.resolve(&paths) {
            Ok(d) => device = Some(d),
            Err(_) => {
                // Keep the detailed recovery state below instead of failing the whole doctor read.
            }
        };
    }
    let mut checks = serde_json::Map::new();
    checks.insert("computer".into(), json!({"state":"ready","message":"Android device tools available"}));
    let enrolled_connected = enrolled && device.is_some();
    let connection_message = if ready.len() == 1 {
        "One authorized Android device is connected"
    } else if enrolled_connected {
        "The enrolled Android device is connected; extra devices are ignored"
    } else if ready.is_empty() && all.iter().any(|d| d.state.as_ref() == "unauthorized") {
        "Android is waiting for USB debugging approval on the device"
    } else if ready.is_empty() && all.iter().any(|d| d.state.as_ref() == "offline") {
        "An Android device is offline; unlock it and reconnect"
    } else if ready.is_empty() {
        "No Android device is connected"
    } else {
        "More than one Android device is connected"
    };
    checks.insert(
        "connection".into(),
        json!({"state":if ready.len()==1 || enrolled_connected{"ready"}else{"attention"},"message":connection_message,"count":ready.len(),"seen":all.len()}),
    );
    if !enrolled {
        checks.insert("enrollment".into(), json!({"state":"attention","message":"No device is enrolled yet; au setup can enroll one connected device"}));
    } else if device.is_some() {
        checks.insert("enrollment".into(), json!({"state":"ready","message":"The enrolled Android device is connected"}));
    } else {
        checks.insert("enrollment".into(), json!({"state":"attention","message":"The enrolled device is not connected"}));
    }
    let (helper, accessibility, notifications, browser) = if let Some(d) = device.as_ref() {
        (
            adb.package_installed(d, "dev.codex.aubridge").unwrap_or(false),
            adb.accessibility_enabled(d).unwrap_or(false),
            adb.notifications_enabled(d).unwrap_or(false),
            adb.browser_installed(d).unwrap_or(false),
        )
    } else {
        (false, false, false, false)
    };
    checks.insert(
        "helper".into(),
        json!({"state":if helper{"ready"}else{"attention"},"message":if helper{"Android Use helper is installed"}else{"Run au setup to install the Android helper"}}),
    );
    checks.insert("accessibility".into(), json!({"state":if accessibility{"ready"}else{"attention"},"message":if accessibility{"Accessibility is enabled"}else{"On Android, enable Android-Use under Settings → Accessibility"}}));
    checks.insert("notifications".into(), json!({"state":if notifications{"ready"}else{"optional"},"message":if notifications{"Notification access is enabled"}else{"Optional: enable notification access when needed"}}));
    checks.insert(
        "browser".into(),
        json!({"state":if browser{"ready"}else{"optional"},"message":if browser{"Chrome is available"}else{"Optional: install Chrome for browser control"}}),
    );
    // Installed and enabled is not the same as answering: a helper that crashes on start still
    // shows both, so ask it something.
    let answering = match (helper && accessibility, device.as_ref()) {
        (true, Some(d)) => {
            use crate::link::DeviceLink as _;
            let mut bridge = crate::link::AdbLink::to(paths.clone(), adb.clone(), d.clone()).helper();
            let mut answered = bridge.as_mut().is_ok_and(|bridge| bridge.status().is_ok());
            if let Some(env) = bridge.as_mut().ok().filter(|_| answered).and_then(|bridge| bridge.query("env", Value::Null).ok()) {
                if env.get("stopped").and_then(Value::as_bool) == Some(true) {
                    answered = false;
                    checks.insert("bridge".into(), json!({"state":"attention","message":"Android-Use was stopped from its notification; run au setup to turn it on"}));
                }
                for (name, flag, ready, bad) in [
                    ("battery", "battery_exempt", true, "Battery optimization is on; exempt Android-Use under Settings → Battery so Android keeps the helper alive"),
                    ("talkback", "talkback", false, "TalkBack is on and can change what the screen reports"),
                ] {
                    if env.get(flag).and_then(Value::as_bool).is_some_and(|on| on != ready) {
                        checks.insert(name.into(), json!({"state":"optional","message":bad}));
                    }
                }
            }
            answered
        }
        _ => true,
    };
    if !answering && !checks.contains_key("bridge") {
        checks.insert("bridge".into(), json!({"state":"attention","message":"The helper is installed but not answering; run au repair"}));
    }
    let (phase, step) = if ready.is_empty() {
        if all.iter().any(|d| d.state.as_ref() == "unauthorized") {
            ("approve_usb", approve_usb_step())
        } else if enrolled {
            ("reconnect_device", reconnect_device_step())
        } else {
            ("connect_device", connect_device_step())
        }
    } else if ready.len() > 1 && device.is_none() {
        ("choose_device", choose_device_step())
    } else if !enrolled || device.is_none() {
        ("setup_host", install_helper_step())
    } else if !helper {
        ("install_helper", install_helper_step())
    } else if !accessibility {
        ("enable_accessibility", enable_accessibility_step())
    } else if !answering {
        ("install_helper", install_helper_step())
    } else {
        ("ready", ready_step())
    };
    let ready_all = enrolled && device.is_some() && helper && accessibility && answering;
    Ok(json!({"ok":1,"ready":ready_all as u8,"phase":phase,"checks":checks,"devices":device_states,"next_step":step,"journal_bytes":journal_bytes}))
}
pub fn update(apk: Option<&Path>, wait_for_approval: bool) -> Result<Value> {
    let mut value = setup(apk, wait_for_approval)?;
    value["updated"] = json!(1);
    Ok(value)
}
/// `au uninstall [--dry-run]`. A dry run only lists what would go. A real uninstall refuses while
/// `au serve` holds the state lock, and holds that lock itself so no server starts mid-removal.
pub fn uninstall(dry_run: bool) -> Result<Value> {
    if dry_run {
        return uninstall_plan(&Paths::locate()?);
    }
    let paths = Paths::discover()?;
    let _lock = paths.lock_for_serving().map_err(|e| match e.code {
        Code::Device => Error::new(Code::Device, "au serve is running for this target; stop it first, then run au uninstall again"),
        _ => e,
    })?;
    let adb = Adb::discover()?;
    let device = adb.resolve(&paths)?;
    adb.uninstall_helper(&device)?;
    let _ = fs::remove_file(&paths.device);
    let _ = fs::remove_file(&paths.journal);
    let _ = fs::remove_file(paths.root.join("screen-space.json"));
    let _ = fs::remove_dir_all(&paths.artifacts);
    Ok(json!({"ok":1,"uninstalled":1,"local_state_removed":1}))
}
/// The `--dry-run` report. It reads the enrolled-device file and lists each local item that exists,
/// so it needs no adb or connected device and creates nothing.
fn uninstall_plan(paths: &Paths) -> Result<Value> {
    let mut would_remove = Vec::new();
    if let Some(endpoint) = fs::read(&paths.device).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()).and_then(|v| v["endpoint"].as_str().map(str::to_owned)) {
        would_remove.push(format!("helper app dev.codex.aubridge on {endpoint}"));
    }
    for (label, path) in
        [("device binding", &paths.device), ("operation journal", &paths.journal), ("screen space", &paths.root.join("screen-space.json")), ("artifacts", &paths.artifacts)]
    {
        if path.exists() {
            would_remove.push(format!("{label}: {}", path.display()));
        }
    }
    Ok(json!({"ok":1,"dry_run":1,"would_remove":would_remove}))
}
/// The helper APK to install: the path given, else `aubridge.apk` beside `au`, else the copy built
/// into release binaries (written to private state first, after its hash is checked).
fn apk_to_install(given: Option<&Path>, paths: &Paths) -> Result<(PathBuf, bool)> {
    if let Some(path) = given.map(PathBuf::from).or_else(bundled_apk) {
        return if path.is_file() { Ok((path, false)) } else { Err(Error::new(Code::Args, "Android-Use helper APK was not found")) };
    }
    let bytes = embedded_apk().ok_or_else(|| Error::new(Code::Args, "pass the Android-Use helper APK path or place aubridge.apk beside au"))?;
    let path = paths.root.join("aubridge.apk");
    crate::device::atomic(&path, bytes)?;
    Ok((path, true))
}

/// The `versionCode` this `au` expects of the helper: major*1,000,000 + minor*1,000 + patch.
fn expected_version_code() -> u64 {
    [env!("CARGO_PKG_VERSION_MAJOR"), env!("CARGO_PKG_VERSION_MINOR"), env!("CARGO_PKG_VERSION_PATCH")].iter().fold(0, |a, p| a * 1000 + p.parse::<u64>().unwrap_or(0))
}
/// Installs the embedded helper over an older installed one. Never downgrades, and does nothing
/// when this `au` embeds no helper or the helper is not installed.
pub(crate) fn upgrade_helper(adb: &Adb, d: &crate::device::Device) -> Result<()> {
    let Some(bytes) = embedded_apk() else { return Ok(()) };
    if adb.helper_version_code(d)?.is_none_or(|installed| installed >= expected_version_code()) {
        return Ok(());
    }
    let path = Paths::discover()?.root.join("aubridge.apk");
    crate::device::atomic(&path, bytes)?;
    adb.install(d, &path, true)
}

#[cfg(embedded_apk)]
fn embedded_apk() -> Option<&'static [u8]> {
    use sha2::{Digest, Sha256};
    static APK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/aubridge.apk"));
    let hash: String = Sha256::digest(APK).iter().map(|b| format!("{b:02x}")).collect();
    (hash == env!("AU_APK_SHA256")).then_some(APK)
}

#[cfg(not(embedded_apk))]
fn embedded_apk() -> Option<&'static [u8]> {
    None
}

fn bundled_apk() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join("aubridge.apk");
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guided_steps_are_machine_readable_and_resumable() {
        let step = enable_accessibility_step();
        assert_eq!(step["kind"], "user");
        assert_eq!(step["title"], "Enable Android Use Accessibility");
        assert!(step["steps"].as_array().is_some_and(|steps| steps.len() >= 2));
        assert!(step["resume"].as_str().is_some_and(|resume| resume.contains("doctor")));
    }

    #[test]
    fn ready_step_points_agents_to_local_mcp() {
        let step = ready_step();
        assert_eq!(step["kind"], "ready");
        assert_eq!(step["command"], "au serve --mcp");
        assert!(step["resume"].as_str().is_some_and(|resume| resume.contains("serve --mcp")));
    }
}
