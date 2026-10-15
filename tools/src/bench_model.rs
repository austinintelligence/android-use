//! `cargo xtask bench-model`: runs a real agent CLI against goals on a device and compares two `au` builds.
//! Success comes from device state (the task's verifier), not the agent's claim. See docs/contributing/testing.md.
use super::ioe;
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Task {
    name: String,
    goal: String,
    reset: Vec<String>,
    /// `(command, text the result must contain, text it must not contain)`
    verify: Vec<(String, String, Option<String>)>,
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v[key].as_str().map(str::to_string).ok_or_else(|| format!("missing string `{key}` in {v}"))
}

fn parse_tasks(src: &str) -> Result<Vec<Task>, String> {
    let doc: Value = serde_json::from_str(src).map_err(ioe)?;
    let tasks = doc["tasks"].as_array().ok_or("tasks file needs a `tasks` array")?;
    tasks
        .iter()
        .map(|t| {
            let list = |key: &str| t[key].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let verify = t["verify"].as_array().ok_or("each task needs `verify`")?;
            let verify = verify.iter().map(|v| Ok((text(v, "cmd")?, text(v, "expect")?, v["not"].as_str().map(str::to_string)))).collect::<Result<Vec<_>, String>>()?;
            if verify.is_empty() {
                return Err(format!("task {} has no verifier", t["name"]));
            }
            Ok(Task { name: text(t, "name")?, goal: text(t, "goal")?, reset: list("reset"), verify })
        })
        .collect()
}

/// Splits on whitespace, then fills `{key}` in each word, so a prompt stays one argument.
fn render(template: &str, vars: &[(&str, &str)]) -> Vec<String> {
    template.split_whitespace().map(|word| vars.iter().fold(word.to_string(), |w, (k, v)| w.replace(&format!("{{{k}}}"), v))).collect()
}

/// MCP tool calls the shim logged: one JSON-RPC line per call.
fn count_calls(log: &str) -> usize {
    log.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|m| m["method"] == "tools/call").count()
}

/// `(input tokens, cost in USD)` from the last JSON line of the agent's output that reports usage; `None` when it reports none.
/// Claude-style usage excludes cached tokens from `input_tokens`, so the three top-level fields add up; Codex-style usage
/// (`cached_input_tokens` present) already includes them.
fn usage(out: &str) -> (Option<f64>, Option<f64>) {
    let last = out.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).find(|v| v.get("usage").is_some() || v.get("total_cost_usd").is_some());
    let Some(v) = last else { return (None, None) };
    let field = |u: &Value, k: &str| u[k].as_f64().unwrap_or(0.0);
    let tokens = v.get("usage").map(|u| {
        let base = field(u, "input_tokens");
        if u.get("cached_input_tokens").is_some() {
            base
        } else {
            base + field(u, "cache_read_input_tokens") + field(u, "cache_creation_input_tokens")
        }
    });
    (tokens, v["total_cost_usd"].as_f64())
}

/// Run order: each round runs every task on both builds, and the build order flips with the round, so drift hits both.
fn schedule(tasks: usize, runs: usize) -> Vec<(usize, usize, usize)> {
    (0..runs).flat_map(|r| (0..tasks).flat_map(move |t| [0, 1].map(move |b| (r, t, (b + r + t) % 2)))).collect()
}

/// Two-sided 95% Student t critical value.
fn t_crit(df: f64) -> f64 {
    const T: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160, 2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064,
        2.060, 2.056, 2.052, 2.048, 2.045, 2.042,
    ];
    if df.is_nan() || df < 1.0 {
        f64::NAN
    } else if df >= 30.0 {
        1.96
    } else {
        T[df.floor() as usize - 1]
    }
}

fn mean_var(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n.max(1.0);
    (m, if n > 1.0 { xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0) } else { f64::NAN })
}

/// Mean and 95% confidence half-width (`null` below two samples).
fn ci(xs: &[f64]) -> Value {
    let (m, v) = mean_var(xs);
    json!({"mean": m, "ci95": t_crit(xs.len() as f64 - 1.0) * (v / xs.len() as f64).sqrt(), "n": xs.len()})
}

/// A minus B with a Welch 95% half-width.
fn diff(a: &[f64], b: &[f64]) -> Value {
    let ((ma, va), (mb, vb)) = (mean_var(a), mean_var(b));
    let (sa, sb) = (va / a.len() as f64, vb / b.len() as f64);
    let df = (sa + sb).powi(2) / (sa * sa / (a.len() as f64 - 1.0) + sb * sb / (b.len() as f64 - 1.0));
    json!({"diff": ma - mb, "ci95": t_crit(df) * (sa + sb).sqrt()})
}

struct Rec {
    build: usize,
    /// False when the start state was not clean or a verifier could not run; such runs are left out of the statistics.
    valid: bool,
    task: String,
    ok: bool,
    calls: usize,
    wall_s: f64,
    tokens: Option<f64>,
}

fn metrics(recs: &[&Rec]) -> Vec<(&'static str, Vec<f64>)> {
    let pick = |f: &dyn Fn(&Rec) -> Option<f64>| recs.iter().filter_map(|r| f(r)).collect::<Vec<_>>();
    vec![("success", pick(&|r| Some(r.ok as u8 as f64))), ("calls", pick(&|r| Some(r.calls as f64))), ("input_tokens", pick(&|r| r.tokens)), ("wall_s", pick(&|r| Some(r.wall_s)))]
}

fn report(recs: &[Rec]) -> Value {
    let by = |b: usize, task: Option<&str>| -> Vec<&Rec> { recs.iter().filter(|r| r.valid && r.build == b && task.is_none_or(|t| r.task == t)).collect() };
    let build = |b: usize, task: Option<&str>| Value::Object(metrics(&by(b, task)).into_iter().map(|(k, xs)| (k.to_string(), ci(&xs))).collect());
    let mut tasks: Vec<&str> = recs.iter().map(|r| r.task.as_str()).collect();
    tasks.sort_unstable();
    tasks.dedup();
    let (ma, mb) = (metrics(&by(0, None)), metrics(&by(1, None)));
    json!({
        "a": build(0, None), "b": build(1, None),
        "a_minus_b": Value::Object(ma.iter().zip(&mb).map(|((k, x), (_, y))| (k.to_string(), diff(x, y))).collect()),
        "tasks": tasks.iter().map(|t| json!({"task": t, "a": build(0, Some(t)), "b": build(1, Some(t))})).collect::<Vec<_>>(),
    })
}

fn table(r: &Value) -> String {
    let cell = |v: &Value, k: &str| format!("{:.2} ±{:.2}", v[k]["mean"].as_f64().unwrap_or(f64::NAN), v[k]["ci95"].as_f64().unwrap_or(f64::NAN));
    let mut out = String::from("metric        A (mean ±95% CI)    B (mean ±95% CI)    A-B\n");
    for k in ["success", "calls", "input_tokens", "wall_s"] {
        let d = &r["a_minus_b"][k];
        out +=
            &format!("{k:<13} {:<19} {:<19} {:+.2} ±{:.2}\n", cell(&r["a"], k), cell(&r["b"], k), d["diff"].as_f64().unwrap_or(f64::NAN), d["ci95"].as_f64().unwrap_or(f64::NAN));
    }
    out
}

/// Runs a reset or verifier command: `read ...`/`act ...` through `au`, or `adb ...` (`AU_ADB` or `adb`). `None` when it fails.
fn au(exe: &Path, cmd: &str) -> Option<String> {
    let (kind, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
    let out = if kind == "adb" {
        Command::new(env::var("AU_ADB").unwrap_or_else(|_| "adb".into())).args(rest.split_whitespace()).output()
    } else {
        Command::new(exe).args([kind, rest]).output()
    };
    out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Whether every verifier passes on the current device state; `None` when one cannot run.
fn verified(exe: &Path, task: &Task) -> Option<bool> {
    task.verify
        .iter()
        .try_fold(true, |all, (cmd, expect, not)| au(exe, cmd).map(|got| all && got.contains(expect.as_str()) && not.as_ref().is_none_or(|n| !got.contains(n.as_str()))))
}

/// Counting shim, started by the agent as its MCP server: relays stdio to `au serve --mcp` and logs each client line to `log`.
pub fn shim(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("bench-shim needs LOG AU".into());
    }
    let mut child = Command::new(&args[1]).args(["serve", "--mcp"]).stdin(Stdio::piped()).stdout(Stdio::inherit()).spawn().map_err(ioe)?;
    let (mut input, mut log) = (child.stdin.take().ok_or("no stdin")?, fs::OpenOptions::new().create(true).append(true).open(&args[0]).map_err(ioe)?);
    for line in BufReader::new(io::stdin()).lines() {
        let line = line.map_err(ioe)?;
        writeln!(log, "{line}").map_err(ioe)?;
        if writeln!(input, "{line}").is_err() {
            break;
        }
    }
    drop(input);
    child.wait().map(|_| ()).map_err(ioe)
}

fn opt(args: &[String], flag: &str, var: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned()).or_else(|| env::var(var).ok())
}

pub fn run(root: &Path) -> Result<(), String> {
    let args: Vec<String> = env::args().skip(2).collect();
    let get = |flag: &str, var: &str| opt(&args, flag, var);
    let builds: Vec<PathBuf> = ["--a", "--b"]
        .iter()
        .zip(["BENCH_A", "BENCH_B"])
        .map(|(f, v)| get(f, v).map(PathBuf::from).ok_or(format!("bench-model needs {f} PATH ({v})")))
        .collect::<Result<_, _>>()?;
    let template = get("--cmd", "BENCH_CMD").ok_or("bench-model needs --cmd TEMPLATE (BENCH_CMD), e.g. `claude -p {prompt} --mcp-config {mcp} --output-format json`")?;
    let num = |flag: &str, var: &str, default: f64| -> Result<f64, String> {
        get(flag, var).map_or(Ok(default), |s| s.parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0).ok_or(format!("{flag} needs a non-negative number, got `{s}`")))
    };
    let (runs, max_runs, max_cost, timeout) = (
        num("--runs", "BENCH_RUNS", 5.0)? as usize,
        num("--max-runs", "BENCH_MAX_RUNS", 200.0)? as usize,
        num("--max-cost", "BENCH_MAX_COST", 25.0)?,
        num("--timeout", "BENCH_TIMEOUT", 300.0)?,
    );
    let model = get("--model", "BENCH_MODEL").unwrap_or_default();
    let tasks_path = get("--tasks", "BENCH_TASKS").map(PathBuf::from).unwrap_or_else(|| root.join("tools/bench/tasks.json"));
    let mut tasks = parse_tasks(&fs::read_to_string(&tasks_path).map_err(ioe)?)?;
    if let Some(only) = get("--task", "BENCH_TASK") {
        tasks.retain(|t| t.name == only);
    }
    let order = schedule(tasks.len(), runs);
    println!("planned_runs={} max_runs={max_runs} max_cost={max_cost}", order.len());
    let (dir, me) = (root.join("target/bench-model"), env::current_exe().map_err(ioe)?);
    fs::create_dir_all(&dir).map_err(ioe)?;
    let (mut recs, mut spent, mut priciest, mut invalid) = (Vec::new(), 0.0, 0.0f64, 0usize);
    for (n, (round, ti, b)) in order.into_iter().enumerate() {
        if n >= max_runs || spent > max_cost {
            println!("spend guard: stopping after {n} runs (cost {spent:.2})");
            break;
        }
        let task = &tasks[ti];
        let tag = format!("{n}-{}-{}", task.name, ["a", "b"][b]);
        let (log, mcp, out) = (dir.join(format!("{tag}.calls")), dir.join(format!("{tag}.mcp.json")), dir.join(format!("{tag}.out")));
        let _ = fs::remove_file(&log);
        let server = json!({"mcpServers": {"android-use": {"command": me, "args": ["bench-shim", log, builds[b]]}}});
        fs::write(&mcp, server.to_string()).map_err(ioe)?;
        // A run counts only from a known start: every reset ran, and the verifiers fail before the agent acts.
        let reset = task.reset.iter().all(|cmd| au(&builds[0], cmd).is_some());
        let start_ok = reset && verified(&builds[0], task) == Some(false);
        let mut rec = Rec { build: b, task: task.name.clone(), valid: start_ok, ok: false, calls: 0, wall_s: 0.0, tokens: None };
        if !start_ok {
            println!("round={round} task={} build={} invalid: reset failed or the goal already holds before the agent acts", task.name, ["A", "B"][b]);
            invalid += 1;
            continue;
        }
        let argv = render(&template, &[("prompt", &task.goal), ("mcp", &mcp.to_string_lossy()), ("model", &model)]);
        let started = Instant::now();
        let mut child = Command::new(&argv[0]).args(&argv[1..]).stdout(fs::File::create(&out).map_err(ioe)?).stderr(Stdio::null()).stdin(Stdio::null()).spawn().map_err(ioe)?;
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(ioe)? {
                break status.code();
            }
            if started.elapsed().as_secs_f64() > timeout {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            thread::sleep(Duration::from_millis(200));
        };
        rec.wall_s = started.elapsed().as_secs_f64();
        let verdict = verified(&builds[0], task);
        let (tokens, cost) = usage(&fs::read_to_string(&out).unwrap_or_default());
        // A killed run printed no total: charge it the costliest run so far rather than nothing.
        spent += cost.unwrap_or(if timed_out { priciest } else { 0.0 });
        priciest = priciest.max(cost.unwrap_or(0.0));
        (rec.valid, rec.ok, rec.tokens) = (verdict.is_some(), verdict == Some(true), tokens);
        rec.calls = count_calls(&fs::read_to_string(&log).unwrap_or_default());
        println!("round={round} task={} build={} ok={} calls={} wall_s={:.1} exit={status:?} timed_out={timed_out}", task.name, ["A", "B"][b], rec.ok, rec.calls, rec.wall_s);
        invalid += !rec.valid as usize;
        recs.push(rec);
    }
    let rep = report(&recs);
    println!("{}", table(&rep));
    let path = get("--out", "BENCH_OUT").map(PathBuf::from).unwrap_or_else(|| root.join("target/bench-model.json"));
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({"a": builds[0], "b": builds[1], "runs": recs.len(), "invalid_runs": invalid, "cost_usd": spent, "report": rep})).map_err(ioe)?,
    )
    .map_err(ioe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_task_file_parses_with_a_verifier_per_task() {
        let tasks = parse_tasks(include_str!("../bench/tasks.json")).unwrap();
        assert!(tasks.len() >= 15 && tasks.iter().all(|t| !t.goal.is_empty() && !t.verify.is_empty()));
        assert!(parse_tasks(r#"{"tasks":[{"name":"x","goal":"g","verify":[]}]}"#).is_err());
        assert!(parse_tasks(r#"{"tasks":[{"name":"x","verify":[{"cmd":"read screen","expect":"a"}]}]}"#).is_err());
    }

    #[test]
    fn a_template_fills_placeholders_and_keeps_the_prompt_one_argument() {
        let argv = render("claude -p {prompt} --mcp-config {mcp} --model {model}", &[("prompt", "turn Wi-Fi off"), ("mcp", "/t/m.json"), ("model", "m1")]);
        assert_eq!(argv, ["claude", "-p", "turn Wi-Fi off", "--mcp-config", "/t/m.json", "--model", "m1"]);
    }

    #[test]
    fn calls_are_counted_from_the_shim_log() {
        let log = "{\"method\":\"initialize\"}\n{\"method\":\"tools/list\"}\n{\"method\":\"tools/call\",\"id\":1}\nnot json\n{\"method\":\"tools/call\",\"id\":2}\n";
        assert_eq!(count_calls(log), 2);
        assert_eq!(count_calls(""), 0);
    }

    #[test]
    fn usage_sums_input_token_fields_and_reads_cost() {
        let out = "{\"type\":\"system\"}\n{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":90,\"output_tokens\":5},\"total_cost_usd\":0.25}\n";
        assert_eq!(usage(out), (Some(100.0), Some(0.25)));
        assert_eq!(usage("plain text"), (None, None));
        // Nested cache breakdowns are not added twice, and Codex's cached count is already inside `input_tokens`.
        let nested =
            "{\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":1000,\"cache_creation\":{\"ephemeral_5m_input_tokens\":1000},\"cache_read_input_tokens\":5000}}";
        assert_eq!(usage(nested).0, Some(6010.0));
        assert_eq!(usage("{\"usage\":{\"input_tokens\":6000,\"cached_input_tokens\":5000}}").0, Some(6000.0));
    }

    #[test]
    fn runs_interleave_builds_and_flip_order_each_round() {
        let s = schedule(2, 2);
        assert_eq!(s.len(), 8);
        assert_eq!(s.iter().filter(|x| x.2 == 0).count(), 4);
        assert_eq!(s.iter().map(|x| x.0).collect::<Vec<_>>(), [0, 0, 0, 0, 1, 1, 1, 1]);
        assert_ne!(s[0].2, s[4].2);
    }

    #[test]
    fn confidence_intervals_use_the_t_distribution() {
        let c = ci(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!((c["mean"].as_f64().unwrap() - 3.0).abs() < 1e-9);
        assert!((c["ci95"].as_f64().unwrap() - 2.776 * (2.5f64 / 5.0).sqrt()).abs() < 1e-9);
        assert!(ci(&[1.0])["ci95"].as_f64().is_none());
        let d = diff(&[5.0, 6.0, 7.0], &[1.0, 2.0, 3.0]);
        assert!((d["diff"].as_f64().unwrap() - 4.0).abs() < 1e-9 && d["ci95"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn invalid_runs_stay_out_of_the_statistics() {
        let rec = |valid, ok| Rec { build: 0, valid, task: "t".into(), ok, calls: 1, wall_s: 1.0, tokens: None };
        assert_eq!(report(&[rec(true, false), rec(false, true)])["a"]["success"]["n"], 1);
    }

    #[test]
    fn the_report_compares_builds_per_task_and_overall() {
        let rec = |build, ok, calls, tokens| Rec { build, valid: true, task: "t".into(), ok, calls, wall_s: 10.0, tokens };
        let recs = [rec(0, true, 4, Some(1000.0)), rec(0, true, 6, Some(1200.0)), rec(1, false, 8, Some(2000.0)), rec(1, true, 10, Some(2400.0))];
        let r = report(&recs);
        assert_eq!(r["a"]["success"]["mean"], 1.0);
        assert_eq!(r["b"]["success"]["mean"], 0.5);
        assert_eq!(r["a_minus_b"]["calls"]["diff"], -4.0);
        assert_eq!(r["tasks"][0]["b"]["input_tokens"]["mean"], 2200.0);
        assert!(table(&r).contains("calls"));
    }
}
