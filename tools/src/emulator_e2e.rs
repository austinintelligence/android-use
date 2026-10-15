//! `cargo xtask emulator-e2e`: a repeatable end-to-end check of the emulator target workflow on one
//! managed emulator, using only the shipped `au` commands.
//!
//! It needs `cargo xtask android` (the debug helper APK), a built `au` (`AU_BIN`, default
//! `target/dist/au`), and the Android SDK (`au emulator install` fetches what is missing). The first
//! run asks you to turn on Android-Use under Settings → Accessibility on the emulator, which Android
//! requires you to do yourself; later runs reuse that setting. State lives under `target/e2e-home`.
//!
//! Environment: `E2E_NAME` (default `au-e2e`), `E2E_WINDOW=1` to show the emulator window,
//! `E2E_DESTROY=1` to delete the emulator afterwards, `AU_ADB` for the adb that presses the keys.
use super::ioe;
use serde_json::Value;
use std::{
    env, fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

type Checks = Vec<(&'static str, Result<(), String>)>;

pub fn run(root: &Path) -> Result<(), String> {
    let au = env::var("AU_BIN").unwrap_or_else(|_| root.join("target/dist/au").display().to_string());
    let name = env::var("E2E_NAME").unwrap_or_else(|_| "au-e2e".into());
    let home = root.join("target/e2e-home");
    fs::create_dir_all(&home).map_err(ioe)?;
    let outcome = body(root, &au, &name, &home);
    // Whatever happened above, do not leave the emulator running (or, when asked, behind).
    let finish = if env::var("E2E_DESTROY").is_ok_and(|v| v == "1") { vec!["emulator", "destroy", name.as_str(), "--yes"] } else { vec!["emulator", "stop", name.as_str()] };
    let _ = Command::new(&au).args(&finish).env("AU_HOME", &home).stdin(Stdio::null()).output();
    let checks = outcome?;
    let mut failed = 0;
    for (what, result) in &checks {
        match result {
            Ok(()) => println!("pass  {what}"),
            Err(why) => {
                failed += 1;
                println!("FAIL  {what}: {why}");
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} of {} emulator checks failed", checks.len()));
    }
    println!("emulator-e2e: {} checks passed", checks.len());
    Ok(())
}

fn body(root: &Path, au: &str, name: &str, home: &Path) -> Result<Checks, String> {
    let apk = root.join("device/app/build/outputs/apk/debug/app-debug.apk");
    let au_run = |args: &[&str]| -> Result<Value, String> {
        let out = Command::new(au).args(args).env("AU_HOME", home).env("AU_DEV_APK", "1").stdin(Stdio::null()).output().map_err(|e| format!("could not run {au}: {e}"))?;
        serde_json::from_slice(&out.stdout).map_err(|_| format!("au {} printed no JSON: {}", args.join(" "), String::from_utf8_lossy(&out.stderr)))
    };
    let ok = |value: Value, what: &str| -> Result<Value, String> {
        if value["ok"] == 1 {
            Ok(value)
        } else {
            Err(format!("{what}: {value}"))
        }
    };
    let mut checks: Checks = Vec::new();

    ok(au_run(&["emulator", "doctor"])?, "emulator doctor (run au emulator install)")?;
    if au_run(&["emulator", "status", name])?["ok"] != 1 {
        ok(au_run(&["emulator", "create", name, "--profile", "small"])?, "create")?;
    }
    let start = |window: bool| -> Result<(), String> {
        let mut args = vec!["emulator", "start", name];
        if window {
            args.push("--window");
        }
        ok(au_run(&args)?, "start").map(|_| ())
    };
    let window = env::var("E2E_WINDOW").is_ok_and(|v| v == "1");
    start(window)?;
    let setup = au_run(&["--target", name, "setup", &apk.display().to_string(), "--no-agents"])?;
    ok(setup.clone(), "setup")?;
    let begun = Instant::now();
    while !au_run(&["--target", name, "read", "status"])?["text"].as_str().is_some_and(|t| t.starts_with("Ready")) {
        if begun.elapsed() > Duration::from_secs(300) {
            return Err("Android-Use is not enabled; turn it on under Settings → Accessibility on the emulator (E2E_WINDOW=1 shows it) and run again".into());
        }
        println!("waiting: turn on Android-Use under Settings → Accessibility on the emulator");
        thread::sleep(Duration::from_secs(5));
    }

    let on = |args: &[&str]| -> Result<Value, String> {
        let mut full = vec!["--target", name];
        full.extend_from_slice(args);
        au_run(&full)
    };
    checks.push((
        "reads the screen",
        ok(on(&["read", "screen"])?, "read screen").and_then(|v| v["text"].as_str().filter(|t| !t.is_empty()).map(|_| ()).ok_or("empty screen".into())),
    ));
    checks.push((
        "acts on the screen",
        on(&["act", "open app \"Settings\""]).and_then(|v| v["text"].as_str().filter(|t| t.starts_with("Done")).map(|_| ()).ok_or(format!("unexpected: {v}"))),
    ));
    checks.push(("lists the target", au_run(&["targets"]).and_then(|v| if v.to_string().contains(name) { Ok(()) } else { Err(v.to_string()) })));

    // Preflight: each unusable state comes back as its own typed result, not a timeout. Every case
    // is undone, and the helper must report Ready again, before the next one.
    let adb = env::var("AU_ADB").unwrap_or_else(|_| "adb".into());
    let shell = |args: &[&str]| {
        let _ = Command::new(&adb).args(["-e", "shell"]).args(args).stdin(Stdio::null()).output();
    };
    let typed = |setup: &[&[&str]], undo: &[&[&str]], code: &str| -> Result<(), String> {
        setup.iter().for_each(|step| shell(step));
        thread::sleep(Duration::from_secs(2));
        let got = on(&["act", "tap \"Wi-Fi\""]);
        undo.iter().for_each(|step| shell(step));
        let begun = Instant::now();
        while begun.elapsed() < Duration::from_secs(60) && !on(&["read", "status"]).is_ok_and(|v| v["text"].as_str().is_some_and(|t| t.starts_with("Ready"))) {
            thread::sleep(Duration::from_secs(3));
        }
        got.and_then(|v| if v["e"] == code && v["m"].as_str().is_some_and(|m| !m.is_empty()) { Ok(()) } else { Err(v.to_string()) })
    };
    let sleep: &[&str] = &["input", "keyevent", "KEYCODE_SLEEP"];
    let wake: &[&str] = &["input", "keyevent", "KEYCODE_WAKEUP"];
    let unlock: &[&str] = &["wm", "dismiss-keyguard"];
    checks.push(("reports a screen that is off", typed(&[sleep], &[wake, unlock], "screen_off")));
    checks.push(("reports a locked phone", typed(&[sleep, wake], &[unlock], "locked")));
    let (off, on_again): (&[&str], &[&str]) = (&["pm", "disable-user", "--user", "0", "dev.codex.aubridge"], &["pm", "enable", "dev.codex.aubridge"]);
    checks.push(("reports a helper that is not running", typed(&[off, &["am", "force-stop", "dev.codex.aubridge"]], &[on_again], "helper_killed")));

    let service: &[&str] = &["am", "start-foreground-service", "-n", "dev.codex.aubridge/.BridgeService"];
    let stop: Vec<&str> = [service, &["-a", "dev.codex.aubridge.action.STOP"]].concat();
    let resume: Vec<&str> = [service, &["--ez", "resume", "true"]].concat();
    checks.push(("reports a helper stopped from its notification", typed(&[&stop], &[&resume], "stopped")));

    // A second serve for the same target must be refused while the first holds the lock.
    let mut first = Command::new(au).args(["--target", name, "serve", "--mcp"]).env("AU_HOME", home).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(ioe)?;
    thread::sleep(Duration::from_secs(3));
    let second = Command::new(au).args(["--target", name, "serve", "--mcp"]).env("AU_HOME", home).stdin(Stdio::null()).output().map_err(ioe)?;
    let refused = String::from_utf8_lossy(&second.stdout).contains("already driving");
    let _ = first.kill();
    let _ = first.wait();
    checks.push(("refuses a second serve", if refused { Ok(()) } else { Err("a second serve was not refused".into()) }));

    // The enrollment must survive a stop and start.
    ok(au_run(&["emulator", "stop", name])?, "stop")?;
    start(window)?;
    // The service rebinds a few seconds after boot; give it up to a minute.
    let begun = Instant::now();
    let mut again = on(&["read", "status"]);
    while again.as_ref().map_or(true, |v| !v["text"].as_str().is_some_and(|t| t.starts_with("Ready"))) && begun.elapsed() < Duration::from_secs(60) {
        thread::sleep(Duration::from_secs(3));
        again = on(&["read", "status"]);
    }
    checks
        .push(("keeps its enrollment across a restart", again.and_then(|v| if v["text"].as_str().is_some_and(|t| t.starts_with("Ready")) { Ok(()) } else { Err(v.to_string()) })));

    checks.push((
        "saves and restores a snapshot",
        ok(au_run(&["emulator", "snapshot", name, "save", "e2e"])?, "snapshot save")
            .and_then(|_| ok(au_run(&["emulator", "snapshot", name, "load", "e2e"])?, "snapshot load"))
            .map(|_| ()),
    ));

    Ok(checks)
}
