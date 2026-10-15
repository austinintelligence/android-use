//! `au uninstall` flags: `--dry-run` only reports, unknown flags fail, and uninstall refuses while
//! `au serve` holds the state lock. Each test runs the real binary against a private state folder,
//! a private home, and (where adb is reached) a fake adb that logs every call.

use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

/// A state folder with an enrolled device, a journal, and an artifact, plus a home folder.
fn sandbox() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let (state, home) = (dir.path().join("state"), dir.path().join("home"));
    fs::create_dir_all(state.join("artifacts")).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::write(state.join("device.json"), r#"{"hardware":"SERIAL123","endpoint":"SERIAL123"}"#).unwrap();
    fs::write(state.join("operations.jsonl"), "[]\n").unwrap();
    fs::write(state.join("artifacts/capture.png"), b"png").unwrap();
    (dir, state, home)
}

/// Writes a fake adb: `devices` lists one device, any `getprop` answers its serial, and every
/// call is appended to the log so a test can prove what was (and was not) run.
#[cfg(unix)]
fn fake_adb(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-adb");
    fs::write(
        &path,
        "#!/bin/sh\necho \"$*\" >> \"$FAKE_ADB_LOG\"\ncase \"$*\" in\n  devices) printf 'List of devices attached\\nSERIAL123\\tdevice\\n' ;;\n  *getprop*) echo SERIAL123 ;;\nesac\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn au(state: &Path, home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_au"))
        .args(args)
        .env("AU_HOME", state)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env_remove("ANDROID_HOME")
        .env_remove("ANDROID_SDK_ROOT")
        .env_remove("AU_TARGET")
        .output()
        .unwrap()
}

fn json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("stdout was not JSON: {}", String::from_utf8_lossy(&output.stdout)))
}

#[test]
fn an_unknown_flag_is_an_error_that_names_the_flag_and_changes_nothing() {
    let (_dir, state, home) = sandbox();
    let output = au(&state, &home, &["uninstall", "--bogus"]);
    assert!(!output.status.success(), "an unknown flag must exit non-zero");
    let message = json(&output)["m"].as_str().unwrap_or_default().to_owned();
    assert!(message.contains("--bogus") && message.contains("--dry-run"), "message should name the flag and the valid ones: {message}");
    assert!(state.join("device.json").exists() && state.join("artifacts/capture.png").exists(), "a rejected command removes nothing");
}

#[test]
fn an_unknown_flag_on_a_named_agent_is_an_error_too() {
    let (_dir, state, home) = sandbox();
    let output = au(&state, &home, &["uninstall", "cursor", "--bogus"]);
    assert!(!output.status.success());
    let message = json(&output)["m"].as_str().unwrap_or_default().to_owned();
    assert!(message.contains("--bogus") && message.contains("--force"), "message should name the flag and the valid ones: {message}");
}

#[test]
fn a_foreign_client_entry_is_kept_until_uninstall_is_given_force() {
    let (_dir, state, home) = sandbox();
    let cursor = home.join(".cursor/mcp.json");
    fs::create_dir_all(cursor.parent().unwrap()).unwrap();
    fs::write(&cursor, r#"{"mcpServers":{"android-use":{"command":"node","args":["/x/server.js"]}}}"#).unwrap();

    // Agent commands report per-agent results with exit 0, so the refusal is in the JSON, not the status.
    let refused = au(&state, &home, &["uninstall", "cursor"]);
    assert_eq!(json(&refused)["ok"], 0, "a foreign entry is refused without --force");
    assert!(String::from_utf8_lossy(&refused.stdout).contains("--force"), "the refusal says to pass --force");
    assert!(fs::read_to_string(&cursor).unwrap().contains("/x/server.js"));

    let forced = au(&state, &home, &["uninstall", "cursor", "--force"]);
    assert_eq!(json(&forced)["ok"], 1, "--force removes the foreign entry: {}", String::from_utf8_lossy(&forced.stdout));
    assert!(!fs::read_to_string(&cursor).unwrap().contains("/x/server.js"));
}

#[test]
fn dry_run_needs_no_device_and_creates_nothing() {
    let dir = TempDir::new().unwrap();
    let (state, home) = (dir.path().join("state"), dir.path().join("home"));
    fs::create_dir_all(&home).unwrap();
    let output = au(&state, &home, &["uninstall", "--dry-run"]);
    assert!(output.status.success(), "dry run without a device should still report: {}", String::from_utf8_lossy(&output.stdout));
    assert_eq!(json(&output)["dry_run"], 1);
    assert!(!state.exists(), "dry run must not create the state folder or its artifacts folder");
}

#[test]
fn uninstall_is_refused_while_au_serve_holds_the_state_lock() {
    let (_dir, state, home) = sandbox();
    // Stand in for a running `au serve`: it holds the same exclusive lock on the target's state.
    let serving = fs::OpenOptions::new().create(true).truncate(false).write(true).open(state.join("lock")).unwrap();
    serving.try_lock().unwrap();

    let output = au(&state, &home, &["uninstall"]);
    assert!(!output.status.success(), "uninstall must refuse while serve runs");
    let message = json(&output)["m"].as_str().unwrap_or_default().to_owned();
    assert!(message.contains("au serve") && message.contains("stop"), "message should say to stop the server first: {message}");
    assert!(state.join("device.json").exists(), "a refused uninstall removes nothing");
    drop(serving);
}

#[cfg(unix)]
#[test]
fn dry_run_reports_what_would_go_and_changes_state_client_config_and_device_not_at_all() {
    let (dir, state, home) = sandbox();
    let adb_log = dir.path().join("adb.log");
    let adb = fake_adb(dir.path());
    let cursor = home.join(".cursor/mcp.json");
    fs::create_dir_all(cursor.parent().unwrap()).unwrap();
    let config = r#"{"mcpServers":{"android-use":{"command":"au","args":["serve","--mcp"]}}}"#;
    fs::write(&cursor, config).unwrap();

    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_au"))
            .args(args)
            .env("AU_HOME", &state)
            .env("HOME", &home)
            .env("AU_ADB", &adb)
            .env("FAKE_ADB_LOG", &adb_log)
            .env_remove("ANDROID_HOME")
            .env_remove("ANDROID_SDK_ROOT")
            .output()
            .unwrap()
    };

    let output = run(&["uninstall", "--dry-run"]);
    assert!(output.status.success(), "dry run should succeed: {}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(json(&output)["dry_run"], 1);
    let listed = json(&output)["would_remove"].to_string();
    assert!(listed.contains("dev.codex.aubridge") && listed.contains("SERIAL123"), "dry run names the helper and device: {listed}");

    let agent = run(&["uninstall", "cursor", "--dry-run"]);
    assert!(agent.status.success());
    assert_eq!(fs::read_to_string(&cursor).unwrap(), config, "dry run leaves the client configuration alone");

    assert!(state.join("device.json").exists() && state.join("operations.jsonl").exists() && state.join("artifacts/capture.png").exists(), "dry run leaves local state alone");
    let adb_calls = fs::read_to_string(&adb_log).unwrap_or_default();
    assert!(!adb_calls.contains("uninstall"), "dry run must not uninstall the helper: {adb_calls}");
}
