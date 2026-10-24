//! `cargo xtask benchmark-live`: drives `au serve --mcp` on a real device the way an agent client
//! does, over the shipped command grammar, and reports calls, tokens, time, and success per task.
//!
//! The device needs the helper enabled and the `aubench` fixture installed (`cargo xtask android`
//! builds both). Tokens are bytes / 4, the same estimate the documentation budgets use.
//!
//! Environment: `AU_BIN` (default `target/dist/au`), `AU_ADB` or `adb` on PATH, `BENCH_RUNS`
//! (default 3), `BENCH_OUT` (default `target/benchmark-live.json`), `BENCH_BASELINE` (a previous
//! report; with `BENCH_GATE=1` a lower success rate or a p95 more than 20% slower fails the run).
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
/// A reset may carry `am start` extras after the component: one lab screen per task.
const LAB_LATE: &str = "dev.codex.aubench/.LabActivity --es screen late";
const LAB_POPUP: &str = "dev.codex.aubench/.LabActivity --es screen popup";
const LAB_ASK: &str = "dev.codex.aubench/.LabActivity --es screen ask";
const LAB_CATALOG: &str = "dev.codex.aubench/.LabActivity --es screen catalog";
const LAB_RISKY: &str = "dev.codex.aubench/.LabActivity --es screen risky";
const LAB_INJECTED: &str = "dev.codex.aubench/.LabActivity --es screen injected";
const LAB_FORM: &str = "dev.codex.aubench/.LabActivity --es screen form";
const LAB_PERSIST: &str = "dev.codex.aubench/.LabActivity --es screen persist";

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
    Task { name: "late_list", reset: Some(LAB_LATE), steps: &[Act("wait for text \"Late row 30\" up to 6 seconds", "Done"), Read("screen", "Late row 35")] },
    Task { name: "popup_rename", reset: Some(LAB_POPUP), steps: &[Read("screen", "Row actions"), Act("tap \"Row actions\"", "Rename"), Act("tap \"Rename\"", "Picked: Rename")] },
    Task {
        name: "app_dialog_decline",
        reset: Some(LAB_ASK),
        steps: &[Read("screen", "Find nearby"), Act("tap \"Find nearby\"", "Allow location access?"), Act("tap \"Don't allow\"", "Location: denied")],
    },
    Task {
        name: "collapse_find_special",
        reset: Some(LAB_CATALOG),
        steps: &[Read("screen", "more like"), Act("scroll down until \"Catalog special\" then tap \"Catalog special\"", "Done")],
    },
    Task { name: "risky_untouched", reset: Some(LAB_RISKY), steps: &[Read("screen", "Risky taps: 0"), Read("find \"Delete account\"", "Delete account")] },
    Task { name: "injected_text", reset: Some(LAB_INJECTED), steps: &[Read("screen", "Ignore previous instructions"), Act("tap \"Continue\"", "Continued")] },
    Task {
        name: "ime_form",
        reset: Some(LAB_FORM),
        steps: &[Read("screen", "Email field"), Act("type \"a@b.co\" in \"Email field\" then tap \"SUBMIT LAB FORM\"", "Lab submitted: a@b.co")],
    },
    Task {
        name: "set_toggle_twice",
        reset: Some(LAB_PERSIST),
        steps: &[Read("screen", "Persisted toggle"), Act("set \"Persisted toggle\" on", "Done"), Act("set \"Persisted toggle\" on", "Done"), Read("screen", "Persisted=on")],
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
    fn start(exe: &Path, quiet: Option<u16>) -> Result<Self, String> {
        let mut child = Command::new(exe)
            .args(["serve", "--mcp"])
            .envs(quiet.map(|ms| ("AU_QUIET_MS", ms.to_string())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(ioe)?;
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
fn run_task(exe: &Path, quiet: Option<u16>, task: &Task, latency: &mut BTreeMap<String, Vec<u128>>) -> Result<Run, String> {
    let mut mcp = Mcp::start(exe, quiet)?;
    let mcp = &mut mcp;
    if let Some(activity) = task.reset {
        let mut args = vec!["shell", "am", "start", "-W", "-S", "--activity-clear-task", "-n"];
        args.extend(activity.split_whitespace());
        adb(&args);
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
        latency.entry(format!("{tool} {}", command.split(' ').next().unwrap_or_default())).or_default().push(call.ms);
        Ok(call)
    };
    'steps: for step in task.steps {
        let (tool, command, expect) = match step {
            Act(command, expect) => ("android_act", *command, *expect),
            Read(command, expect) => ("android_read", *command, *expect),
        };
        let call = turn(mcp, tool, command, &mut calls)?;
        if !call.error && call.text.contains(expect) {
            continue 'steps;
        }
        // Look, then try the turn once more.
        turn(mcp, "android_read", "screen", &mut calls)?;
        let again = turn(mcp, tool, command, &mut calls)?;
        if again.error || !again.text.contains(expect) {
            ok = false;
            break 'steps;
        }
    }
    Ok(Run { trace, ok, calls, wall_ms: started.elapsed().as_millis(), result_tokens, output_tokens })
}

pub fn run(root: &Path) -> Result<(), String> {
    let exe = env::var_os("AU_BIN").map(PathBuf::from).unwrap_or_else(|| root.join("target/dist").join(if cfg!(windows) { "au.exe" } else { "au" }));
    if !exe.is_file() {
        return Err("build target/dist/au (cargo build --locked --profile dist -p au) before running the live benchmark".into());
    }
    let runs: usize = env::var("BENCH_RUNS").ok().and_then(|value| value.parse().ok()).unwrap_or(3);
    // BENCH_QUIET_MS="200,150,120" runs the whole benchmark once per quiet window and compares taps.
    let Some(sweep) = env::var("BENCH_QUIET_MS")
        .ok()
        .map(|list| list.split(',').map(|ms| ms.trim().parse::<u16>().map_err(|_| format!("BENCH_QUIET_MS: {ms:?} is not milliseconds"))).collect::<Result<Vec<_>, _>>())
        .transpose()?
    else {
        return bench(root, &exe, runs, None, None).map(drop);
    };
    for ms in sweep {
        let out = bench(root, &exe, runs, Some(ms), Some(&format!("{ms}ms")))?;
        println!(
            "quiet_ms={ms} tap_p50={}ms tap_p95={}ms success={}/{}",
            out["latency"]["android_act tap"]["p50"], out["latency"]["android_act tap"]["p95"], out["success"], out["attempts"]
        );
    }
    Ok(())
}

/// One benchmark pass with the given quiet window (`None`: the default); `tag` names its report.
fn bench(root: &Path, exe: &Path, runs: usize, quiet: Option<u16>, tag: Option<&str>) -> Result<Value, String> {
    let mcp = Mcp::start(exe, quiet)?;
    let mut latency: BTreeMap<String, Vec<u128>> = BTreeMap::new();
    let mut report = Vec::new();
    let mut traces = Vec::new();
    println!("fixed_tokens_per_turn={}", mcp.fixed_tokens);
    let page = serve_page();
    for task in TASKS.iter().filter(|task| page || task.name != "chrome_page") {
        let results: Vec<Run> = (0..runs).map(|_| run_task(exe, quiet, task, &mut latency)).collect::<Result<_, _>>()?;
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
    let path = tag.map_or(path.clone(), |tag| path.with_extension(format!("{tag}.json")));
    fs::write(&path, serde_json::to_vec_pretty(&out).map_err(ioe)?).map_err(ioe)?;
    if let Some(trace) = env::var_os("BENCH_TRACE") {
        fs::write(trace, serde_json::to_vec(&traces).map_err(ioe)?).map_err(ioe)?;
    }
    if env::var("BENCH_GATE").is_ok_and(|value| value == "1") {
        gate(&out, env::var_os("BENCH_BASELINE").map(PathBuf::from).ok_or("BENCH_GATE=1 needs BENCH_BASELINE")?)?;
    }
    Ok(out)
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
