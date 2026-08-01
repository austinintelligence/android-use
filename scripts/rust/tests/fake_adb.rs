#![cfg(windows)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use android_use::adb::{shell_quote, Adb};
use android_use::config::Config;
use android_use::persistent::ShellPool;
use std::time::Duration;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn compile_fake_adb(root: &Path) -> PathBuf {
    let source = root.join("fake_adb.rs");
    let binary = root.join("fake-adb.exe");
    fs::write(
        &source,
        r#"
use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if let Ok(path) = env::var("AU_FAKE_ADB_LOG") {
        let mut log = String::new();
        for argument in &args {
            log.push_str(&format!("{}:{}\n", argument.len(), argument));
        }
        fs::write(path, log).unwrap();
    }
    match env::var("AU_FAKE_ADB_MODE").as_deref() {
        _ if args.iter().any(|argument| argument == "shell")
            && matches!(
                env::var("AU_FAKE_ADB_MODE").as_deref(),
                Ok("interactive") | Ok("interactive-huge") | Ok("interactive-desync")
            ) =>
        {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            let mut line = String::new();
            let mut output = io::stdout().lock();
            loop {
                line.clear();
                if input.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let Some(start) = line.find("\\036AU:") else {
                    continue;
                };
                let marker_body = &line[start + "\\036AU:".len()..];
                let Some(end) = marker_body.find(":%s\\037") else {
                    continue;
                };
                let Some((nonce, sequence)) = marker_body[..end].split_once(':') else {
                    continue;
                };
                if env::var("AU_FAKE_ADB_MODE").as_deref() == Ok("interactive-huge") {
                    for _ in 0..48 {
                        output.write_all(&[b'x'; 8192]).unwrap();
                    }
                }
                let status = if env::var("AU_FAKE_ADB_MODE").as_deref() == Ok("interactive-desync") {
                    "bad"
                } else {
                    "0"
                };
                write!(output, "\x1eAU:{nonce}:{sequence}:{status}\x1f").unwrap();
                output.flush().unwrap();
            }
            return;
        }
        Ok("sleep") => {
            thread::sleep(Duration::from_secs(2));
            return;
        }
        Ok("huge") => {
            let mut out = io::stdout().lock();
            for _ in 0..512 {
                out.write_all(&[b'x'; 8192]).unwrap();
            }
            return;
        }
        _ => {}
    }
    if args.iter().any(|argument| argument == "exec-out") {
        io::stdout().write_all(b"\x89PNG\r\n\x1a\nAU").unwrap();
    } else {
        println!("fake-ok");
    }
}
"#,
    )
    .expect("write fake adb source");
    let status = Command::new("rustc")
        .args([
            source.as_os_str(),
            "-O".as_ref(),
            "-o".as_ref(),
            binary.as_os_str(),
        ])
        .status()
        .expect("run rustc");
    assert!(status.success(), "compile fake adb");
    binary
}

#[test]
fn fake_adb_preserves_boundaries_and_enforces_streaming_limits() {
    let _environment = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let root = tempfile::tempdir().expect("temporary fake adb root");
    let fake = compile_fake_adb(root.path());
    let log = root.path().join("args.log");
    env::set_var("AU_FAKE_ADB_LOG", &log);
    env::remove_var("AU_FAKE_ADB_MODE");

    let config = Config {
        adb_path: Some(fake),
        ..Config::default()
    };
    let adb = Adb::from_config(&config, 2_000).expect("fake adb client");
    let injected = "https://example.invalid/?q=one&echo PWNED;$(id)".to_owned();
    let command = vec![
        "shell".into(),
        "am".into(),
        "start".into(),
        "-d".into(),
        injected.clone(),
    ];
    adb.device("TEST-DEVICE", &command)
        .expect("bounded fake command");
    let observed = fs::read_to_string(&log).expect("fake argument log");
    let structured = format!("'am' 'start' '-d' {}", shell_quote(&injected));
    assert_eq!(
        observed.lines().collect::<Vec<_>>(),
        [
            "2:-s",
            "11:TEST-DEVICE",
            "5:shell",
            &format!("{}:{structured}", structured.len()),
        ]
    );

    adb.raw_shell("TEST-DEVICE", &command[1..])
        .expect("raw shell command");
    let raw_observed = fs::read_to_string(&log).expect("raw fake argument log");
    assert_eq!(
        raw_observed.lines().collect::<Vec<_>>(),
        [
            "2:-s",
            "11:TEST-DEVICE",
            "5:shell",
            "2:am",
            "5:start",
            "2:-d",
            &format!("{}:{injected}", injected.len()),
        ]
    );

    let png = root.path().join("capture.png");
    let capture = adb
        .device_to_file(
            "TEST-DEVICE",
            &["exec-out".into(), "screencap".into(), "-p".into()],
            png.clone(),
        )
        .expect("binary capture");
    assert!(capture.stdout.bytes.is_empty());
    assert_eq!(fs::read(&png).expect("png bytes"), b"\x89PNG\r\n\x1a\nAU");

    env::set_var("AU_FAKE_ADB_MODE", "huge");
    let huge = adb
        .device("TEST-DEVICE", &["shell".into(), "huge".into()])
        .expect("huge output");
    assert!(huge.stdout.truncated);
    assert!(huge.stdout.total_bytes > huge.stdout.bytes.len() as u64);

    env::set_var("AU_FAKE_ADB_MODE", "sleep");
    let timeout_adb = Adb::from_config(&config, 60).expect("short fake adb client");
    let timeout = timeout_adb
        .device("TEST-DEVICE", &["shell".into(), "hang".into()])
        .expect_err("hung fake adb must time out");
    assert_eq!(timeout.kind(), "E_TIMEOUT");

    env::remove_var("AU_FAKE_ADB_LOG");
    env::remove_var("AU_FAKE_ADB_MODE");
}

#[test]
fn fake_adb_persistent_shell_frames_and_bounds_output() {
    let _environment = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let root = tempfile::tempdir().expect("temporary fake adb root");
    let fake = compile_fake_adb(root.path());
    let config = Config {
        adb_path: Some(fake),
        ..Config::default()
    };
    let adb = Adb::from_config(&config, 2_000).expect("fake adb client");

    env::set_var("AU_FAKE_ADB_MODE", "interactive");
    let mut pool = ShellPool::new(adb.clone());
    let reply = pool
        .transact("TEST-DEVICE", "printf stable", Duration::from_secs(2))
        .expect("framed persistent shell reply");
    assert!(reply.stdout.is_empty());

    env::set_var("AU_FAKE_ADB_MODE", "interactive-desync");
    let mut desync = ShellPool::new(adb.clone());
    let error = desync
        .transact("TEST-DEVICE", "printf desync", Duration::from_secs(2))
        .expect_err("malformed persistent shell frame");
    assert_eq!(error.kind(), "E_SHELL");

    env::set_var("AU_FAKE_ADB_MODE", "interactive-huge");
    let mut huge = ShellPool::new(adb);
    let error = huge
        .transact("TEST-DEVICE", "printf huge", Duration::from_secs(2))
        .expect_err("oversized persistent shell output");
    assert_eq!(error.kind(), "E_OUTPUT_LIMIT");
    env::remove_var("AU_FAKE_ADB_MODE");
}
