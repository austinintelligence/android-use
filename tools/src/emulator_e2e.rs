//! `cargo xtask emulator-e2e`: a repeatable end-to-end check of the emulator target workflow on one
//! managed emulator, using only the shipped `au` commands.
//!
//! It needs `cargo xtask android` (the debug helper APK), a built `au` (`AU_BIN`, default
//! `target/dist/au`), and the Android SDK (`au emulator install` fetches what is missing). The first
//! run asks you to turn on Android-Use under Settings → Accessibility on the emulator, which Android
//! requires you to do yourself; later runs reuse that setting. State lives under `target/e2e-home`.
//!
//! Environment: `E2E_NAME` (default `au-e2e`), `E2E_WINDOW=1` to show the emulator window,
//! `E2E_DESTROY=1` to delete the emulator afterwards.
use super::ioe;
use serde_json::Value;
use std::{
    env, fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn run(root: &Path) -> Result<(), String> {
    let au = env::var("AU_BIN").unwrap_or_else(|_| root.join("target/dist/au").display().to_string());
    let apk = root.join("device/app/build/outputs/apk/debug/app-debug.apk");
    let name = env::var("E2E_NAME").unwrap_or_else(|_| "au-e2e".into());
    let home = root.join("target/e2e-home");
    fs::create_dir_all(&home).map_err(ioe)?;
    let au_run = |args: &[&str]| -> Result<Value, String> {
        let out = Command::new(&au).args(args).env("AU_HOME", &home).stdin(Stdio::null()).output().map_err(|e| format!("could not run {au}: {e}"))?;
        serde_json::from_slice(&out.stdout).map_err(|_| format!("au {} printed no JSON: {}", args.join(" "), String::from_utf8_lossy(&out.stderr)))
    };
    let ok = |value: Value, what: &str| -> Result<Value, String> {
        if value["ok"] == 1 {
            Ok(value)
        } else {
            Err(format!("{what}: {value}"))
        }
    };
    let mut checks: Vec<(&str, Result<(), String>)> = Vec::new();

    ok(au_run(&["emulator", "doctor"])?, "emulator doctor (run au emulator install)")?;
    if au_run(&["emulator", "status", &name])?["ok"] != 1 {
        ok(au_run(&["emulator", "create", &name, "--profile", "small"])?, "create")?;
    }
    let start = |window: bool| -> Result<(), String> {
        let mut args = vec!["emulator", "start", name.as_str()];
        if window {
            args.push("--window");
        }
        ok(au_run(&args)?, "start").map(|_| ())
    };
    let window = env::var("E2E_WINDOW").is_ok_and(|v| v == "1");
    start(window)?;
    let setup = au_run(&["--target", &name, "setup", &apk.display().to_string(), "--no-agents"])?;
    ok(setup.clone(), "setup")?;
    let begun = Instant::now();
    while !au_run(&["--target", &name, "read", "status"])?["text"].as_str().is_some_and(|t| t.starts_with("Ready")) {
        if begun.elapsed() > Duration::from_secs(300) {
            return Err("Android-Use is not enabled; turn it on under Settings → Accessibility on the emulator (E2E_WINDOW=1 shows it) and run again".into());
        }
        println!("waiting: turn on Android-Use under Settings → Accessibility on the emulator");
        thread::sleep(Duration::from_secs(5));
    }

    let on = |args: &[&str]| -> Result<Value, String> {
        let mut full = vec!["--target", name.as_str()];
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
    checks.push(("lists the target", au_run(&["targets"]).and_then(|v| if v.to_string().contains(&name) { Ok(()) } else { Err(v.to_string()) })));

    // A second serve for the same target must be refused while the first holds the lock.
    let mut first = Command::new(&au).args(["--target", &name, "serve", "--mcp"]).env("AU_HOME", &home).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(ioe)?;
    thread::sleep(Duration::from_secs(3));
    let second = Command::new(&au).args(["--target", &name, "serve", "--mcp"]).env("AU_HOME", &home).stdin(Stdio::null()).output().map_err(ioe)?;
    let refused = String::from_utf8_lossy(&second.stdout).contains("already driving");
    let _ = first.kill();
    let _ = first.wait();
    checks.push(("refuses a second serve", if refused { Ok(()) } else { Err("a second serve was not refused".into()) }));

    // The enrollment must survive a stop and start.
    ok(au_run(&["emulator", "stop", &name])?, "stop")?;
    start(window)?;
    let again = on(&["read", "status"]);
    checks
        .push(("keeps its enrollment across a restart", again.and_then(|v| if v["text"].as_str().is_some_and(|t| t.starts_with("Ready")) { Ok(()) } else { Err(v.to_string()) })));

    checks.push((
        "saves and restores a snapshot",
        ok(au_run(&["emulator", "snapshot", &name, "save", "e2e"])?, "snapshot save")
            .and_then(|_| ok(au_run(&["emulator", "snapshot", &name, "load", "e2e"])?, "snapshot load"))
            .map(|_| ()),
    ));

    if env::var("E2E_DESTROY").is_ok_and(|v| v == "1") {
        ok(au_run(&["emulator", "destroy", &name, "--yes"])?, "destroy")?;
    } else {
        ok(au_run(&["emulator", "stop", &name])?, "stop")?;
    }
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
