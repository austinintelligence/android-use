use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adb::Adb;
use crate::batch::{self, BatchAction, Boundary};
use crate::cli::Cli;
use crate::config::{load, save, AppPaths, Config};
use crate::device::{DeviceInventory, EndpointKind};
use crate::error::{AuError, Result};
use crate::{app, helper, location, media, system, tape, trace, vision, web};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Brief {
    Ok,
    Count(u32),
    Path(String),
    Text(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ActionResult {
    pub brief: Brief,
    pub data: Value,
}

/// Explicit daemon-lifetime state for the fast path. Keeping the shell,
/// helper, CDP, tape, and selection caches together lets adjacent agent calls
/// reuse transport state without mutable globals.
#[derive(Default)]
pub struct DaemonRuntime {
    pub shell_pool: Option<crate::persistent::ShellPool>,
    pub helper_pool: Option<helper::HelperPool>,
    pub web_pool: Option<web::WebForwardPool>,
    pub tape_session: tape::TapeSession,
    pub selection: crate::device::SelectionCache,
}

impl ActionResult {
    fn ok(data: Value) -> Self {
        Self {
            brief: Brief::Ok,
            data,
        }
    }

    fn count(count: u32, data: Value) -> Self {
        Self {
            brief: Brief::Count(count),
            data,
        }
    }

    fn path(path: String, data: Value) -> Self {
        Self {
            brief: Brief::Path(path),
            data,
        }
    }

    pub fn text(value: impl Into<String>, data: Value) -> Self {
        Self {
            brief: Brief::Text(value.into()),
            data,
        }
    }
}

pub fn execute(cli: &Cli) -> Result<ActionResult> {
    trace::configure(cli.trace_path.as_deref(), cli.trace_id.as_deref())?;
    let _span = trace::span(
        "action.execute",
        json!({"c":cli.command,"a":cli.args.len(),"serial":cli.serial}),
    );
    let paths = AppPaths::discover()?;
    let mut config = load(&paths)?;
    let adb = Adb::from_config(&config, cli.timeout_ms)?;
    let result = dispatch(cli, &paths, &mut config, &adb);
    trace_result(&result);
    result
}

pub fn execute_daemon(
    cli: &Cli,
    paths: &AppPaths,
    config: &mut Config,
    runtime: &mut DaemonRuntime,
) -> Result<ActionResult> {
    trace::configure(cli.trace_path.as_deref(), cli.trace_id.as_deref())?;
    let _span = trace::span(
        "action.daemon_execute",
        json!({"c":cli.command,"a":cli.args.len(),"serial":cli.serial}),
    );
    let adb = Adb::from_config(config, cli.timeout_ms)?;
    let mut effective_cli = cli.clone();
    if command_uses_selection_cache(cli) {
        effective_cli.resolved_endpoint = Some(runtime.selection.resolve(
            &adb,
            config,
            cli.serial.as_deref(),
        )?);
    }
    let cli = &effective_cli;
    let result = match cli.command.as_str() {
        "b" | "batch" => run_batch_fast(
            cli,
            paths,
            config,
            &adb,
            &mut runtime.shell_pool,
            &mut runtime.helper_pool,
            &mut runtime.web_pool,
        ),
        "tape" | "x" if cli.disassemble => tape_disassemble(cli),
        "tape" | "x" => tape_fast(
            cli,
            paths,
            config,
            &adb,
            &mut runtime.shell_pool,
            &mut runtime.helper_pool,
            &mut runtime.tape_session,
        ),
        "t" | "tap" | "dt" | "lp" | "long" | "sw" | "swipe" | "dr" | "drag" | "tx" | "text"
        | "k" | "key" | "home" | "back" | "recents" | "notify" | "quick" | "wake" | "sleep"
        | "rot" => gui_fast(cli, config, &adb, &mut runtime.shell_pool),
        "exp" => experiment_with_pool(
            cli,
            paths,
            config,
            &adb,
            runtime
                .helper_pool
                .get_or_insert_with(helper::HelperPool::new),
        ),
        "ui" => semantic_ui_with_pool(
            cli,
            paths,
            config,
            &adb,
            Some(
                runtime
                    .helper_pool
                    .get_or_insert_with(helper::HelperPool::new),
            ),
        ),
        "web" => web_action_with_pool(
            cli,
            paths,
            config,
            &adb,
            Some(
                runtime
                    .web_pool
                    .get_or_insert_with(web::WebForwardPool::new),
            ),
        ),
        _ => dispatch(cli, paths, config, &adb),
    };
    if matches!(
        cli.command.as_str(),
        "u" | "use" | "p" | "pair" | "c" | "connect" | "dc" | "disconnect"
    ) || result.as_ref().is_err_and(|error| {
        matches!(
            error.kind(),
            "E_ADB" | "E_DEVICE" | "E_IDENTITY" | "E_SHELL"
        )
    }) {
        runtime.selection.invalidate();
    }
    trace_result(&result);
    result
}

fn trace_result(result: &Result<ActionResult>) {
    match result {
        Ok(result) => trace::event(
            "action.result",
            json!({"ok":true,"kind":match &result.brief { Brief::Ok => "ok", Brief::Count(_) => "count", Brief::Path(_) => "path", Brief::Text(_) => "text" }}),
        ),
        Err(error) => trace::event("action.result", json!({"ok":false,"e":error.kind()})),
    }
}

pub fn dispatch(
    cli: &Cli,
    paths: &AppPaths,
    config: &mut Config,
    adb: &Adb,
) -> Result<ActionResult> {
    match cli.command.as_str() {
        "d" | "devices" => devices(adb),
        "u" | "use" => use_endpoint(cli, paths, config, adb),
        "p" | "pair" => pair(adb, cli),
        "c" | "connect" => connect(adb, cli),
        "dc" | "disconnect" => disconnect(adb, cli),
        "st" | "status" => status(cli, config, adb),
        "cap" => capability(cli, paths, config, adb),
        "doctor" => doctor(cli, paths, config, adb),
        "exp" => experiment(cli, paths, config, adb),
        "b" | "batch" => run_batch(cli, paths, config, adb),
        "tape" | "x" => {
            if cli.disassemble {
                tape_disassemble(cli)
            } else {
                tape(cli, paths, config, adb)
            }
        }
        "pipe" => pipe(cli, paths, config, adb),
        "t" | "tap" | "dt" | "lp" | "long" | "sw" | "swipe" | "dr" | "drag" | "tx" | "text"
        | "k" | "key" | "home" | "back" | "recents" | "notify" | "quick" | "wake" | "sleep"
        | "rot" => gui(cli, config, adb),
        "ss" | "screenshot" | "mirror" | "screen" | "cam" | "mic" => {
            media_action(cli, paths, config, adb)
        }
        "ui" => semantic_ui(cli, paths, config, adb),
        "vision" => vision_action(cli, paths, config, adb),
        "web" => web_action(cli, paths, config, adb),
        "app" => app_action(cli, config, adb),
        "loc" => location_action(cli, paths, config, adb),
        "clip" | "notif" | "file" | "prop" | "settings" | "sys" | "log" | "ps" | "fwd" | "rev" => {
            system_action(cli, paths, config, adb)
        }
        "adb" => raw_adb(cli, config, adb),
        "sh" => raw_shell(cli, config, adb),
        other => Err(AuError::code("E_ARGS", format!("unknown command {other}"))),
    }
}

fn devices(adb: &Adb) -> Result<ActionResult> {
    let inventory = DeviceInventory::discover(adb)?;
    let count = inventory.endpoints.len() as u32;
    Ok(ActionResult::count(
        count,
        json!({"endpoints":inventory.endpoints,"identities":inventory.identities()}),
    ))
}

fn use_endpoint(
    cli: &Cli,
    paths: &AppPaths,
    config: &mut Config,
    adb: &Adb,
) -> Result<ActionResult> {
    let requested = required(&cli.args, 0, "u ENDPOINT")?;
    let inventory =
        DeviceInventory::discover_for_identity(adb, config.enrolled_serial().unwrap_or_default())?;
    let endpoint = inventory.resolve(config, Some(requested))?;
    let hardware_serial = endpoint.hardware_serial.clone().ok_or_else(|| {
        AuError::code(
            "E_IDENTITY",
            format!("endpoint {requested} did not report ro.serialno"),
        )
    })?;
    config.hardware_serial = hardware_serial.clone();
    config.selected_endpoint = Some(endpoint.endpoint.clone());
    if endpoint.kind == EndpointKind::Wifi
        && !config
            .known_wifi_endpoints
            .iter()
            .any(|item| item == &endpoint.endpoint)
    {
        config.known_wifi_endpoints.push(endpoint.endpoint.clone());
    }
    save(paths, config)?;
    Ok(ActionResult::ok(
        json!({"endpoint":endpoint.endpoint,"hardware_serial":hardware_serial,"enrolled":true}),
    ))
}

fn pair(adb: &Adb, cli: &Cli) -> Result<ActionResult> {
    let address = required(&cli.args, 0, "p HOST:PORT CODE")?;
    let code = required(&cli.args, 1, "p HOST:PORT CODE")?;
    adb.global(&["pair".into(), address.into(), code.into()])?;
    Ok(ActionResult::ok(json!({"paired":address})))
}

fn connect(adb: &Adb, cli: &Cli) -> Result<ActionResult> {
    let address = required(&cli.args, 0, "c HOST:PORT")?;
    adb.global(&["connect".into(), address.into()])?;
    Ok(ActionResult::ok(json!({"connected":address})))
}

fn disconnect(adb: &Adb, cli: &Cli) -> Result<ActionResult> {
    let address = required(&cli.args, 0, "dc HOST:PORT")?;
    adb.global(&["disconnect".into(), address.into()])?;
    Ok(ActionResult::ok(json!({"disconnected":address})))
}

fn status(cli: &Cli, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    let state = adb.device(&endpoint.endpoint, &["get-state".into()])?;
    let android = adb.device(
        &endpoint.endpoint,
        &[
            "shell".into(),
            "getprop".into(),
            "ro.build.version.release".into(),
        ],
    )?;
    Ok(ActionResult::ok(json!({
        "endpoint":endpoint.endpoint,
        "kind":endpoint.kind,
        "hardware_serial":endpoint.hardware_serial,
        "state":String::from_utf8_lossy(&state.stdout.bytes).trim(),
        "android":String::from_utf8_lossy(&android.stdout.bytes).trim()
    })))
}

fn capability(cli: &Cli, paths: &AppPaths, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    let helper = helper::capability(adb, &endpoint.endpoint)?;
    let camera = adb
        .device(
            &endpoint.endpoint,
            &["shell".into(), "cmd".into(), "camera".into(), "help".into()],
        )
        .is_ok();
    let ui = adb
        .device(
            &endpoint.endpoint,
            &["shell".into(), "uiautomator".into(), "help".into()],
        )
        .is_ok();
    Ok(ActionResult::ok(
        json!({"endpoint":endpoint.endpoint,"helper":helper,"camera_backend":camera,"uiautomator":ui,"artifact_dir":paths.artifacts}),
    ))
}

fn doctor(cli: &Cli, paths: &AppPaths, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let inventory =
        DeviceInventory::discover_for_identity(adb, config.enrolled_serial().unwrap_or_default())?;
    let selected_endpoint = if config.enrolled_serial().is_some() {
        inventory.resolve(config, cli.serial.as_deref()).ok()
    } else {
        None
    };
    let selection = match selected_endpoint.as_ref() {
        Some(endpoint) => json!({"ok":true,"endpoint":endpoint}),
        None if config.enrolled_serial().is_none() => {
            json!({"ok":false,"error":"E_ENROLL","message":"no Android device is enrolled"})
        }
        None => {
            json!({"ok":false,"error":"E_DEVICE","message":"enrolled device is offline or mismatched"})
        }
    };
    let (endpoint, helper, location) = if let Some(endpoint) = selected_endpoint.as_ref() {
        let helper = helper::capability(adb, &endpoint.endpoint)
            .map(|value| json!({"ok":true,"value":value}))
            .unwrap_or_else(
                |error| json!({"ok":false,"error":error.kind(),"message":error.to_string()}),
            );
        let location = location::status(adb, paths, &endpoint.endpoint, false)
            .map(|value| json!({"ok":true,"value":value}))
            .unwrap_or_else(
                |error| json!({"ok":false,"error":error.kind(),"message":error.to_string()}),
            );
        (json!(endpoint), helper, location)
    } else {
        (
            Value::Null,
            json!({"ok":false,"error":"E_ENROLL","message":"helper unavailable until enrollment"}),
            json!({"ok":false,"error":"E_ENROLL","message":"location unavailable until enrollment"}),
        )
    };
    let forwarding = fs::read_to_string(&paths.forwards).unwrap_or_else(|_| "[]".into());
    Ok(ActionResult::ok(json!({
        "endpoint":endpoint,
        "selection":selection,
        "helper":helper,
        "location":location,
        "tracked_forwards":serde_json::from_str::<Value>(&forwarding).unwrap_or_else(|_| json!({"corrupt":true})),
        "config":paths.config,
        "enrolled":config.enrolled_serial().is_some(),
        "inventory":inventory.endpoints
    })))
}

/// Versioned falsification experiment for the proof-carrying execution claim.
///
/// This deliberately stays on the existing JSON helper path. The helper owns
/// one bounded find.unique -> act -> wait -> assert transaction so the host
/// pays one authenticated frame rather than four sequential helper calls.
/// Binary model tapes, dictionaries, and direct ADB client code are
/// intentionally not part of this first gate.
fn experiment(cli: &Cli, paths: &AppPaths, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let (selector, postselector, timeout_ms) = experiment_args(cli)?;
    let endpoint = selected(cli, config, adb)?;
    let mut session = helper::HelperSession::open(adb, paths, &endpoint.endpoint)?;
    let outcome = run_f1_steps(selector, postselector, timeout_ms, |operation, args| {
        session.call_with_timeout(operation, args, Duration::from_millis(timeout_ms))
    });
    let close = session.close();
    let node_id = match outcome {
        Ok(node_id) => {
            close?;
            node_id
        }
        Err(error) => {
            let _ = close;
            return Err(error);
        }
    };
    Ok(f1_result(node_id, postselector))
}

fn experiment_with_pool(
    cli: &Cli,
    paths: &AppPaths,
    config: &Config,
    adb: &Adb,
    helper_pool: &mut helper::HelperPool,
) -> Result<ActionResult> {
    let (selector, postselector, timeout_ms) = experiment_args(cli)?;
    let endpoint = selected(cli, config, adb)?;
    let node_id = run_f1_steps(selector, postselector, timeout_ms, |operation, args| {
        helper_pool.call_with_timeout(
            adb,
            paths,
            &endpoint.endpoint,
            operation,
            args,
            Duration::from_millis(timeout_ms),
        )
    })?;
    Ok(f1_result(node_id, postselector))
}

fn experiment_args(cli: &Cli) -> Result<(&str, &str, u64)> {
    if cli.args.first().map(String::as_str) != Some("f1") {
        return Err(AuError::code(
            "E_ARGS",
            "exp f1 SELECTOR POSTSELECTOR [TIMEOUT_MS]",
        ));
    }
    let selector = required(&cli.args, 1, "exp f1 SELECTOR POSTSELECTOR [TIMEOUT_MS]")?;
    let postselector = required(&cli.args, 2, "exp f1 SELECTOR POSTSELECTOR [TIMEOUT_MS]")?;
    crate::selector::Selector::parse(selector)?;
    crate::selector::Selector::parse(postselector)?;
    let timeout_ms = cli
        .args
        .get(3)
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(5_000)
        .clamp(1, 30_000);
    Ok((selector, postselector, timeout_ms))
}

fn run_f1_steps<F>(selector: &str, postselector: &str, timeout_ms: u64, mut call: F) -> Result<u64>
where
    F: FnMut(&str, Value) -> Result<Value>,
{
    let found = call(
        "ui.proof",
        json!({"args":[selector, postselector, timeout_ms.to_string()]}),
    )?;
    let node_id = found
        .pointer("/node/id")
        .and_then(Value::as_u64)
        .ok_or_else(|| AuError::code("E_PROTOCOL", "ui.find omitted a node handle"))?;
    Ok(node_id)
}

fn f1_result(node_id: u64, postselector: &str) -> ActionResult {
    ActionResult::count(
        1,
        json!({
            "experiment":"f1",
            "version":1,
            "receipt":"find.unique>tap>wait>assert",
            "node":node_id,
            "postcondition":postselector
        }),
    )
}

fn tape(cli: &Cli, paths: &AppPaths, config: &mut Config, adb: &Adb) -> Result<ActionResult> {
    let mut shell_pool = Some(crate::persistent::ShellPool::new(adb.clone()));
    let mut helper_pool = None;
    let mut session = tape::TapeSession::default();
    tape_fast(
        cli,
        paths,
        config,
        adb,
        &mut shell_pool,
        &mut helper_pool,
        &mut session,
    )
}

fn tape_disassemble(cli: &Cli) -> Result<ActionResult> {
    let source = required(&cli.args, 0, "x TAPE_OR_FILE")?;
    let text = if Path::new(source).is_file() {
        fs::read_to_string(source)?
    } else {
        source.into()
    };
    let decoded = tape::disassemble(&text)?;
    let rendered = decoded.lines.join("\n");
    Ok(ActionResult::text(
        rendered,
        json!({
            "v": decoded.version,
            "expanded": decoded.expanded,
            "instructions": decoded.instructions,
            "state_actions": decoded.state_actions,
            "lines": decoded.lines,
        }),
    ))
}

fn tape_fast(
    cli: &Cli,
    paths: &AppPaths,
    config: &mut Config,
    adb: &Adb,
    pool: &mut Option<crate::persistent::ShellPool>,
    helper_pool: &mut Option<helper::HelperPool>,
    session: &mut tape::TapeSession,
) -> Result<ActionResult> {
    let source = required(&cli.args, 0, "x TAPE_OR_FILE")?;
    let text = if Path::new(source).is_file() {
        fs::read_to_string(source)?
    } else {
        source.into()
    };
    let program = tape::parse(&text)?;
    let endpoint = {
        let shell_pool = pool.get_or_insert_with(|| crate::persistent::ShellPool::new(adb.clone()));
        shell_pool.endpoint(cli, config)?.clone()
    };
    let mut registers = HashMap::<u8, String>::new();
    let mut pending_shell = Vec::<BatchAction>::new();
    let mut completed = 0u32;
    let mut evidence = None;

    for operation in program.ops {
        match operation {
            tape::Op::Dict { slot, value } => session.define(slot, value)?,
            tape::Op::Reset => {
                session.reset();
                registers.clear();
            }
            tape::Op::Home => pending_shell.push(BatchAction {
                command: "home".into(),
                args: Vec::new(),
                retries: 0,
                repeat: 1,
            }),
            tape::Op::Back => pending_shell.push(BatchAction {
                command: "back".into(),
                args: Vec::new(),
                retries: 0,
                repeat: 1,
            }),
            tape::Op::Key { key } => pending_shell.push(BatchAction {
                command: "k".into(),
                args: vec![resolve_tape_value(session, &key)?],
                retries: 0,
                repeat: 1,
            }),
            tape::Op::TapAt { x, y } => pending_shell.push(BatchAction {
                command: "t".into(),
                args: vec![
                    resolve_tape_value(session, &x)?,
                    resolve_tape_value(session, &y)?,
                ],
                retries: 0,
                repeat: 1,
            }),
            tape::Op::Find { slot, selector } => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let selector = resolve_tape_selector(session, &selector)?;
                let data = tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.find",
                    json!({"args":[selector,"--compact"]}),
                )?;
                let node = data
                    .get("node")
                    .and_then(Value::as_array)
                    .and_then(|values| values.first())
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AuError::code("E_PROTOCOL", "tape find omitted node handle"))?;
                registers.insert(slot, node.to_string());
            }
            tape::Op::Tap { target } => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let target = resolve_tape_target(session, &registers, &target)?;
                tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.tap",
                    json!({"args":[target]}),
                )?;
                completed += 1;
            }
            tape::Op::Long { target } => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let target = resolve_tape_target(session, &registers, &target)?;
                tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.long",
                    json!({"args":[target]}),
                )?;
                completed += 1;
            }
            tape::Op::Set { target, text } => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let target = resolve_tape_target(session, &registers, &target)?;
                let text = resolve_tape_literal(session, &text)?;
                tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.set",
                    json!({"args":[target,text]}),
                )?;
                completed += 1;
            }
            tape::Op::Scroll { target, direction } => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let target = resolve_tape_target(session, &registers, &target)?;
                tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.scroll",
                    json!({"args":[target,direction]}),
                )?;
                completed += 1;
            }
            tape::Op::Wait {
                selector,
                timeout_ms,
            } => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let selector = resolve_tape_selector(session, &selector)?;
                tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.wait",
                    json!({"args":[selector,timeout_ms.to_string()]}),
                )?;
            }
            tape::Op::Assert {
                selector,
                timeout_ms,
            } => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let selector = resolve_tape_selector(session, &selector)?;
                tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.assert",
                    json!({"args":[selector,timeout_ms.to_string()]}),
                )?;
            }
            tape::Op::Proof {
                selector,
                postcondition,
                timeout_ms,
            } => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let selector = resolve_tape_selector(session, &selector)?;
                let postcondition = resolve_tape_selector(session, &postcondition)?;
                let data = tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.proof",
                    json!({"args":[selector,postcondition,timeout_ms.to_string()]}),
                )?;
                evidence = Some(
                    data.get("proof")
                        .cloned()
                        .unwrap_or_else(|| json!("find.unique>tap>wait>assert")),
                );
                completed += 1;
            }
            tape::Op::Frontier => {
                completed +=
                    flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
                let data = tape_helper_call(
                    cli,
                    paths,
                    adb,
                    helper_pool,
                    &endpoint.endpoint,
                    "ui.snap",
                    json!({"args":["--compact","--frontier"]}),
                )?;
                evidence = Some(json!({"frontier":data}));
            }
            tape::Op::Repeat { .. } => {
                return Err(AuError::code(
                    "E_TAPE",
                    "repeat must be expanded before tape execution",
                ));
            }
        }
    }
    completed += flush_tape_shell(cli, adb, pool, &endpoint.endpoint, &mut pending_shell)?;
    let mut data = json!({
        "v": tape::TAPE_VERSION,
        "e": session.epoch,
        "h": session.checksum()
    });
    if let Some(evidence) = evidence {
        data["p"] = evidence;
    }
    Ok(ActionResult::count(completed, data))
}

fn flush_tape_shell(
    cli: &Cli,
    adb: &Adb,
    pool: &mut Option<crate::persistent::ShellPool>,
    serial: &str,
    pending: &mut Vec<BatchAction>,
) -> Result<u32> {
    if pending.is_empty() {
        return Ok(0);
    }
    let dimensions = if pending.iter().any(has_percentage_coordinate) {
        Some(screen_dimensions(adb, serial)?)
    } else {
        None
    };
    let normalized = pending
        .drain(..)
        .map(|action| normalize_action(action, dimensions))
        .collect::<Result<Vec<_>>>()?;
    let script = batch::lower_shell(&normalized)?;
    pool.as_mut()
        .ok_or_else(|| AuError::code("E_DAEMON", "tape shell pool disappeared"))?
        .transact(serial, &script, Duration::from_millis(cli.timeout_ms))?;
    Ok(normalized.len() as u32)
}

fn tape_helper_call(
    cli: &Cli,
    paths: &AppPaths,
    adb: &Adb,
    helper_pool: &mut Option<helper::HelperPool>,
    serial: &str,
    operation: &str,
    args: Value,
) -> Result<Value> {
    helper_pool
        .get_or_insert_with(helper::HelperPool::new)
        .call_with_timeout(
            adb,
            paths,
            serial,
            operation,
            args,
            Duration::from_millis(cli.timeout_ms),
        )
}

fn resolve_tape_selector(session: &tape::TapeSession, value: &str) -> Result<String> {
    if value.starts_with('$') {
        return Err(AuError::code(
            "E_TAPE",
            "selectors use dictionary refs @N, not node registers $N",
        ));
    }
    let selector = session.resolve(value)?;
    crate::selector::Selector::parse(&selector)?;
    Ok(selector)
}

fn resolve_tape_value(session: &tape::TapeSession, value: &str) -> Result<String> {
    if let Some(slot) = value.strip_prefix('$') {
        return Err(AuError::code(
            "E_TAPE",
            format!("register ${slot} is not valid for this operand"),
        ));
    }
    session.resolve(value)
}

fn resolve_tape_literal(session: &tape::TapeSession, value: &str) -> Result<String> {
    resolve_tape_value(session, value)
}

fn resolve_tape_target(
    session: &tape::TapeSession,
    registers: &HashMap<u8, String>,
    value: &str,
) -> Result<String> {
    if let Some(slot) = value.strip_prefix('$') {
        let slot = parse_tape_slot(slot)?;
        return registers
            .get(&slot)
            .cloned()
            .ok_or_else(|| AuError::code("E_TAPE", format!("register ${slot} is undefined")));
    }
    session.resolve(value)
}

fn parse_tape_slot(value: &str) -> Result<u8> {
    let slot = value.parse::<u8>()?;
    if slot < tape::MAX_DICTIONARY_ENTRIES as u8 {
        Ok(slot)
    } else {
        Err(AuError::code("E_TAPE", "tape slot must be 0..31"))
    }
}

fn run_batch(cli: &Cli, paths: &AppPaths, config: &mut Config, adb: &Adb) -> Result<ActionResult> {
    // --no-daemon changes ownership, not execution semantics. Keep one
    // persistent shell for this invocation so local and daemon batches share
    // quoting, framing, pacing, retry, and recovery behavior.
    let mut shell_pool = Some(crate::persistent::ShellPool::new(adb.clone()));
    let mut helper_pool = None;
    let mut web_pool = None;
    run_batch_fast(
        cli,
        paths,
        config,
        adb,
        &mut shell_pool,
        &mut helper_pool,
        &mut web_pool,
    )
}

fn run_batch_fast(
    cli: &Cli,
    paths: &AppPaths,
    config: &mut Config,
    adb: &Adb,
    pool: &mut Option<crate::persistent::ShellPool>,
    helper_pool: &mut Option<helper::HelperPool>,
    web_pool: &mut Option<web::WebForwardPool>,
) -> Result<ActionResult> {
    let source = required(&cli.args, 0, "b DSL_OR_FILE")?;
    let text = if Path::new(source).is_file() {
        fs::read_to_string(source)?
    } else {
        source.into()
    };
    let actions = batch::parse(&text)?;
    let endpoint = {
        let shell_pool = pool.get_or_insert_with(|| crate::persistent::ShellPool::new(adb.clone()));
        shell_pool.endpoint(cli, config)?.clone()
    };
    let dimensions = if actions.iter().any(has_percentage_coordinate) {
        Some(screen_dimensions(adb, &endpoint.endpoint)?)
    } else {
        None
    };
    let normalized = actions
        .into_iter()
        .map(|action| normalize_action(action, dimensions))
        .collect::<Result<Vec<_>>>()?;
    let mut completed = 0u32;
    let mut first_action = true;
    let mut index = 0usize;
    while index < normalized.len() {
        if batch::boundary(&normalized[index]) == Boundary::Shell {
            let end = normalized[index..]
                .iter()
                .position(|action| batch::boundary(action) != Boundary::Shell)
                .map(|offset| index + offset)
                .unwrap_or(normalized.len());
            let script = batch::lower_shell_with_delay(
                &normalized[index..end],
                cli.batch_delay_ms,
                !first_action,
            )?;
            pool.as_mut()
                .ok_or_else(|| AuError::code("E_DAEMON", "shell pool disappeared"))?
                .transact(
                    &endpoint.endpoint,
                    &script,
                    Duration::from_millis(cli.timeout_ms),
                )?;
            completed += normalized[index..end]
                .iter()
                .map(|action| u32::from(action.repeat))
                .sum::<u32>();
            first_action = false;
            index = end;
        } else {
            let action = &normalized[index];
            for _ in 0..action.repeat {
                let mut succeeded = false;
                for attempt in 0..=action.retries {
                    if !first_action && cli.batch_delay_explicit && cli.batch_delay_ms > 0 {
                        std::thread::sleep(Duration::from_millis(cli.batch_delay_ms));
                    }
                    first_action = false;
                    if action.command == "if"
                        && !conditional_matches(
                            paths,
                            adb,
                            &endpoint.endpoint,
                            required(&action.args, 0, "if ui:SELECTOR then ACTION")?,
                            helper_pool.as_mut(),
                        )?
                    {
                        succeeded = true;
                        break;
                    }
                    let nested_action = conditional_action(action).unwrap_or_else(|| {
                        batch::semantic_shorthand(action).unwrap_or_else(|| action.clone())
                    });
                    let nested = cli.child(nested_action.command, nested_action.args, &endpoint);
                    let mut context = NestedContext {
                        paths,
                        config,
                        adb,
                        pool,
                        helper_pool,
                        web_pool,
                        tape_session: None,
                    };
                    match execute_nested_with_pools(&nested, &mut context) {
                        Ok(_) => {
                            succeeded = true;
                            break;
                        }
                        Err(error)
                            if attempt < action.retries
                                && batch_semantic_retryable(action, &error) =>
                        {
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                }
                if succeeded {
                    completed += 1;
                }
            }
            index += 1;
        }
    }
    Ok(ActionResult::count(
        completed,
        json!({"completed":completed,"persistent_transaction":true}),
    ))
}

/// Execution resources retained by a foreground batch/pipe session.
struct NestedContext<'a> {
    paths: &'a AppPaths,
    config: &'a mut Config,
    adb: &'a Adb,
    pool: &'a mut Option<crate::persistent::ShellPool>,
    helper_pool: &'a mut Option<helper::HelperPool>,
    web_pool: &'a mut Option<web::WebForwardPool>,
    tape_session: Option<&'a mut tape::TapeSession>,
}

/// Execute one already-parsed command while retaining the foreground pools.
/// Batch actions and JSONL pipe requests must use the same dispatch table so
/// typed commands never accidentally fall through the batch-only executor.
fn execute_nested_with_pools(
    nested: &Cli,
    context: &mut NestedContext<'_>,
) -> Result<ActionResult> {
    match nested.command.as_str() {
        "b" | "batch" => run_batch_fast(
            nested,
            context.paths,
            context.config,
            context.adb,
            context.pool,
            context.helper_pool,
            context.web_pool,
        ),
        "tape" | "x" => {
            if let Some(session) = context.tape_session.as_deref_mut() {
                tape_fast(
                    nested,
                    context.paths,
                    context.config,
                    context.adb,
                    context.pool,
                    context.helper_pool,
                    session,
                )
            } else {
                dispatch(nested, context.paths, context.config, context.adb)
            }
        }
        "ui" => semantic_ui_with_pool(
            nested,
            context.paths,
            context.config,
            context.adb,
            Some(
                context
                    .helper_pool
                    .get_or_insert_with(helper::HelperPool::new),
            ),
        ),
        "web" => web_action_with_pool(
            nested,
            context.paths,
            context.config,
            context.adb,
            Some(
                context
                    .web_pool
                    .get_or_insert_with(web::WebForwardPool::new),
            ),
        ),
        "t" | "tap" | "dt" | "lp" | "long" | "sw" | "swipe" | "dr" | "drag" | "tx" | "text"
        | "k" | "key" | "home" | "back" | "recents" | "notify" | "quick" | "wake" | "sleep"
        | "rot" => gui_fast(nested, context.config, context.adb, context.pool),
        _ => dispatch(nested, context.paths, context.config, context.adb),
    }
}

fn conditional_action(action: &BatchAction) -> Option<BatchAction> {
    if action.command != "if" {
        return None;
    }
    Some(BatchAction {
        command: action.args.get(1)?.clone(),
        args: action.args.iter().skip(2).cloned().collect(),
        retries: 0,
        repeat: 1,
    })
}

/// Semantic retries are narrower than shell retries. A lost helper response
/// after a mutation may mean the mutation already happened; only read-only
/// and synchronization actions may be replayed automatically.
fn batch_semantic_retryable(action: &BatchAction, error: &AuError) -> bool {
    let operation = if action.command == "if" {
        action.args.get(1).map(String::as_str)
    } else if matches!(action.command.as_str(), "ui" | "web" | "app" | "loc") {
        action.args.first().map(String::as_str)
    } else {
        Some(action.command.as_str())
    };
    let read_only = matches!(
        operation,
        Some(
            "find"
                | "snap"
                | "wait"
                | "assert"
                | "watch"
                | "tabs"
                | "text"
                | "list"
                | "info"
                | "perm"
                | "status"
                | "get"
        )
    );
    read_only
        && matches!(
            error.kind(),
            "E_TIMEOUT" | "E_HELPER" | "E_PROTOCOL" | "E_DEVICE"
        )
}

fn conditional_matches(
    paths: &AppPaths,
    adb: &Adb,
    serial: &str,
    selector: &str,
    helper_pool: Option<&mut helper::HelperPool>,
) -> Result<bool> {
    let result = if let Some(pool) = helper_pool {
        pool.call(adb, paths, serial, "ui.find", json!({"args":[selector]}))
    } else {
        helper::call(adb, paths, serial, "ui.find", json!({"args":[selector]}))
    };
    match result {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == "E_UI" => Ok(false),
        Err(error) => Err(error),
    }
}

/// Run a foreground pipe with one warm execution context and emit each input
/// line's result as soon as its bounded batch completes. The callback is kept
/// outside the action engine so the binary can choose text, compact, wire, or
/// diagnostic rendering without duplicating execution semantics.
pub fn stream_pipe<F>(cli: &Cli, mut emit: F) -> Result<ActionResult>
where
    F: FnMut(std::result::Result<ActionResult, AuError>) -> Result<()>,
{
    let paths = AppPaths::discover()?;
    let mut config = load(&paths)?;
    let adb = Adb::from_config(&config, cli.timeout_ms)?;
    pipe_stream_with_context(cli, &paths, &mut config, &adb, &mut emit)
}

fn pipe(cli: &Cli, paths: &AppPaths, config: &mut Config, adb: &Adb) -> Result<ActionResult> {
    let mut discard = |_result: std::result::Result<ActionResult, AuError>| Ok(());
    pipe_stream_with_context(cli, paths, config, adb, &mut discard)
}

fn pipe_stream_with_context<F>(
    cli: &Cli,
    paths: &AppPaths,
    config: &mut Config,
    adb: &Adb,
    emit: &mut F,
) -> Result<ActionResult>
where
    F: FnMut(std::result::Result<ActionResult, AuError>) -> Result<()>,
{
    let stdin = io::stdin();
    let mut completed = 0u32;
    let mut shell_pool = Some(crate::persistent::ShellPool::new(adb.clone()));
    // Keep authenticated helper and CDP sessions alive across lines. Reusing
    // them is the difference between a persistent agent pipe and a loop of
    // cold-started commands.
    let mut helper_pool = None;
    let mut web_pool = None;
    let mut tape_session = tape::TapeSession::default();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let (command, args) = match pipe_line(cli, &line) {
            Ok(request) => request,
            Err(error) => {
                emit(Err(error))?;
                continue;
            }
        };
        let mut nested = cli.clone();
        nested.command = command;
        nested.args = args;
        nested.no_daemon = true;
        nested.daemon_child = true;
        let result = (|| -> Result<ActionResult> {
            nested.resolved_endpoint = if command_uses_selection_cache(&nested) {
                Some(
                    shell_pool
                        .get_or_insert_with(|| crate::persistent::ShellPool::new(adb.clone()))
                        .endpoint(&nested, config)?
                        .clone(),
                )
            } else {
                None
            };
            let mut context = NestedContext {
                paths,
                config,
                adb,
                pool: &mut shell_pool,
                helper_pool: &mut helper_pool,
                web_pool: &mut web_pool,
                tape_session: Some(&mut tape_session),
            };
            execute_nested_with_pools(&nested, &mut context)
        })();
        match result {
            Ok(result) => {
                if let Brief::Count(count) = &result.brief {
                    completed += *count;
                }
                emit(Ok(result))?;
            }
            Err(error) => {
                // A line is an independent bounded transaction. Return its
                // typed failure immediately, then keep the warm context alive
                // so the next line can perform the documented recovery.
                emit(Err(error))?;
            }
        }
    }
    Ok(ActionResult::count(
        completed,
        json!({"completed":completed,"mode":"foreground"}),
    ))
}

fn pipe_line(cli: &Cli, line: &str) -> Result<(String, Vec<String>)> {
    if !cli.pipe_jsonl {
        return Ok(("b".into(), vec![line.into()]));
    }
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|error| AuError::code("E_PIPE", format!("invalid JSONL request: {error}")))?;
    match value {
        Value::String(dsl) => Ok(("b".into(), vec![dsl])),
        Value::Object(object) => {
            if let Some(dsl) = object.get("b").and_then(Value::as_str) {
                return Ok(("b".into(), vec![dsl.into()]));
            }
            let command = object
                .get("c")
                .or_else(|| object.get("command"))
                .and_then(Value::as_str)
                .ok_or_else(|| AuError::code("E_PIPE", "JSONL request requires c or b"))?;
            let args_value = object
                .get("a")
                .or_else(|| object.get("args"))
                .cloned()
                .unwrap_or_else(|| json!([]));
            let args = serde_json::from_value::<Vec<String>>(args_value).map_err(|error| {
                AuError::code("E_PIPE", format!("JSONL args must be strings: {error}"))
            })?;
            Ok((command.into(), args))
        }
        _ => Err(AuError::code(
            "E_PIPE",
            "JSONL request must be a string or object",
        )),
    }
}

fn gui(cli: &Cli, config: &Config, adb: &Adb) -> Result<ActionResult> {
    // The one-shot path is the same engine used by the daemon. It owns the
    // shell only for this invocation, so it remains safely cancellable while
    // avoiding a second quoting and status implementation.
    let mut shell_pool = Some(crate::persistent::ShellPool::new(adb.clone()));
    gui_fast(cli, config, adb, &mut shell_pool)
}

fn gui_fast(
    cli: &Cli,
    config: &Config,
    adb: &Adb,
    pool: &mut Option<crate::persistent::ShellPool>,
) -> Result<ActionResult> {
    let shell_pool = pool.get_or_insert_with(|| crate::persistent::ShellPool::new(adb.clone()));
    let endpoint = shell_pool.endpoint(cli, config)?;
    let mut action = BatchAction {
        command: cli.command.clone(),
        args: cli.args.clone(),
        retries: 0,
        repeat: 1,
    };
    if has_percentage_coordinate(&action) {
        action = normalize_action(action, Some(screen_dimensions(adb, &endpoint.endpoint)?))?;
    }
    let script = batch::lower_shell(&[action])?;
    shell_pool.transact(
        &endpoint.endpoint,
        &script,
        Duration::from_millis(cli.timeout_ms),
    )?;
    Ok(ActionResult::ok(
        json!({"action":cli.command,"persistent":true}),
    ))
}

fn media_action(cli: &Cli, paths: &AppPaths, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    let data = media::execute(
        adb,
        paths,
        &endpoint.endpoint,
        &cli.command,
        &cli.args,
        media::MediaOptions {
            output: cli.output_path.as_deref(),
            force: cli.force,
            binary: cli.output.binary,
        },
    )?;
    if let Some(path) = data.get("path").and_then(Value::as_str) {
        return Ok(ActionResult::path(path.into(), data));
    }
    Ok(ActionResult::ok(data))
}

fn semantic_ui(cli: &Cli, paths: &AppPaths, config: &Config, adb: &Adb) -> Result<ActionResult> {
    semantic_ui_with_pool(cli, paths, config, adb, None)
}

fn semantic_ui_with_pool(
    cli: &Cli,
    paths: &AppPaths,
    config: &Config,
    adb: &Adb,
    helper_pool: Option<&mut helper::HelperPool>,
) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    let operation = cli.args.first().map(String::as_str).unwrap_or("snap");
    let args = &cli.args[usize::from(!cli.args.is_empty())..];
    if matches!(
        operation,
        "find" | "tap" | "long" | "set" | "scroll" | "wait" | "assert"
    ) {
        if let Some(selector) = args
            .first()
            .filter(|value| !value.chars().all(|character| character.is_ascii_digit()))
        {
            crate::selector::Selector::parse(selector)?;
        }
    }
    let mut helper_args = args.to_vec();
    if cli.output.compact
        && matches!(operation, "snap" | "find")
        && !helper_args.iter().any(|arg| arg == "--compact")
    {
        helper_args.push("--compact".into());
    }
    if operation == "gesture" {
        if !(4..=5).contains(&helper_args.len()) {
            return Err(AuError::code("E_ARGS", "ui gesture X1 Y1 X2 Y2 [MS]"));
        }
        let dimensions = if helper_args.iter().take(4).any(|value| value.ends_with('%')) {
            Some(screen_dimensions(adb, &endpoint.endpoint)?)
        } else {
            None
        };
        for (index, value) in helper_args.iter_mut().enumerate().take(4) {
            let extent = dimensions
                .map(|(width, height)| if index % 2 == 0 { width } else { height })
                .unwrap_or(0);
            *value = coordinate(value, extent)?.to_string();
        }
    }
    let operation = format!("ui.{operation}");
    let data = if let Some(pool) = helper_pool {
        pool.call_with_timeout(
            adb,
            paths,
            &endpoint.endpoint,
            &operation,
            json!({"args":helper_args}),
            Duration::from_millis(cli.timeout_ms),
        )?
    } else {
        helper::call(
            adb,
            paths,
            &endpoint.endpoint,
            &operation,
            json!({"args":helper_args}),
        )?
    };
    Ok(ActionResult::ok(data))
}

fn vision_action(cli: &Cli, paths: &AppPaths, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    let data = vision::execute(
        adb,
        paths,
        &endpoint.endpoint,
        &cli.args,
        cli.output_path.as_deref(),
        cli.force,
    )?;
    if let Some(path) = data.get("path").and_then(Value::as_str) {
        return Ok(ActionResult::path(path.into(), data));
    }
    Ok(ActionResult::ok(data))
}

fn web_action(cli: &Cli, paths: &AppPaths, config: &Config, adb: &Adb) -> Result<ActionResult> {
    web_action_with_pool(cli, paths, config, adb, None)
}

fn web_action_with_pool(
    cli: &Cli,
    paths: &AppPaths,
    config: &Config,
    adb: &Adb,
    pool: Option<&mut web::WebForwardPool>,
) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    let data = match pool {
        Some(pool) => web::execute_with_pool(
            adb,
            paths,
            &endpoint.endpoint,
            &cli.args,
            cli.output_path.as_deref(),
            cli.force,
            pool,
        )?,
        None => web::execute(
            adb,
            paths,
            &endpoint.endpoint,
            &cli.args,
            cli.output_path.as_deref(),
            cli.force,
        )?,
    };
    if let Some(path) = data.get("path").and_then(Value::as_str) {
        return Ok(ActionResult::path(path.into(), data));
    }
    Ok(ActionResult::ok(data))
}

fn app_action(cli: &Cli, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    Ok(ActionResult::ok(app::execute(
        adb,
        &endpoint.endpoint,
        &cli.args,
    )?))
}

fn location_action(
    cli: &Cli,
    paths: &AppPaths,
    config: &Config,
    adb: &Adb,
) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    Ok(ActionResult::ok(location::execute(
        adb,
        paths,
        &endpoint.endpoint,
        &cli.args,
    )?))
}

fn system_action(cli: &Cli, paths: &AppPaths, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let endpoint = selected(cli, config, adb)?;
    let data = system::execute(
        adb,
        paths,
        &endpoint.endpoint,
        endpoint
            .hardware_serial
            .as_deref()
            .unwrap_or(&endpoint.endpoint),
        &cli.command,
        &cli.args,
        system::SystemOptions {
            output: cli.output_path.as_deref(),
            force: cli.force,
        },
    )?;
    if let Some(path) = data.get("path").and_then(Value::as_str) {
        return Ok(ActionResult::path(path.into(), data));
    }
    Ok(ActionResult::ok(data))
}

fn raw_adb(cli: &Cli, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let mut arguments = cli.args.clone();
    let global = arguments.first().is_some_and(|value| value == "-g");
    if global {
        arguments.remove(0);
    }
    if arguments.first().is_some_and(|value| value == "--") {
        arguments.remove(0);
    } else {
        return Err(AuError::code(
            "E_ARGS",
            "raw adb requires --; use au adb -- … or au adb -g -- …",
        ));
    }
    if arguments.is_empty() {
        return Err(AuError::code(
            "E_ARGS",
            "raw adb requires arguments after --",
        ));
    }
    let result = if global {
        adb.global(&arguments)?
    } else {
        let endpoint = selected(cli, config, adb)?;
        adb.device(&endpoint.endpoint, &arguments)?
    };
    let value = String::from_utf8_lossy(&result.stdout.bytes)
        .trim()
        .chars()
        .take(16_000)
        .collect::<String>();
    Ok(ActionResult::ok(
        json!({"stdout":value,"truncated":result.stdout.truncated,"bytes":result.stdout.total_bytes}),
    ))
}

fn raw_shell(cli: &Cli, config: &Config, adb: &Adb) -> Result<ActionResult> {
    let mut arguments = cli.args.clone();
    if arguments.first().is_some_and(|value| value == "--") {
        arguments.remove(0);
    } else {
        return Err(AuError::code(
            "E_ARGS",
            "raw sh requires --; raw shell is unrestricted and not a safety boundary",
        ));
    }
    if arguments.is_empty() {
        return Err(AuError::code(
            "E_ARGS",
            "raw sh requires arguments after --",
        ));
    }
    let endpoint = selected(cli, config, adb)?;
    let result = adb.raw_shell(&endpoint.endpoint, &arguments)?;
    let value = String::from_utf8_lossy(&result.stdout.bytes)
        .trim()
        .chars()
        .take(16_000)
        .collect::<String>();
    Ok(ActionResult::ok(
        json!({"stdout":value,"truncated":result.stdout.truncated,"bytes":result.stdout.total_bytes}),
    ))
}

fn command_uses_selection_cache(cli: &Cli) -> bool {
    if matches!(
        cli.command.as_str(),
        "d" | "devices"
            | "u"
            | "use"
            | "p"
            | "pair"
            | "c"
            | "connect"
            | "dc"
            | "disconnect"
            | "b"
            | "batch"
            | "tape"
            | "x"
            | "pipe"
            | "daemon"
    ) {
        return false;
    }
    if cli.command == "adb" {
        return cli.args.first().is_none_or(|arg| arg != "-g");
    }
    true
}

fn selected(cli: &Cli, config: &Config, adb: &Adb) -> Result<crate::device::Endpoint> {
    if let Some(endpoint) = cli.resolved_endpoint.as_ref() {
        if config.identity_matches(endpoint.hardware_serial.as_deref())
            && crate::device::endpoint_matches_requested(endpoint, cli.serial.as_deref())
        {
            return Ok(endpoint.clone());
        }
        return Err(AuError::code(
            "E_IDENTITY",
            "validated batch endpoint no longer matches the requested hardware",
        ));
    }
    let inventory =
        DeviceInventory::discover_for_identity(adb, config.enrolled_serial().unwrap_or_default())?;
    inventory.resolve(config, cli.serial.as_deref())
}

fn required<'a>(args: &'a [String], index: usize, usage: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| AuError::code("E_ARGS", usage))
}

fn screen_dimensions(adb: &Adb, serial: &str) -> Result<(i32, i32)> {
    // `wm size` reports the panel's natural orientation on some tablets. Input
    // injection, screenshots, and accessibility bounds instead use the active
    // logical viewport. Prefer InputManager's viewport so percentage coordinates
    // stay aligned after a rotation on tablets that expose a landscape desktop mode.
    if let Ok(result) = adb.device(serial, &["shell".into(), "dumpsys".into(), "input".into()]) {
        if let Some(dimensions) =
            parse_input_viewport(&String::from_utf8_lossy(&result.stdout.bytes))
        {
            return Ok(dimensions);
        }
    }
    let result = adb.device(serial, &["shell".into(), "wm".into(), "size".into()])?;
    let line = String::from_utf8_lossy(&result.stdout.bytes)
        .lines()
        .find(|line| line.contains('x'))
        .ok_or_else(|| AuError::code("E_GUI", "wm size returned no dimensions"))?
        .to_owned();
    let dimensions = line
        .split_whitespace()
        .find(|part| part.contains('x'))
        .ok_or_else(|| AuError::code("E_GUI", "wm size returned malformed dimensions"))?;
    let (width, height) = dimensions
        .split_once('x')
        .ok_or_else(|| AuError::code("E_GUI", "invalid wm size"))?;
    Ok((width.parse()?, height.parse()?))
}

fn parse_input_viewport(input: &str) -> Option<(i32, i32)> {
    let line = input
        .lines()
        .find(|line| line.contains("Viewport INTERNAL:") && line.contains("logicalFrame=["))?;
    let frame = line.split("logicalFrame=[").nth(1)?.split(']').next()?;
    let values = frame
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    match values.as_slice() {
        [left, top, right, bottom] if right > left && bottom > top => {
            Some((right - left, bottom - top))
        }
        _ => None,
    }
}

fn has_percentage_coordinate(action: &BatchAction) -> bool {
    matches!(
        action.command.as_str(),
        "t" | "tap" | "dt" | "lp" | "long" | "sw" | "swipe" | "dr" | "drag"
    ) && action.args.iter().any(|value| value.ends_with('%'))
}

fn normalize_action(
    mut action: BatchAction,
    dimensions: Option<(i32, i32)>,
) -> Result<BatchAction> {
    let Some((width, height)) = dimensions else {
        return Ok(action);
    };
    let positions: &[usize] = match action.command.as_str() {
        "t" | "tap" | "dt" | "lp" | "long" => &[0, 1],
        "sw" | "swipe" | "dr" | "drag" => &[0, 1, 2, 3],
        _ => &[],
    };
    for position in positions {
        if let Some(value) = action.args.get_mut(*position) {
            let extent = if position % 2 == 0 { width } else { height };
            *value = coordinate(value, extent)?.to_string();
        }
    }
    Ok(action)
}

fn coordinate(value: &str, extent: i32) -> Result<i32> {
    if let Some(percent) = value.strip_suffix('%') {
        let percent: f64 = percent.parse()?;
        if !(0.0..=100.0).contains(&percent) {
            return Err(AuError::code(
                "E_ARGS",
                "percentage coordinate must be 0..100%",
            ));
        }
        return Ok((extent as f64 * percent / 100.0).round() as i32);
    }
    let coordinate: i32 = value.parse()?;
    if coordinate < 0 {
        return Err(AuError::code("E_ARGS", "coordinate must be non-negative"));
    }
    Ok(coordinate)
}

#[cfg(test)]
mod tests {
    use super::{coordinate, parse_input_viewport, pipe_line};
    use crate::cli::Cli;

    #[test]
    fn percentage_coordinates_are_resolved_locally() {
        assert_eq!(coordinate("50%", 800).expect("coordinate"), 400);
        assert_eq!(coordinate("0%", 1280).expect("coordinate"), 0);
    }

    #[test]
    fn percentage_coordinates_follow_the_active_rotated_viewport() {
        let input = "Viewport INTERNAL: displayId=0, orientation=1, logicalFrame=[0, 0, 1280, 800], physicalFrame=[0, 0, 1280, 800]";
        assert_eq!(parse_input_viewport(input), Some((1280, 800)));
    }

    #[test]
    fn jsonl_pipe_requests_preserve_typed_command_boundaries() {
        let cli = Cli::parse(vec!["pipe".into(), "--jsonl".into()]).expect("pipe mode");
        assert_eq!(
            pipe_line(&cli, r#"{"c":"ui","a":["snap","--compact"]}"#).expect("jsonl request"),
            ("ui".into(), vec!["snap".into(), "--compact".into()])
        );
        assert_eq!(
            pipe_line(&cli, r#"{"b":"home; back"}"#).expect("dsl shorthand"),
            ("b".into(), vec!["home; back".into()])
        );
        assert_eq!(
            pipe_line(&cli, "not-json")
                .expect_err("invalid JSONL")
                .kind(),
            "E_PIPE"
        );
    }
}
