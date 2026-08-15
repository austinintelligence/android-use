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
            "Keep au and aubridge.apk together in the same release directory.",
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
            "On Android, open Settings → Accessibility → Android Use.",
            "Turn Android Use on and approve Android's warning.",
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
            "Configure the local MCP server with the absolute path to au.",
            "Use the arguments serve --mcp and reload the agent.",
            "Verify with android.read q=status, then q=observe without changing the device.",
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
    let apk = apk.map(PathBuf::from).or_else(bundled_apk).ok_or_else(|| Error::new(Code::Args, "pass the AU Bridge APK path or place aubridge.apk beside au"))?;
    if !apk.is_file() {
        return Err(Error::new(Code::Args, "AU Bridge APK was not found"));
    }
    adb.install(&device, &apk)?;
    adb.start_helper(&device)?;
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
        json!({"ok":1,"installed":installed as u8,"enrolled":enrolled_now as u8,"device":device.hardware,"accessibility":accessibility as u8,"settings_opened":settings_opened as u8,"ready":(installed&&accessibility) as u8,"phase":phase,"next":(!installed).then_some("Android Use could not confirm the helper after installation; run au setup again.").or((!accessibility).then_some("Android Use opened Accessibility settings on your device. Tap Android Use, turn it on, then run au setup again.")),"next_step":step}),
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
    checks.insert("accessibility".into(), json!({"state":if accessibility{"ready"}else{"attention"},"message":if accessibility{"Accessibility is enabled"}else{"On Android, enable Android Use under Settings → Accessibility"}}));
    checks.insert("notifications".into(), json!({"state":if notifications{"ready"}else{"optional"},"message":if notifications{"Notification access is enabled"}else{"Optional: enable notification access when needed"}}));
    checks.insert(
        "browser".into(),
        json!({"state":if browser{"ready"}else{"optional"},"message":if browser{"Chrome is available"}else{"Optional: install Chrome for browser control"}}),
    );
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
    } else {
        ("ready", ready_step())
    };
    let ready_all = enrolled && device.is_some() && helper && accessibility;
    Ok(json!({"ok":1,"ready":ready_all as u8,"phase":phase,"checks":checks,"devices":device_states,"next_step":step,"journal_bytes":journal_bytes}))
}
pub fn update(apk: Option<&Path>, wait_for_approval: bool) -> Result<Value> {
    let mut value = setup(apk, wait_for_approval)?;
    value["updated"] = json!(1);
    Ok(value)
}
pub fn uninstall() -> Result<Value> {
    let paths = Paths::discover()?;
    let adb = Adb::discover()?;
    let device = adb.resolve(&paths)?;
    adb.uninstall_helper(&device)?;
    let _ = fs::remove_file(&paths.device);
    let _ = fs::remove_file(&paths.journal);
    let _ = fs::remove_dir_all(&paths.artifacts);
    Ok(json!({"ok":1,"uninstalled":1,"local_state_removed":1}))
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
