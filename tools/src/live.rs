//! `cargo xtask benchmark-live`: drives `au serve --mcp` on a real device the way an agent client
//! does, over the shipped command grammar, and reports calls, tokens, time, and success per task.
//!
//! The device needs the helper enabled and the `aubench` fixture installed (`cargo xtask android`
//! builds both). Tokens are bytes / 4, the same estimate the documentation budgets use.
//!
//! Environment: `AU_BIN` (default `target/dist/au`), `AU_ADB` or `adb` on PATH, `BENCH_RUNS`
//! (default 3), `BENCH_OUT` (default `target/benchmark-live.json`), `BENCH_BASELINE` (a previous
//! report; with `BENCH_GATE=1` a lower success rate or a p95 more than 20% slower fails the run),
//! `BENCH_POLICY` (`1.2`, the default, or `act-first`). Each policy's `GATES` are checked last.
use super::{approx_tokens, ioe, quantile};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{BufRead, BufReader, Read as _, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SETTINGS: &str = "com.android.settings/.Settings";
const AUBENCH: &str = "dev.codex.aubench/.MainActivity";

/// One scripted turn. `expect` must appear in the result; when it does not, the agent reads the
/// screen and tries the turn once more, as a model following the documentation would.
enum Step {
    Act(&'static str, &'static str),
    Read(&'static str, &'static str),
}

struct Task {
    name: &'static str,
    /// Activity to cold-start before the task, so every run begins from the same screen.
    reset: Option<&'static str>,
    steps: &'static [Step],
}

use Step::{Act, Read};

/// A static page the Chrome tasks open; the emulator reaches the host's loopback as 10.0.2.2.
const PAGE_PORT: u16 = 18765;
const PAGE: &str = "<!doctype html><title>Bench page</title><h1>Hello bench</h1><button onclick=\"document.title='Clicked'\">Press me</button><input placeholder=\"Your name\">";

/// Serves `PAGE` until the process ends; false when the port is taken (the Chrome tasks then skip).
fn serve_page() -> bool {
    let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", PAGE_PORT)) else { return false };
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.read(&mut [0u8; 1024]);
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}", PAGE.len());
        }
    });
    true
}

const TASKS: &[Task] = &[
    Task { name: "status_and_screen", reset: None, steps: &[Read("status", "Ready"), Read("screen", "")] },
    Task { name: "toggle_switch", reset: Some(AUBENCH), steps: &[Act("open app \"AU Benchmark Fixture\"", "Done"), Act("tap \"Benchmark toggle\"", "=on")] },
    Task {
        name: "fill_submit_form",
        reset: Some(AUBENCH),
        steps: &[Act("open app \"AU Benchmark Fixture\"", "Done"), Act("type \"hello\" in \"Fixture text\" then tap \"SUBMIT FORM\"", "Submitted: hello")],
    },
    Task {
        name: "dismiss_dialog",
        reset: Some(AUBENCH),
        steps: &[Act("open app \"AU Benchmark Fixture\"", "Done"), Act("tap \"OPEN BENCHMARK DIALOG\"", "CANCEL"), Act("tap \"CANCEL\"", "SUBMIT FORM")],
    },
    Task {
        name: "confirm_dialog",
        reset: Some(AUBENCH),
        steps: &[Act("open app \"AU Benchmark Fixture\"", "Done"), Act("tap \"OPEN BENCHMARK DIALOG\"", "CONFIRM"), Act("tap \"CONFIRM\"", "Dialog confirmed")],
    },
    Task { name: "duplicate_label", reset: Some(AUBENCH), steps: &[Act("open app \"AU Benchmark Fixture\"", "Done"), Act("tap \"Details\" number 2", "Details 2 opened")] },
    Task { name: "near_miss_label", reset: Some(AUBENCH), steps: &[Act("open app \"AU Benchmark Fixture\"", "Done"), Act("tap \"Save\"", "Saved draft")] },
    Task {
        name: "slow_content",
        reset: Some(AUBENCH),
        steps: &[
            Act("open app \"AU Benchmark Fixture\"", "Done"),
            Act("tap \"LOAD SLOW CONTENT\" then wait for text \"Slow content ready\" up to 5 seconds", "Slow content ready"),
        ],
    },
    Task {
        name: "scroll_to_row_40",
        reset: Some(AUBENCH),
        steps: &[Act("open app \"AU Benchmark Fixture\" then scroll down in \"Benchmark scroll surface\" until \"Fixture row 40\" then tap \"Fixture row 40\"", "Done")],
    },
    Task {
        name: "find_setting_scroll",
        reset: Some(SETTINGS),
        steps: &[Act("open app \"Settings\" then scroll down until \"About emulated device\" then tap \"About emulated device\"", "Android version")],
    },
    Task {
        name: "toggle_wifi",
        reset: Some(SETTINGS),
        steps: &[Act("open app \"Settings\"", "Done"), Act("tap \"Network & internet\"", "Internet"), Act("tap \"Internet\"", "Wi-Fi"), Act("tap \"Wi-Fi\"", "Done")],
    },
    Task {
        name: "chrome_page",
        reset: None,
        steps: &[
            Act("open app \"Chrome\"", "Done"),
            Act("page open \"http://10.0.2.2:18765/\" then page click \"Press me\" then page text matching \"Hello\"", "Hello bench"),
            Read("browser tabs", "Clicked"),
        ],
    },
    Task {
        name: "settings_search",
        reset: Some(SETTINGS),
        steps: &[
            Act("open app \"Settings\"", "Done"),
            Act("tap \"Search Settings\"", "Search"),
            Act("type \"battery\" in \"Search settings\" then wait for text \"Battery usage\" up to 8 seconds", "Done"),
            Act("tap \"Use Battery Saver\"", "Done"),
        ],
    },
];

struct Mcp {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    id: u64,
    fixed_tokens: usize,
}

struct Call {
    ms: u128,
    text: String,
    error: bool,
}

impl Mcp {
    fn start(exe: &Path) -> Result<Self, String> {
        let mut child = Command::new(exe).args(["serve", "--mcp"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(ioe)?;
        let input = child.stdin.take().ok_or("MCP stdin was unavailable")?;
        let output = BufReader::new(child.stdout.take().ok_or("MCP stdout was unavailable")?);
        let mut mcp = Self { child, input: Some(input), output, id: 0, fixed_tokens: 0 };
        let init = mcp.rpc("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"xtask","version":"0"}}))?;
        mcp.notify("notifications/initialized")?;
        let tools = mcp.rpc("tools/list", json!({}))?;
        let instructions = init["instructions"].as_str().unwrap_or_default();
        mcp.fixed_tokens = approx_tokens(instructions.len() + serde_json::to_vec(&tools["tools"]).map_err(ioe)?.len());
        Ok(mcp)
    }

    fn send(&mut self, value: Value) -> Result<(), String> {
        let input = self.input.as_mut().ok_or("MCP stdin was closed")?;
        input.write_all(&serde_json::to_vec(&value).map_err(ioe)?).map_err(ioe)?;
        input.write_all(b"\n").map_err(ioe)?;
        input.flush().map_err(ioe)
    }

    fn notify(&mut self, method: &str) -> Result<(), String> {
        self.send(json!({"jsonrpc":"2.0","method":method}))
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.id += 1;
        let id = self.id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        loop {
            let mut line = String::new();
            if self.output.read_line(&mut line).map_err(ioe)? == 0 {
                return Err("au serve closed before replying".into());
            }
            let message: Value = serde_json::from_str(&line).map_err(ioe)?;
            if message["id"] == json!(id) {
                return message.get("result").cloned().ok_or_else(|| format!("{method} failed: {message}"));
            }
        }
    }

    fn call(&mut self, tool: &str, command: &str) -> Result<Call, String> {
        let started = Instant::now();
        let result = self.rpc("tools/call", json!({"name":tool,"arguments":{"command":command}}))?;
        let text: String = result["content"].as_array().into_iter().flatten().filter_map(|part| part["text"].as_str()).collect();
        Ok(Call { ms: started.elapsed().as_millis(), text, error: result["isError"] == json!(true) })
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.input.take();
        for _ in 0..40 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Run {
    /// Every call as `[tool, command, result]`, for `BENCH_TRACE` (real tokenizer counts).
    trace: Vec<Value>,
    ok: bool,
    calls: usize,
    wall_ms: u128,
    result_tokens: usize,
    output_tokens: usize,
}

fn adb(args: &[&str]) {
    let program = env::var("AU_ADB").unwrap_or_else(|_| "adb".into());
    let _ = Command::new(program).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Each run gets a fresh server: a new agent has seen no screen, so none is summarized as a diff.
fn run_task(exe: &Path, task: &Task, act_first: bool, latency: &mut BTreeMap<String, Vec<u128>>) -> Result<Run, String> {
    let mut mcp = Mcp::start(exe)?;
    let mcp = &mut mcp;
    if let Some(activity) = task.reset {
        adb(&["shell", "am", "start", "-W", "-S", "--activity-clear-task", "-n", activity]);
    }
    let started = Instant::now();
    let (mut calls, mut result_tokens, mut output_tokens, mut ok) = (0, 0, 0, true);
    let mut trace = Vec::new();
    let mut turn = |mcp: &mut Mcp, tool: &str, command: &str, calls: &mut usize| -> Result<Call, String> {
        let call = mcp.call(tool, command)?;
        *calls += 1;
        if env::var_os("BENCH_VERBOSE").is_some() {
            println!("  {tool} {command} -> {}ms {}", call.ms, call.text.lines().take(3).collect::<Vec<_>>().join(" | "));
        }
        trace.push(json!([tool, command, call.text]));
        result_tokens += approx_tokens(call.text.len());
        output_tokens += approx_tokens(command.len() + 40);
        latency.entry(latency_key(tool, command, act_first)).or_default().push(call.ms);
        Ok(call)
    };
    'steps: for (tool, command, expect) in script(task.steps, act_first) {
        let (command, expect) = (command.as_str(), expect);
        let call = turn(mcp, tool, command, &mut calls)?;
        if !call.error && call.text.contains(expect) {
            continue 'steps;
        }
        // Look, then try the turn once more.
        turn(mcp, "android_read", "screen", &mut calls)?;
        // Act-first retries only the chain's last action: the earlier ones already ran.
        let retry = command.rsplit_once(" then ").filter(|_| act_first).map_or(command, |(_, last)| last);
        let again = turn(mcp, tool, retry, &mut calls)?;
        if again.error || !again.text.contains(expect) {
            ok = false;
            break 'steps;
        }
    }
    Ok(Run { trace, ok, calls, wall_ms: started.elapsed().as_millis(), result_tokens, output_tokens })
}

/// Latency bucket of a call: its first verb, or its last under act-first (chains start with
/// `open app`); calls that wait on slow content are kept out of the tap and type buckets.
fn latency_key(tool: &str, command: &str, act_first: bool) -> String {
    let verb =
        if command.contains(" wait for ") { "wait" } else { command.rsplit(if act_first { " then " } else { "\0" }).next().and_then(|c| c.split(' ').next()).unwrap_or_default() };
    format!("{tool} {verb}")
}

/// The turns of a task. Act-first acts without looking: consecutive actions become one `then`
/// chain judged by its last result, and a toggle that must end `on` uses `set`.
fn script(steps: &[Step], act_first: bool) -> Vec<(&'static str, String, &'static str)> {
    let mut turns: Vec<(&'static str, String, &'static str)> = Vec::new();
    for step in steps {
        match *step {
            Read(command, expect) => turns.push(("android_read", command.into(), expect)),
            Act(command, expect) => {
                let (command, expect) = match command.strip_prefix("tap ").filter(|_| act_first && expect == "=on") {
                    Some(label) => (format!("set {label} on"), " on."),
                    None => (command.to_string(), expect),
                };
                match turns.last_mut() {
                    Some(last) if act_first && last.0 == "android_act" => *last = (last.0, format!("{} then {command}", last.1), expect),
                    _ => turns.push(("android_act", command, expect)),
                }
            }
        }
    }
    turns
}

/// Absolute gates as (name, policy or all, metric, most allowed); see `metric`.
const GATES: &[(&str, Option<&str>, &str, f64)] = &[
    ("misses over 10 runs", Some("1.2"), "misses", 1.0),
    ("late-loading Settings misses", Some("1.2"), "late_misses", 0.0),
    ("calls per task", Some("1.2"), "calls", 2.40),
    ("calls per task", Some("act-first"), "calls", 1.6),
    ("failure percent", Some("act-first"), "failure_pct", 2.0),
    ("tap p50 ms", None, "android_act tap p50", 600.0),
    ("tap p95 ms", None, "android_act tap p95", 1500.0),
    ("type p95 ms", None, "android_act type p95", 4000.0),
];
/// Open-app p95 has no absolute number: `gate` holds it to 20% of a baseline.
const LATE: [&str; 3] = ["find_setting_scroll", "toggle_wifi", "settings_search"];

fn metric(out: &Value, key: &str) -> Option<f64> {
    let tasks = out["tasks"].as_array()?;
    let sum = |field: &str, keep: &dyn Fn(&Value) -> bool| tasks.iter().filter(|t| keep(t)).filter_map(|t| t[field].as_f64()).sum::<f64>();
    let (success, attempts) = (out["success"].as_f64()?, out["attempts"].as_f64()?);
    match key {
        "misses" => Some(attempts - success),
        "late_misses" => {
            Some(sum("runs", &|t| LATE.contains(&t["task"].as_str().unwrap_or_default())) - sum("success", &|t| LATE.contains(&t["task"].as_str().unwrap_or_default())))
        }
        "failure_pct" => Some(100.0 * (attempts - success) / attempts.max(1.0)),
        "calls" => Some(sum("calls", &|_| true) / tasks.len().max(1) as f64),
        _ => key.rsplit_once(' ').and_then(|(name, q)| out["latency"][name][q].as_f64()),
    }
}

/// One pass/fail line per gate that applies to `policy`; a metric with no samples fails.
fn gates(out: &Value, policy: &str) -> Vec<(String, bool)> {
    let applies = GATES.iter().filter(|g| g.1.is_none_or(|p| p == policy));
    applies
        .map(|&(name, _, key, max)| match metric(out, key) {
            Some(value) => (format!("gate {name}: {value:.2} (max {max}) {}", if value <= max + 1e-9 { "pass" } else { "FAIL" }), value <= max + 1e-9),
            None => (format!("gate {name}: no samples FAIL"), false),
        })
        .collect()
}

pub fn run(root: &Path) -> Result<(), String> {
    let exe = env::var_os("AU_BIN").map(PathBuf::from).unwrap_or_else(|| root.join("target/dist").join(if cfg!(windows) { "au.exe" } else { "au" }));
    if !exe.is_file() {
        return Err("build target/dist/au (cargo build --profile dist -p au) before running the live benchmark".into());
    }
    let runs: usize = env::var("BENCH_RUNS").ok().and_then(|value| value.parse().ok()).unwrap_or(3);
    let policy = env::var("BENCH_POLICY").unwrap_or_else(|_| "1.2".into());
    if policy != "1.2" && policy != "act-first" {
        return Err(format!("BENCH_POLICY must be 1.2 or act-first, not {policy}"));
    }
    let mcp = Mcp::start(&exe)?;
    let mut latency: BTreeMap<String, Vec<u128>> = BTreeMap::new();
    let mut report = Vec::new();
    let mut traces = Vec::new();
    println!("fixed_tokens_per_turn={}", mcp.fixed_tokens);
    let page = serve_page();
    for task in TASKS.iter().filter(|task| page || task.name != "chrome_page") {
        let results: Vec<Run> = (0..runs).map(|_| run_task(&exe, task, policy == "act-first", &mut latency)).collect::<Result<_, _>>()?;
        let mean = |f: &dyn Fn(&Run) -> f64| results.iter().map(f).sum::<f64>() / results.len() as f64;
        let entry = json!({
            "task": task.name,
            "success": results.iter().filter(|run| run.ok).count(),
            "runs": results.len(),
            "calls": mean(&|run| run.calls as f64),
            "input_tokens": mean(&|run| (run.result_tokens + run.calls * mcp.fixed_tokens) as f64),
            "output_tokens": mean(&|run| run.output_tokens as f64),
            "wall_ms": mean(&|run| run.wall_ms as f64),
        });
        println!(
            "task={} success={}/{} calls={:.1} input_tokens={:.0} output_tokens={:.0} wall_ms={:.0}",
            task.name,
            entry["success"],
            runs,
            entry["calls"].as_f64().unwrap_or(0.0),
            entry["input_tokens"].as_f64().unwrap_or(0.0),
            entry["output_tokens"].as_f64().unwrap_or(0.0),
            entry["wall_ms"].as_f64().unwrap_or(0.0)
        );
        traces.push(json!({"task": task.name, "runs": results.iter().map(|run| json!({"ok": run.ok, "calls": run.trace})).collect::<Vec<_>>()}));
        report.push(entry);
    }
    let mut actions = serde_json::Map::new();
    for (name, samples) in &mut latency {
        samples.sort_unstable();
        println!("latency {name}: p50={}ms p95={}ms n={}", quantile(samples, 0.5), quantile(samples, 0.95), samples.len());
        actions.insert(name.clone(), json!({"p50": quantile(samples, 0.5), "p95": quantile(samples, 0.95), "n": samples.len()}));
    }
    let total = |key: &str| report.iter().filter_map(|entry| entry[key].as_f64()).sum::<f64>();
    let (success, attempts) = (report.iter().map(|e| e["success"].as_u64().unwrap_or(0)).sum::<u64>(), report.iter().map(|e| e["runs"].as_u64().unwrap_or(0)).sum::<u64>());
    println!("mean_calls={:.2} mean_input_tokens={:.0} success={success}/{attempts}", total("calls") / report.len() as f64, total("input_tokens") / report.len() as f64);
    let out = json!({"fixed_tokens": mcp.fixed_tokens, "tasks": report, "latency": actions, "success": success, "attempts": attempts});
    let path = env::var_os("BENCH_OUT").map(PathBuf::from).unwrap_or_else(|| root.join("target/benchmark-live.json"));
    fs::write(&path, serde_json::to_vec_pretty(&out).map_err(ioe)?).map_err(ioe)?;
    if let Some(trace) = env::var_os("BENCH_TRACE") {
        fs::write(trace, serde_json::to_vec(&traces).map_err(ioe)?).map_err(ioe)?;
    }
    let lines = gates(&out, &policy);
    lines.iter().for_each(|line| println!("{}", line.0));
    let mut failures = Vec::new();
    if lines.iter().any(|line| !line.1) {
        failures.push(format!("{policy} policy missed an absolute gate"));
    }
    if env::var("BENCH_GATE").is_ok_and(|value| value == "1") {
        if let Err(error) = gate(&out, env::var_os("BENCH_BASELINE").map(PathBuf::from).ok_or("BENCH_GATE=1 needs BENCH_BASELINE")?) {
            failures.push(error);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// Fails when the success rate drops or an action's p95 is more than 20% slower than the baseline.
fn gate(now: &Value, baseline: PathBuf) -> Result<(), String> {
    let before: Value = serde_json::from_slice(&fs::read(baseline).map_err(ioe)?).map_err(ioe)?;
    let rate = |report: &Value| report["success"].as_f64().unwrap_or(0.0) / report["attempts"].as_f64().unwrap_or(1.0).max(1.0);
    if rate(now) + 1e-9 < rate(&before) {
        return Err(format!("success rate fell from {:.2} to {:.2}", rate(&before), rate(now)));
    }
    for (name, was) in before["latency"].as_object().into_iter().flatten() {
        if let (Some(old), Some(new)) = (was["p95"].as_f64(), now["latency"][name]["p95"].as_f64()) {
            if new > old * 1.2 + 50.0 {
                return Err(format!("{name} p95 regressed from {old}ms to {new}ms"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(success: u64, attempts: u64, calls: f64, late_success: u64, tap: (u64, u64), kind: u64) -> Value {
        let task = |name: &str, success: u64| json!({"task": name, "success": success, "runs": 10, "calls": calls});
        json!({"success": success, "attempts": attempts, "tasks": [task("find_setting_scroll", late_success), task("toggle_switch", 10)],
            "latency": {"android_act tap": {"p50": tap.0, "p95": tap.1}, "android_act type": {"p50": 1, "p95": kind}}})
    }
    fn failed(out: &Value, policy: &str) -> Vec<String> {
        gates(out, policy).into_iter().filter(|g| !g.1).map(|g| g.0).collect()
    }

    #[test]
    fn gates_pass_and_fail_each_limit() {
        assert!(failed(&out(129, 130, 2.4, 10, (600, 1500), 4000), "1.2").is_empty());
        assert!(failed(&out(128, 130, 2.4, 10, (600, 1500), 4000), "1.2")[0].contains("misses over"));
        assert!(failed(&out(129, 130, 2.4, 9, (600, 1500), 4000), "1.2")[0].contains("late-loading"));
        assert!(failed(&out(130, 130, 2.41, 10, (600, 1500), 4000), "1.2")[0].contains("calls per task"));
        assert!(failed(&out(130, 130, 2.0, 10, (600, 1500), 4000), "act-first")[0].contains("calls per task"));
        assert!(failed(&out(127, 130, 1.6, 10, (600, 1500), 4000), "act-first")[0].contains("failure percent"));
        assert!(failed(&out(128, 130, 1.6, 10, (600, 1500), 4000), "act-first").is_empty());
        assert!(failed(&out(130, 130, 1.6, 10, (601, 1500), 4000), "act-first")[0].contains("tap p50"));
        assert!(failed(&out(130, 130, 1.6, 10, (600, 1501), 4000), "act-first")[0].contains("tap p95"));
        assert!(failed(&out(130, 130, 1.6, 10, (600, 1500), 4001), "act-first")[0].contains("type p95"));
    }

    #[test]
    fn short_runs_judge_attempts_made_and_missing_samples_fail() {
        assert!(failed(&out(30, 30, 2.0, 10, (1, 1), 1), "1.2").is_empty());
        assert!(failed(&out(29, 30, 2.0, 10, (1, 1), 1), "1.2").is_empty());
        assert!(failed(&out(28, 30, 2.0, 10, (1, 1), 1), "1.2").iter().any(|g| g.contains("misses over")));
        let mut no_tap = out(130, 130, 1.0, 10, (1, 1), 1);
        no_tap["latency"] = json!({});
        assert!(failed(&no_tap, "act-first").iter().all(|g| g.contains("no samples")));
    }

    #[test]
    fn act_first_chains_actions_and_sets() {
        let steps = [Act("open app \"A\"", "Done"), Act("tap \"Benchmark toggle\"", "=on"), Read("status", "Ready"), Act("tap \"B\"", "ok")];
        let first = script(&steps, true);
        assert_eq!(first.len(), 3);
        assert_eq!((first[0].1.as_str(), first[0].2), ("open app \"A\" then set \"Benchmark toggle\" on", " on."));
        assert_eq!(first[2].0, "android_act");
        assert_eq!(script(&steps, false).len(), 4);
        assert_eq!(script(&steps, false)[1].1, "tap \"Benchmark toggle\"");
    }

    #[test]
    fn latency_keys_follow_the_verb_and_skip_waits() {
        let chain = "open app \"A\" then type \"x\" in \"F\" then tap \"S\"";
        assert_eq!(latency_key("android_act", chain, false), "android_act open");
        assert_eq!(latency_key("android_act", chain, true), "android_act tap");
        assert_eq!(latency_key("android_act", "tap \"L\" then wait for text \"R\" up to 5 seconds", true), "android_act wait");
        assert_eq!(latency_key("android_act", "tap \"L\" then wait for text \"R\" up to 5 seconds", false), "android_act wait");
    }
}
