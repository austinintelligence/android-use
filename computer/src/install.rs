use crate::{
    api::{Code, Error, Result},
    device::{Adb, Paths},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn enroll(endpoint: &str) -> Result<Value> {
    let paths = Paths::discover()?;
    let adb = Adb::discover()?;
    let _ = adb.enroll(&paths, endpoint)?;
    Ok(json!({"ok":1,"enrolled":1}))
}
pub fn setup(apk: Option<&Path>) -> Result<Value> {
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
    let accessibility = adb.accessibility_enabled(&device).unwrap_or(false);
    Ok(
        json!({"ok":1,"installed":installed as u8,"enrolled":enrolled_now as u8,"device":device.hardware,"accessibility":accessibility as u8,"ready":(installed&&accessibility) as u8,"next":(!accessibility).then_some("On Android, open Settings → Accessibility → Android Use → turn it on") }),
    )
}
pub fn doctor() -> Result<Value> {
    let paths = Paths::discover()?;
    let journal_bytes = fs::metadata(&paths.journal).map(|m| m.len()).unwrap_or(0);
    let adb = match Adb::discover() {
        Ok(adb) => adb,
        Err(e) => {
            return Ok(
                json!({"ok":0,"ready":0,"checks":{"computer":{"state":"broken","message":"Android device tools were not found. Install Android platform tools or set AU_ADB."}},"error":e.code.wire()}),
            )
        }
    };
    let all = adb.devices_all()?;
    let ready: Vec<_> = all.iter().filter(|d| d.state.as_ref() == "device").collect();
    let mut checks = serde_json::Map::new();
    checks.insert("computer".into(), json!({"state":"ready","message":"Android device tools available"}));
    checks.insert("connection".into(), json!({"state":if ready.len()==1{"ready"}else{"attention"},"message":if ready.len()==1{"One Android device is connected"}else if ready.is_empty(){"Connect and unlock one Android device"}else{"Connect one Android device or enroll a specific endpoint"},"count":ready.len()}));
    let mut device = None;
    let enrolled = paths.device.is_file();
    if enrolled {
        match adb.resolve(&paths) {
            Ok(d) => device = Some(d),
            Err(_) => {
                checks.insert("enrollment".into(), json!({"state":"attention","message":"The enrolled device is not connected"}));
            }
        };
    }
    if !enrolled {
        checks.insert("enrollment".into(), json!({"state":"attention","message":"No device is enrolled yet; au setup can enroll one connected device"}));
    } else if device.is_some() {
        checks.insert("enrollment".into(), json!({"state":"ready","message":"The enrolled Android device is connected"}));
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
    let ready_all = enrolled && device.is_some() && helper && accessibility;
    Ok(json!({"ok":1,"ready":ready_all as u8,"checks":checks,"journal_bytes":journal_bytes}))
}
pub fn update(apk: Option<&Path>) -> Result<Value> {
    let mut value = setup(apk)?;
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
