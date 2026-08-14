#![forbid(unsafe_code)]

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn main() {
    if let Err(e) = run() {
        eprintln!("tools: {e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), String> {
    let root = root()?;
    match env::args().nth(1).as_deref().unwrap_or("help") {
        "check" => {
            cmd(&root, "cargo", &["fmt", "--all", "--", "--check"])?;
            cmd(&root, "cargo", &["clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"])?;
            gate(&root)
        }
        "test" => {
            cmd(&root, "cargo", &["test", "--workspace", "--all-targets"])?;
            android(&root, "testDebugUnitTest")
        }
        "android" => android(&root, "assembleDebug"),
        "package" => package(&root),
        "release" => {
            verify(&root)?;
            package(&root)
        }
        "manifest" => manifest(&root),
        "verify" => verify(&root),
        "benchmark" => benchmark(&root),
        "benchmark-live" => benchmark_live(&root),
        "stress-live" => stress_live(&root),
        "live" => live(&root),
        "size" => size(&root).map(|_| ()),
        "docs" => docs(&root),
        _ => {
            println!("cargo xtask check|test|android|package|release|manifest|verify|benchmark|benchmark-live|stress-live|live|size|docs");
            Ok(())
        }
    }
}
fn verify(root: &Path) -> Result<(), String> {
    cmd(root, "cargo", &["fmt", "--all", "--", "--check"])?;
    cmd(root, "cargo", &["clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"])?;
    cmd(root, "cargo", &["test", "--workspace", "--all-targets"])?;
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    cmd(root, npm, &["test"])?;
    cmd(root, npm, &["run", "lint"])?;
    android(root, "testDebugUnitTest")?;
    android(root, "assembleRelease")?;
    gate(root)?;
    docs(root)?;
    cmd(root, "cargo", &["build", "--profile", "dist", "-p", "au"])?;
    size(root).map(|_| ())
}
fn package(root: &Path) -> Result<(), String> {
    cmd(root, "cargo", &["build", "--profile", "dist", "-p", "au"])?;
    android(root, "assembleRelease")?;
    let dist = root.join("dist");
    fs::create_dir_all(&dist).map_err(ioe)?;
    let exe = if cfg!(windows) { "au.exe" } else { "au" };
    copy(&root.join("target/dist").join(exe), &dist.join(exe))?;
    let apk = release_apk(root)?;
    copy(&apk, &dist.join("aubridge.apk"))?;
    copy(&root.join("skills/android-use/SKILL.md"), &dist.join("SKILL.md"))?;
    let bundle = dist.join("npm/android-use");
    copy_tree(&root.join("install"), &bundle)?;
    let bin = bundle.join("bin").join(platform_name());
    fs::create_dir_all(&bin).map_err(ioe)?;
    copy(&root.join("target/dist").join(exe), &bin.join(exe))?;
    copy(&apk, &bin.join("aubridge.apk"))?;
    let files = json!({
        format!("bin/{}/{}", platform_name(), exe): hex(&Sha256::digest(fs::read(bin.join(exe)).map_err(ioe)?)),
        format!("bin/{}/aubridge.apk", platform_name()): hex(&Sha256::digest(fs::read(bin.join("aubridge.apk")).map_err(ioe)?)),
    });
    fs::write(bundle.join("manifest.json"), serde_json::to_vec(&json!({"schema":1,"version":env!("CARGO_PKG_VERSION"),"files":files})).map_err(|e| e.to_string())?).map_err(ioe)?;
    manifest(root)
}
fn manifest(root: &Path) -> Result<(), String> {
    let dist = root.join("dist");
    let mut assets = serde_json::Map::new();
    for e in fs::read_dir(&dist).map_err(ioe)? {
        let p = e.map_err(ioe)?.path();
        if p.file_name().and_then(|s| s.to_str()) == Some("manifest.json") || !p.is_file() {
            continue;
        }
        let b = fs::read(&p).map_err(ioe)?;
        assets.insert(p.file_name().unwrap().to_string_lossy().into_owned(), json!({"bytes":b.len(),"sha256":hex(&Sha256::digest(&b))}));
    }
    fs::write(dist.join("manifest.json"), serde_json::to_vec(&json!({"schema":3,"version":env!("CARGO_PKG_VERSION"),"assets":assets})).map_err(|e| e.to_string())?).map_err(ioe)
}
fn benchmark(root: &Path) -> Result<(), String> {
    cmd(root, "cargo", &["build", "--release", "-p", "au"])?;
    let exe = root.join("target/release").join(if cfg!(windows) { "au.exe" } else { "au" });
    let mut samples = Vec::with_capacity(40);
    for _ in 0..40 {
        let start = Instant::now();
        let s = Command::new(&exe).arg("--version").stdout(Stdio::null()).status().map_err(ioe)?;
        if !s.success() {
            return Err("benchmark command failed".into());
        }
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    println!("startup_p50_us={}", quantile(&samples, 0.50));
    println!("startup_p95_us={}", quantile(&samples, 0.95));
    println!("startup_p99_us={}", quantile(&samples, 0.99));
    println!("binary_bytes={}", fs::metadata(&exe).map_err(ioe)?.len());
    println!("apk_bytes={}", file_bytes(&release_apk(root)?)?);
    println!("installer_bytes={}", tree_bytes(&root.join("install"))?);
    let golden: Value = serde_json::from_str(&fs::read_to_string(root.join("protocol-golden.json")).map_err(ioe)?).map_err(|e| e.to_string())?;
    for key in ["frame", "success", "stale"] {
        let bytes = serde_json::to_vec(golden.get(key).ok_or_else(|| format!("golden fixture is missing {key}"))?).map_err(|e| e.to_string())?;
        println!("{key}_bytes={}", bytes.len());
    }
    Ok(())
}

struct JsonlSession {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl JsonlSession {
    fn start(exe: &Path) -> Result<Self, String> {
        let mut child = Command::new(exe).args(["serve", "--jsonl"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(ioe)?;
        let input = child.stdin.take().ok_or_else(|| "JSONL stdin was unavailable".to_owned())?;
        let output = child.stdout.take().ok_or_else(|| "JSONL stdout was unavailable".to_owned())?;
        Ok(Self { child, input: Some(input), output: BufReader::new(output) })
    }

    fn request(&mut self, value: Value) -> Result<(Value, usize, usize, u128), String> {
        let request = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        let started = Instant::now();
        let input = self.input.as_mut().ok_or_else(|| "JSONL stdin was closed".to_owned())?;
        input.write_all(&request).map_err(ioe)?;
        input.write_all(b"\n").map_err(ioe)?;
        input.flush().map_err(ioe)?;
        let mut response = Vec::new();
        self.output.read_until(b'\n', &mut response).map_err(ioe)?;
        if response.is_empty() {
            return Err("JSONL server closed before replying".into());
        }
        if response.len() > 1_048_576 {
            return Err("JSONL response exceeded 1 MiB".into());
        }
        while response.last().is_some_and(u8::is_ascii_whitespace) {
            response.pop();
        }
        let parsed = serde_json::from_slice(&response).map_err(|e| format!("JSONL response was invalid: {e}"))?;
        Ok((parsed, request.len(), response.len(), started.elapsed().as_micros()))
    }
}

impl Drop for JsonlSession {
    fn drop(&mut self) {
        self.input.take();
        for _ in 0..40 {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn benchmark_live(root: &Path) -> Result<(), String> {
    let exe = root.join("target/dist").join(if cfg!(windows) { "au.exe" } else { "au" });
    if !exe.is_file() {
        return Err("build target/dist/au before running the live benchmark".into());
    }
    let mut session = JsonlSession::start(&exe)?;
    let mut request_bytes = 0usize;
    let mut response_bytes = 0usize;
    let mut round_trips = 0usize;
    let suffix = format!("bench-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis());
    let mut call = |name: &str, request: Value| -> Result<Value, String> {
        let (value, sent, received, elapsed) = session.request(request)?;
        request_bytes = request_bytes.saturating_add(sent);
        response_bytes = response_bytes.saturating_add(received);
        round_trips += 1;
        println!("live_{name}_us={elapsed} live_{name}_request_bytes={sent} live_{name}_response_bytes={received} live_{name}_response_tokens={}", approx_tokens(received));
        Ok(value)
    };
    let status = call("status", json!({"tool":"android.read","arguments":{"q":"status"}}))?;
    require_ok(&status, "live benchmark status")?;
    let first = call("observe_cold", json!({"tool":"android.read","arguments":{"q":"observe"}}))?;
    let first_generation = generation(&first)?;
    for i in 0..4 {
        let warm = call(&format!("observe_warm_{i}"), json!({"tool":"android.read","arguments":{"q":"observe","base":first_generation.to_string()}}))?;
        if warm.get("=").and_then(Value::as_u64) != Some(1) && warm.get("g").and_then(Value::as_u64) != Some(first_generation) {
            return Err("warm observe did not preserve the generation".into());
        }
    }
    let settings =
        call("launch_settings", json!({"tool":"android.act","arguments":{"id":format!("{suffix}-settings"),"g":first_generation,"p":[["launch","com.android.settings"]]}}))?;
    require_ok(&settings, "live benchmark launch Settings")?;
    let after_settings = call("observe_after_launch", json!({"tool":"android.read","arguments":{"q":"observe"}}))?;
    require_ok(
        &call("launch_chrome", json!({"tool":"android.act","arguments":{"id":format!("{suffix}-chrome"),"g":generation(&after_settings)?,"p":[["launch","com.android.chrome"]]}}))?,
        "live benchmark launch Chrome",
    )?;
    let tabs = call("browser_tabs", json!({"tool":"android.read","arguments":{"q":"browser","op":"tabs"}}))?;
    let browser_generation = generation(&tabs)?;
    let navigate = call(
        "browser_navigate",
        json!({"tool":"android.act","arguments":{"target":"browser","id":format!("{suffix}-page"),"g":browser_generation,"deadline_ms":15000,"p":[["navigate","https://example.com"],["wait",["text","Example Domain"],10000]]}}),
    )?;
    require_ok(&navigate, "live benchmark navigate Example Domain")?;
    let page = call("browser_text", json!({"tool":"android.read","arguments":{"q":"browser","op":"text"}}))?;
    if !page.get("text").and_then(Value::as_str).unwrap_or("").contains("Example Domain") {
        return Err("live benchmark did not verify Example Domain".into());
    }
    let mut elapsed = Vec::new();
    for i in 0..3 {
        let started = Instant::now();
        let _ = call(&format!("browser_text_warm_{i}"), json!({"tool":"android.read","arguments":{"q":"browser","op":"text"}}))?;
        elapsed.push(started.elapsed().as_micros());
    }
    elapsed.sort_unstable();
    println!("live_round_trips={round_trips} live_request_bytes={request_bytes} live_response_bytes={response_bytes} live_response_tokens={} live_browser_text_warm_p50_us={} live_browser_text_warm_p95_us={}", approx_tokens(response_bytes), quantile(&elapsed, 0.50), quantile(&elapsed, 0.95));
    Ok(())
}

fn approx_tokens(bytes: usize) -> usize {
    bytes.saturating_add(3) / 4
}
fn stress_call(session: &mut JsonlSession, request: Value, samples: &mut Vec<u128>) -> Result<Value, String> {
    let (value, _, _, elapsed) = session.request(request)?;
    samples.push(elapsed);
    Ok(value)
}
fn stress_retryable(value: &Value) -> bool {
    matches!(value.get("e").and_then(Value::as_str), Some("stale" | "helper" | "timeout"))
}
fn stress_launch(session: &mut JsonlSession, package: &str, id: &str, actions: &mut Vec<u128>, observes: &mut Vec<u128>) -> Result<u64, String> {
    for attempt in 0..4 {
        let current = stress_call(session, json!({"tool":"android.read","arguments":{"q":"observe"}}), observes)?;
        let g = generation(&current)?;
        let result = stress_call(session, json!({"tool":"android.act","arguments":{"id":format!("{id}-{attempt}"),"g":g,"p":[["launch",package]]}}), actions)?;
        if result.get("ok").and_then(Value::as_u64) == Some(1) {
            return Ok(result.get("g").and_then(Value::as_u64).unwrap_or(g));
        }
        if !stress_retryable(&result) {
            return Err(format!("launch {package} failed: {result}"));
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err(format!("launch {package} stayed stale after retries"))
}
fn stress_navigate(session: &mut JsonlSession, id: &str, samples: &mut Vec<u128>) -> Result<(), String> {
    for attempt in 0..4 {
        let tabs = stress_call(session, json!({"tool":"android.read","arguments":{"q":"browser","op":"tabs"}}), samples)?;
        let g = generation(&tabs)?;
        let result = stress_call(
            session,
            json!({"tool":"android.act","arguments":{"target":"browser","id":format!("{id}-{attempt}"),"g":g,"deadline_ms":15000,"p":[["navigate","https://example.com"],["wait",["text","Example Domain"],10000]]}}),
            samples,
        )?;
        if result.get("ok").and_then(Value::as_u64) == Some(1) {
            return Ok(());
        }
        if !stress_retryable(&result) {
            return Err(format!("navigate failed: {result}"));
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err("navigate stayed stale after retries".into())
}
fn stress_live(root: &Path) -> Result<(), String> {
    let exe = root.join("target/dist").join(if cfg!(windows) { "au.exe" } else { "au" });
    if !exe.is_file() {
        return Err("build target/dist/au before running the live stress suite".into());
    }
    let suffix = format!("stress-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis());
    let mut session = JsonlSession::start(&exe)?;
    let mut observe_us = Vec::new();
    let mut browser_us = Vec::new();
    let mut action_us = Vec::new();
    let mut scenario_us = Vec::new();
    let mut reconnect_us = Vec::new();
    let mut cdp_us = Vec::new();
    let mut unknown = 0usize;
    require_ok(&stress_call(&mut session, json!({"tool":"android.read","arguments":{"q":"status"}}), &mut observe_us)?, "stress status")?;
    let first = stress_call(&mut session, json!({"tool":"android.read","arguments":{"q":"observe"}}), &mut observe_us)?;
    let generation_value = generation(&first)?;
    for _ in 0..100 {
        let warm = stress_call(&mut session, json!({"tool":"android.read","arguments":{"q":"observe","base":generation_value.to_string()}}), &mut observe_us)?;
        if warm.get("=").and_then(Value::as_u64) != Some(1) && warm.get("g").and_then(Value::as_u64) != Some(generation_value) {
            return Err("warm observe changed generation unexpectedly".into());
        }
    }
    let mut semantic = Vec::with_capacity(25);
    for _ in 0..25 {
        semantic.push(json!(["assert", ["generation_after", 0]]));
    }
    let action = stress_call(&mut session, json!({"tool":"android.act","arguments":{"id":format!("{suffix}-assert"),"g":generation_value,"p":semantic}}), &mut action_us)?;
    require_ok(&action, "25 safe semantic assertions")?;
    stress_launch(&mut session, "com.android.settings", &format!("{suffix}-settings"), &mut action_us, &mut observe_us)?;
    stress_launch(&mut session, "com.android.chrome", &format!("{suffix}-chrome"), &mut action_us, &mut observe_us)?;
    stress_navigate(&mut session, &format!("{suffix}-navigate"), &mut browser_us)?;
    let page = stress_call(&mut session, json!({"tool":"android.read","arguments":{"q":"browser","op":"text"}}), &mut browser_us)?;
    if !page.get("text").and_then(Value::as_str).unwrap_or("").contains("Example Domain") {
        return Err("stress browser text did not contain Example Domain".into());
    }
    for _ in 0..49 {
        let page = stress_call(&mut session, json!({"tool":"android.read","arguments":{"q":"browser","op":"text"}}), &mut browser_us)?;
        if !page.get("text").and_then(Value::as_str).unwrap_or("").contains("Example Domain") {
            return Err("browser text lost Example Domain".into());
        }
    }
    for i in 0..10 {
        let started = Instant::now();
        stress_launch(&mut session, "com.android.settings", &format!("{suffix}-s{i}"), &mut scenario_us, &mut observe_us)?;
        stress_launch(&mut session, "com.android.chrome", &format!("{suffix}-c{i}"), &mut scenario_us, &mut observe_us)?;
        stress_navigate(&mut session, &format!("{suffix}-p{i}"), &mut scenario_us)?;
        let page = stress_call(&mut session, json!({"tool":"android.read","arguments":{"q":"browser","op":"text"}}), &mut scenario_us)?;
        if !page.get("text").and_then(Value::as_str).unwrap_or("").contains("Example Domain") {
            return Err(format!("scenario {i} lost Example Domain"));
        }
        if page.get("e").and_then(Value::as_str) == Some("unknown") {
            unknown += 1;
        }
        println!("stress_scenario_{i}_ms={}", started.elapsed().as_millis());
    }
    drop(session);
    for _ in 0..5 {
        let started = Instant::now();
        let mut fresh = JsonlSession::start(&exe)?;
        require_ok(&stress_call(&mut fresh, json!({"tool":"android.read","arguments":{"q":"status"}}), &mut reconnect_us)?, "reconnect status")?;
        reconnect_us.push(started.elapsed().as_micros());
    }
    for i in 0..5 {
        let started = Instant::now();
        let mut fresh = JsonlSession::start(&exe)?;
        let tabs = stress_call(&mut fresh, json!({"tool":"android.read","arguments":{"q":"browser","op":"tabs"}}), &mut cdp_us)?;
        let _ = generation(&tabs)?;
        cdp_us.push(started.elapsed().as_micros());
        println!("stress_cdp_cycle_{i}=ok");
    }
    for samples in [&mut observe_us, &mut browser_us, &mut action_us, &mut scenario_us, &mut reconnect_us, &mut cdp_us] {
        samples.sort_unstable();
    }
    println!("stress_observes=100 stress_browser_reads=50 stress_semantic_actions=25 stress_scenarios=10 stress_reconnects=5 stress_cdp_reconnects=5 unknown_receipts={unknown}");
    println!("stress_observe_p50_us={} stress_observe_p95_us={} stress_browser_p50_us={} stress_browser_p95_us={} stress_action_p50_us={} stress_action_p95_us={} stress_scenario_p50_us={} stress_scenario_p95_us={} stress_reconnect_p50_us={} stress_reconnect_p95_us={} stress_cdp_p50_us={} stress_cdp_p95_us={}", quantile(&observe_us, 0.50), quantile(&observe_us, 0.95), quantile(&browser_us, 0.50), quantile(&browser_us, 0.95), quantile(&action_us, 0.50), quantile(&action_us, 0.95), quantile(&scenario_us, 0.50), quantile(&scenario_us, 0.95), quantile(&reconnect_us, 0.50), quantile(&reconnect_us, 0.95), quantile(&cdp_us, 0.50), quantile(&cdp_us, 0.95));
    Ok(())
}
fn live(root: &Path) -> Result<(), String> {
    let exe = root.join("target/dist").join(if cfg!(windows) { "au.exe" } else { "au" });
    if !exe.is_file() {
        return Err("build target/dist/au before running the live scenarios".into());
    }
    let started = Instant::now();
    let status = run_json(&exe, &["status"])?;
    require_ok(&status, "status")?;
    let first = run_json(&exe, &["observe"])?;
    let first_generation = generation(&first)?;
    let suffix = format!("{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis());

    require_ok(&run_plan(&exe, json!({"id":format!("live-settings-{suffix}"),"g":first_generation,"p":[["launch","com.android.settings"]]}))?, "launch settings")?;
    let mut settings = run_json(&exe, &["observe"])?;
    if node_ref(&settings, "Search settings", None).is_some() {
        tap_label(&exe, "Search settings", &suffix, "open settings search")?;
        if wait_for_label(&exe, "Search", Some("i")).is_ok() {
            type_and_submit(&exe, "Search", "battery", &suffix, "search settings")?;
            let _battery = wait_for_label(&exe, "Battery", None)?;
        }
        settings = run_json(&exe, &["observe"])?;
    } else {
        // Some Android Settings builds expose the category list but omit the
        // search affordance from the semantic tree. Exercise a real Settings
        // screen in that case, then return before starting the browser flow.
        for attempt in 0..4 {
            if node_ref(&settings, "Network & internet", None).is_some() {
                break;
            }
            if node_ref(&settings, "Search settings", None).is_some() {
                break;
            }
            let result = run_plan(&exe, json!({"id":format!("live-settings-back-{suffix}-{attempt}"),"g":generation(&settings)?,"p":[["key","back"]]}))?;
            if result.get("ok").and_then(Value::as_u64) != Some(1) {
                if result.get("e").and_then(Value::as_str) == Some("stale") {
                    settings = run_json(&exe, &["observe"])?;
                    continue;
                }
                return Err(format!("return to Settings home failed: {result}"));
            }
            thread::sleep(Duration::from_millis(250));
            settings = run_json(&exe, &["observe"])?;
        }
        if node_ref(&settings, "Search settings", None).is_some() {
            tap_label(&exe, "Search settings", &suffix, "open settings search")?;
            if wait_for_label(&exe, "Search", Some("i")).is_ok() {
                type_and_submit(&exe, "Search", "battery", &suffix, "search settings")?;
                let _battery = wait_for_label(&exe, "Battery", None)?;
            }
            settings = run_json(&exe, &["observe"])?;
        } else if node_ref(&settings, "Network & internet", None).is_some() {
            // The category is visible but not actionable on some Settings
            // builds. Observing it is still a valid app checkpoint; avoid
            // guessing a coordinate or treating a non-clickable container as
            // a failed user action.
            settings = wait_for_label(&exe, "Network & internet", None)?;
        } else {
            return Err("Settings exposed neither search nor a navigable category".into());
        }
    }

    require_ok(&run_plan(&exe, json!({"id":format!("live-chrome-{suffix}"),"g":generation(&settings)?,"p":[["launch","com.android.chrome"]]}))?, "launch Chrome")?;
    let tabs = run_json(&exe, &["browser", "tabs"])?;
    let browser_generation = generation(&tabs)?;
    require_ok(
        &run_plan(
            &exe,
            json!({"target":"browser","id":format!("live-page-{suffix}"),"g":browser_generation,"deadline_ms":15000,"p":[["navigate","https://example.com"],["wait",["text","Example Domain"],10000]]}),
        )?,
        "navigate example.com",
    )?;
    let page = run_json(&exe, &["browser", "text"])?;
    let text = page.get("text").and_then(Value::as_str).unwrap_or("");
    if !text.contains("Example Domain") {
        return Err("browser scenario did not observe Example Domain".into());
    }
    let page_generation = generation(&page)?;
    require_ok(
        &run_plan(
            &exe,
            json!({"target":"browser","id":format!("live-scroll-{suffix}"),"g":page_generation,"deadline_ms":15000,"p":[["scroll",320],["wait",["text","Example Domain"],5000]]}),
        )?,
        "scroll example.com",
    )?;
    let after_scroll = run_json(&exe, &["browser", "text"])?;
    require_ok(
        &run_plan(
            &exe,
            json!({"target":"browser","id":format!("live-reload-{suffix}"),"g":generation(&after_scroll)?,"deadline_ms":15000,"p":[["reload"],["wait",["text","Example Domain"],10000]]}),
        )?,
        "reload example.com",
    )?;
    let final_page = run_json(&exe, &["browser", "text"])?;
    if !final_page.get("text").and_then(Value::as_str).unwrap_or("").contains("Example Domain") {
        return Err("browser reload did not preserve Example Domain".into());
    }
    println!("live_scenarios=4 elapsed_ms={}", started.elapsed().as_millis());
    Ok(())
}
fn run_json(exe: &Path, args: &[&str]) -> Result<Value, String> {
    let out = Command::new(exe).args(args).output().map_err(ioe)?;
    if !out.status.success() {
        return Err(format!("{} exited with {}: {}", exe.display(), out.status, String::from_utf8_lossy(&out.stdout).trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("{} returned invalid JSON: {e}", exe.display()))
}
fn run_plan(exe: &Path, plan: Value) -> Result<Value, String> {
    let encoded = serde_json::to_string(&plan).map_err(|e| e.to_string())?;
    run_json(exe, &["act", &encoded])
}
fn require_ok(value: &Value, step: &str) -> Result<(), String> {
    if value.get("ok").and_then(Value::as_u64) == Some(1) {
        Ok(())
    } else {
        Err(format!("{step} failed: {}", value))
    }
}
fn generation(value: &Value) -> Result<u64, String> {
    value.get("g").and_then(Value::as_u64).ok_or_else(|| format!("response omitted generation: {value}"))
}
fn wait_for_label(exe: &Path, label: &str, role: Option<&str>) -> Result<Value, String> {
    for _ in 0..40 {
        let value = run_json(exe, &["observe"])?;
        if node_ref(&value, label, role).is_some() {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(format!("UI did not expose {label:?}"))
}
fn tap_label(exe: &Path, label: &str, suffix: &str, step: &str) -> Result<(), String> {
    for attempt in 0..4 {
        let current = run_json(exe, &["observe"])?;
        let reference = node_ref(&current, label, None).ok_or_else(|| format!("{label} control was not found"))?;
        let result = run_plan(exe, json!({"id":format!("live-tap-{suffix}-{attempt}"),"g":generation(&current)?,"p":[["tap",reference]]}))?;
        if result.get("ok").and_then(Value::as_u64) == Some(1) {
            return Ok(());
        }
        match result.get("e").and_then(Value::as_str) {
            Some("stale") | Some("helper") | Some("timeout") => {}
            _ => return Err(format!("{step} failed: {result}")),
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err(format!("{step} stayed retryable after retries"))
}
fn type_and_submit(exe: &Path, label: &str, text: &str, suffix: &str, step: &str) -> Result<(), String> {
    for attempt in 0..4 {
        let current = run_json(exe, &["observe"])?;
        let reference = node_ref(&current, label, Some("i")).ok_or_else(|| format!("{label} input was not found"))?;
        let result = run_plan(exe, json!({"id":format!("live-type-{suffix}-{attempt}"),"g":generation(&current)?,"p":[["text",reference,text],["key","enter"]]}))?;
        if result.get("ok").and_then(Value::as_u64) == Some(1) {
            return Ok(());
        }
        match result.get("e").and_then(Value::as_str) {
            Some("stale") | Some("helper") | Some("timeout") => {}
            _ => return Err(format!("{step} failed: {result}")),
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err(format!("{step} stayed retryable after retries"))
}
fn node_ref(value: &Value, label: &str, role: Option<&str>) -> Option<u64> {
    let rows = value.get("n")?.as_array()?;
    for (index, row) in rows.iter().enumerate() {
        let row = row.as_array()?;
        let id = row.first()?.as_u64()?;
        let text = row.get(1)?.as_str()?;
        let kind = row.get(2)?.as_str()?;
        if !text.contains(label) || role.is_some_and(|want| want != kind) {
            continue;
        }
        if role.is_some() || row.get(3).and_then(Value::as_u64).unwrap_or(0) & 1 != 0 {
            return Some(id);
        }
        for parent in rows[..index].iter().rev() {
            let parent = parent.as_array()?;
            if parent.get(3).and_then(Value::as_u64).unwrap_or(0) & 1 != 0 {
                return parent.first().and_then(Value::as_u64);
            }
        }
        return Some(id);
    }
    None
}
fn size(root: &Path) -> Result<(usize, usize, usize), String> {
    let rust = lines(&root.join("computer/src"), &["rs"])?;
    let java = lines(&root.join("device/app/src/main"), &["java"])?;
    let automation = lines(&root.join("tools"), &["rs"])? + lines(&root.join("install"), &["mjs"])?;
    let tests = lines(&root.join("computer/tests"), &["rs"])? + lines(&root.join("device/app/src/test"), &["java"])? + lines(&root.join("install/test"), &["mjs"])?;
    let authored = lines(root, &["rs", "java", "mjs", "js", "ps1", "sh", "cmd", "kt"])?;
    let source_files = file_count(root, &["rs", "java", "mjs", "js", "ps1", "sh", "cmd", "kt"])?;
    let production_modules = file_count(&root.join("computer/src"), &["rs"])? + file_count(&root.join("device/app/src/main"), &["java"])?;
    println!("rust_production={rust} android_production={java} tests={tests} automation={automation} authored_code={authored} source_files={source_files} production_modules={production_modules}");
    if rust > 6500 || java > 2200 || automation > 900 || authored > 11300 {
        return Err("source budget exceeded".into());
    }
    Ok((rust, java, automation))
}
fn gate(root: &Path) -> Result<(), String> {
    for p in ["crates", "packages", "wire", "android", "xtask", "scripts", "packaging"] {
        if root.join(p).is_dir() {
            return Err(format!("old wrapper folder remains: {p}"));
        }
    }
    for p in walk(&root.join("computer/src"))? {
        if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            let s = fs::read_to_string(&p).map_err(ioe)?;
            if s.contains("unsafe {") || s.contains("Command::new(\"sh\")") || s.contains("Command::new(\"adb\")") {
                return Err(format!("security gate failed: {}", p.display()));
            }
            if s.lines().count() > 900 {
                return Err(format!("production file exceeds 900 lines: {}", p.display()));
            }
        }
    }
    size(root).map(|_| ())
}
fn docs(root: &Path) -> Result<(), String> {
    for p in [
        "README.md",
        "AGENTS.md",
        "llms.txt",
        "SECURITY.md",
        "docs/README.md",
        "docs/getting-started.md",
        "docs/agents/quickstart.md",
        "docs/reference/cli.md",
        "docs/reference/agent-protocol.md",
        "docs/troubleshooting.md",
        "examples/README.md",
        "skills/android-use/SKILL.md",
        "skills/android-use/references/protocol.md",
        "skills/android-use/references/safety.md",
        "skills/android-use/references/setup.md",
    ] {
        if !root.join(p).is_file() {
            return Err(format!("missing document {p}"));
        }
    }
    Ok(())
}
fn android(root: &Path, task: &str) -> Result<(), String> {
    let gradle = env::var_os("AU_GRADLE")
        .map(PathBuf::from)
        .or_else(|| if cfg!(windows) { env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join("Codex/android-use/tools/gradle-9.1.0/bin/gradle.bat")) } else { None })
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "gradle.bat" } else { "gradle" }));
    let task_arg = format!(":app:{task}");
    let native_dir = env::temp_dir().join("android-use-gradle-native");
    let args = if cfg!(windows) {
        vec!["--no-daemon".to_owned(), format!("-Dorg.gradle.native.dir={}", native_dir.display()), task_arg]
    } else {
        vec!["--no-daemon".to_owned(), task_arg]
    };
    println!("+ {} {}", gradle.display(), args.join(" "));
    let mut command = Command::new(&gradle);
    command.args(&args).current_dir(root.join("device"));
    if env::var_os("ANDROID_HOME").is_none() {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            let sdk = PathBuf::from(local).join("Android/Sdk");
            command.env("ANDROID_HOME", &sdk).env("ANDROID_SDK_ROOT", sdk);
        }
    }
    if env::var_os("GRADLE_USER_HOME").is_none() {
        if let Some(user) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
            command.env("GRADLE_USER_HOME", PathBuf::from(user).join(".gradle"));
        }
    }
    let status = command.status().map_err(ioe)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", gradle.display()))
    }
}
fn lines(root: &Path, exts: &[&str]) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }
    let mut n = 0;
    for p in walk(root)? {
        if p.components().any(|c| matches!(c.as_os_str().to_str(), Some("target" | "build" | ".git" | "node_modules" | ".gradle" | "artifacts" | "dist"))) {
            continue;
        }
        if p.is_file() && exts.contains(&p.extension().and_then(|s| s.to_str()).unwrap_or("")) {
            n += fs::read_to_string(p).map_err(ioe)?.lines().count();
        }
    }
    Ok(n)
}
fn walk(root: &Path) -> Result<Vec<PathBuf>, String> {
    let (mut out, mut todo) = (Vec::new(), vec![root.to_path_buf()]);
    while let Some(p) = todo.pop() {
        for e in fs::read_dir(p).map_err(ioe)? {
            let p = e.map_err(ioe)?.path();
            if p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| matches!(name, "target" | "build" | ".git" | "node_modules" | ".gradle" | "artifacts" | "dist" | ".codex-run"))
            {
                continue;
            }
            if p.is_dir() {
                todo.push(p.clone())
            }
            out.push(p)
        }
    }
    Ok(out)
}
fn root() -> Result<PathBuf, String> {
    let mut p = env::current_dir().map_err(ioe)?;
    loop {
        if p.join("tools/Cargo.toml").is_file() {
            return Ok(p);
        }
        if !p.pop() {
            return Err("run inside the android-use checkout".into());
        }
    }
}
fn cmd(cwd: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    println!("+ {program} {}", args.join(" "));
    let status = Command::new(program).args(args).current_dir(cwd).status().map_err(ioe)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(ioe)?;
    }
    fs::copy(from, to).map(|_| ()).map_err(ioe)
}
fn release_apk(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("device/app/build/outputs/apk/release");
    for name in ["bridge-release.apk", "app-release.apk"] {
        let path = dir.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!("release APK was not found under {}", dir.display()))
}
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(ioe)?;
    for entry in fs::read_dir(from).map_err(ioe)? {
        let entry = entry.map_err(ioe)?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            copy(&source, &target)?;
        }
    }
    Ok(())
}
fn platform_name() -> String {
    let os = if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "ia32"
    } else {
        env::consts::ARCH
    };
    format!("{os}-{arch}")
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}
fn quantile(values: &[u128], percentile: f64) -> u128 {
    let index = ((values.len().saturating_sub(1)) as f64 * percentile).round() as usize;
    values[index.min(values.len().saturating_sub(1))]
}
fn file_bytes(path: &Path) -> Result<u64, String> {
    Ok(fs::metadata(path).map(|m| m.len()).unwrap_or(0))
}
fn tree_bytes(root: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    for path in walk(root)? {
        if path.is_file() {
            total = total.saturating_add(fs::metadata(path).map_err(ioe)?.len());
        }
    }
    Ok(total)
}
fn file_count(root: &Path, exts: &[&str]) -> Result<usize, String> {
    Ok(walk(root)?.into_iter().filter(|p| p.is_file() && exts.contains(&p.extension().and_then(|s| s.to_str()).unwrap_or(""))).count())
}
fn ioe(e: impl std::fmt::Display) -> String {
    e.to_string()
}
