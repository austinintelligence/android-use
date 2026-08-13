use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::actions::{self, ActionResult, Brief};
use crate::cli::Cli;
use crate::config::{atomic_write, load, AppPaths, Config};
use crate::contract::{
    self, DeviceRef, ExecuteParams, ObserveParams, PlanStep, Request, StatusParams,
};
use crate::error::{AuError, Result};
use crate::recipe;
use crate::{CONTRACT_VERSION, MAX_OUTPUT_BYTES};

static OBSERVATION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Warm, explicit state for one JSONL/MCP contract lifetime.
///
/// The v2 server used to parse back into the cold CLI dispatcher for every
/// observation and every plan step. Retaining the daemon runtime here keeps
/// device selection, ADB shell, helper, and CDP transports warm without a
/// process-global mutable singleton.
pub struct ContractRuntime {
    paths: AppPaths,
    config: Config,
    daemon: actions::DaemonRuntime,
    capability_cache: HashMap<String, (Instant, Value)>,
    observations: HashMap<String, ObservationRecord>,
}

struct ObservationRecord {
    generation: u64,
    scope: String,
    created: Instant,
}

struct CompiledHelperPlan {
    args: Vec<String>,
    validated: crate::plan::ValidatedPlan,
}

impl ContractRuntime {
    pub fn new() -> Result<Self> {
        let paths = AppPaths::discover()?;
        let config = load(&paths)?;
        Ok(Self {
            paths,
            config,
            daemon: actions::DaemonRuntime::default(),
            capability_cache: HashMap::new(),
            observations: HashMap::new(),
        })
    }

    pub fn handle(&mut self, request: &Request) -> Result<Value> {
        handle_with(self, request)
    }

    pub fn contract_response(&mut self, request: &Request) -> contract::Response {
        match self.handle(request) {
            Ok(result) => contract::ok(request.id.clone(), result),
            Err(error) => contract::error(request.id.clone(), &error),
        }
    }

    fn invoke(&mut self, args: Vec<String>) -> Result<ActionResult> {
        let cli = Cli::parse(args)?;
        actions::execute_daemon(&cli, &self.paths, &mut self.config, &mut self.daemon)
    }
}

pub fn handle(request: &Request) -> Result<Value> {
    ContractRuntime::new()?.handle(request)
}

fn handle_with(runtime: &mut ContractRuntime, request: &Request) -> Result<Value> {
    match request.method.as_str() {
        "android.status" => {
            let params: StatusParams =
                serde_json::from_value(request.params.clone()).map_err(|error| {
                    AuError::code("E_ARGS", format!("invalid status params: {error}"))
                })?;
            invoke_status(runtime, params)
        }
        "android.observe" => {
            let params: ObserveParams =
                serde_json::from_value(request.params.clone()).map_err(|error| {
                    AuError::code("E_ARGS", format!("invalid observe params: {error}"))
                })?;
            observe(runtime, params)
        }
        "android.execute" => {
            let mut params: ExecuteParams = serde_json::from_value(request.params.clone())
                .map_err(|error| {
                    AuError::code("E_ARGS", format!("invalid execute params: {error}"))
                })?;
            if params.operation_id.is_none() {
                params.operation_id = Some(request.id.clone());
            }
            execute_plan_with(runtime, params)
        }
        "android.artifact" => artifact(&request.params),
        "android.recipe" => recipe::contract_call(&request.params),
        method => Err(AuError::code(
            "E_PROTOCOL",
            format!("unsupported method {method}"),
        )),
    }
}

pub fn device_free(command: &str, args: &[String]) -> Result<Option<ActionResult>> {
    match command {
        "schema" => Ok(Some(ActionResult {
            brief: Brief::Ok,
            data: contract::schema(),
        })),
        "agent" => Ok(Some(crate::agent::action(args)?)),
        "recipe" if matches!(args.first().map(String::as_str), Some("list" | "show")) => {
            Ok(Some(recipe::action(&AppPaths::discover()?, args)?))
        }
        _ => Ok(None),
    }
}

pub fn contract_response(request: &Request) -> contract::Response {
    match ContractRuntime::new() {
        Ok(mut runtime) => runtime.contract_response(request),
        Err(error) => contract::error(request.id.clone(), &error),
    }
}

fn invoke_status(runtime: &mut ContractRuntime, params: StatusParams) -> Result<Value> {
    let device: DeviceRef = params.device;
    let selector = device_selector(&device)?;
    let mut args = vec!["--no-daemon".into(), "-j".into()];
    if let Some(selector) = selector.as_deref() {
        args.extend(["-s".into(), selector.into()]);
    }
    args.push("st".into());
    let result = runtime.invoke(args)?;
    let mut capability_args = vec!["--no-daemon".into(), "-j".into()];
    if let Some(selector) = selector.as_deref() {
        capability_args.extend(["-s".into(), selector.into()]);
    }
    capability_args.push("cap".into());
    let cache_key = selector.clone().unwrap_or_else(|| "enrolled".into());
    let capabilities = if !params.fresh {
        runtime
            .capability_cache
            .get(&cache_key)
            .filter(|(at, _)| at.elapsed() <= Duration::from_secs(2))
            .map(|(_, value)| value.clone())
    } else {
        None
    }
    .unwrap_or_else(|| {
        let value = runtime
            .invoke(capability_args)
            .map(|value| value.data)
            .unwrap_or_else(|error| json!({"available":false,"error":error.kind()}));
        runtime
            .capability_cache
            .insert(cache_key, (Instant::now(), value.clone()));
        value
    });
    let ready = result
        .data
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|state| state == "device")
        && result
            .data
            .get("hardware_serial")
            .and_then(Value::as_str)
            .is_some_and(|serial| !serial.is_empty());
    Ok(json!({"v":CONTRACT_VERSION,"ready":ready,"data":result.data,"capabilities":capabilities}))
}

fn observe(runtime: &mut ContractRuntime, params: ObserveParams) -> Result<Value> {
    if params.mode != "delta" && params.base_observation.is_some() {
        return Err(AuError::code(
            "E_ARGS",
            "base_observation is valid only for delta mode",
        ));
    }
    let scope = observation_scope(&params.device)?;
    let expected_base = if params.mode == "delta" {
        params
            .base_observation
            .as_deref()
            .map(|base| {
                let record = runtime.observations.get(base).ok_or_else(|| {
                    AuError::code("E_STALE", "base observation is unknown or expired")
                })?;
                if record.scope != scope {
                    return Err(AuError::code(
                        "E_IDENTITY",
                        "base observation belongs to a different device scope",
                    ));
                }
                Ok(record.generation)
            })
            .transpose()?
    } else {
        None
    };
    let mut args = vec!["--no-daemon".into(), "-j".into()];
    append_device_selector(&mut args, &params.device)?;
    args.push("ui".into());
    match params.mode.as_str() {
        "choices" | "frontier" => args.extend([
            "snap".into(),
            "--compact".into(),
            "--frontier".into(),
            "--contract".into(),
        ]),
        "delta" => args.extend(["snap".into(), "--compact".into(), "--delta".into()]),
        "expanded" | "context" => args.extend(["snap".into()]),
        "query" => {
            args.push("find".into());
            args.push(
                params
                    .query
                    .ok_or_else(|| AuError::code("E_ARGS", "query mode requires query"))?,
            );
        }
        other => {
            return Err(AuError::code(
                "E_ARGS",
                format!("unknown observe mode {other}"),
            ))
        }
    }
    let mut result = runtime.invoke(args)?;
    if expected_base.is_some_and(|expected| !delta_matches_base(&result.data, expected)) {
        let mut reset_args = vec!["--no-daemon".into(), "-j".into()];
        append_device_selector(&mut reset_args, &params.device)?;
        reset_args.extend(["ui".into(), "snap".into(), "--compact".into()]);
        result = runtime.invoke(reset_args)?;
    }
    let observation = format!("o{}", OBSERVATION_COUNTER.fetch_add(1, Ordering::Relaxed));
    let generation = result
        .data
        .get("g")
        .or_else(|| result.data.get("generation"))
        .cloned()
        .unwrap_or(Value::Null);
    let normalized = normalize_observation(
        &result.data,
        &params.mode,
        params.budget.nodes.unwrap_or(64),
        &params.encoding,
    );
    let output = if params.encoding == "dense" {
        json!({"v":CONTRACT_VERSION,"o":observation,"g":generation,"m":params.mode,"d":normalized})
    } else {
        json!({
            "v": CONTRACT_VERSION,
            "obs": observation,
            "g": generation,
            "mode": params.mode,
            "data": normalized,
            "redaction": "compatibility-helper"
        })
    };
    let bytes = serde_json::to_vec(&output)?.len();
    let budget_bytes = params
        .budget
        .bytes
        .unwrap_or(12 * 1024)
        .min(MAX_OUTPUT_BYTES);
    ensure_observation_budget(bytes, budget_bytes, &params.mode)?;
    if let Some(generation) = generation.as_u64() {
        if runtime.observations.len() >= 256 {
            if let Some(oldest) = runtime
                .observations
                .iter()
                .min_by_key(|(_, record)| record.created)
                .map(|(key, _)| key.clone())
            {
                runtime.observations.remove(&oldest);
            }
        }
        runtime.observations.insert(
            observation,
            ObservationRecord {
                generation,
                scope,
                created: Instant::now(),
            },
        );
    }
    Ok(output)
}

fn ensure_observation_budget(bytes: usize, budget: usize, mode: &str) -> Result<()> {
    if bytes <= budget {
        return Ok(());
    }
    Err(AuError::code(
        "E_OUTPUT_LIMIT",
        "observation exceeds transcript budget; use query/frontier or raise the budget",
    )
    .with_details(json!({
        "bytes": bytes,
        "budget": budget,
        "mode": mode,
        "next": "use query/frontier or raise budget"
    })))
}

fn observation_scope(device: &DeviceRef) -> Result<String> {
    let paths = AppPaths::discover()?;
    let config = load(&paths)?;
    Ok(serde_json::to_string(&json!({
        "enrolled": config.enrolled_serial(),
        "device": device
    }))?)
}

fn delta_matches_base(data: &Value, expected: u64) -> bool {
    data.get("n").and_then(Value::as_array).is_some()
        || data.get("base").and_then(Value::as_u64) == Some(expected)
        || (data.get("same").and_then(Value::as_bool) == Some(true)
            && data.get("g").and_then(Value::as_u64) == Some(expected))
}

pub fn execute_plan(params: ExecuteParams) -> Result<Value> {
    let mut runtime = ContractRuntime::new()?;
    execute_plan_with(&mut runtime, params)
}

fn execute_plan_with(runtime: &mut ContractRuntime, params: ExecuteParams) -> Result<Value> {
    validate_plan(&params)?;
    let operation_id = params.operation_id.clone().unwrap_or_else(new_operation_id);
    validate_operation_id(&operation_id)?;
    let binding = operation_binding(runtime, &params)?;
    let paths = AppPaths::discover()?;
    if let Some(record) = load_operation(&paths, &operation_id)? {
        let previous = validate_operation_record(&record, &binding)?;
        if previous
            .get("unknown_commit")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(AuError::code(
                "E_UNKNOWN_COMMIT",
                "operation id was previously left with an unknown outcome; observe before continuing",
            )
            .with_details(previous));
        }
        if previous
            .get("partial")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(AuError::code(
                "E_PARTIAL",
                "operation id already has a partial outcome; observe and issue a new operation",
            )
            .with_details(previous));
        }
        return Ok(json!({
            "v": CONTRACT_VERSION,
            "operation_id": operation_id,
            "replayed": true,
            "receipt": previous
        }));
    }
    let started = Instant::now();
    let deadline = Duration::from_millis(params.deadline_ms);
    let _ = device_selector(&params.device)?;
    if let Some(identity) = params.expected_identity.as_deref() {
        if !device_identity_matches(&params.device, identity)? {
            return Err(AuError::code(
                "E_IDENTITY",
                "expected identity does not match the enrolled hardware identity",
            ));
        }
    }
    verify_conditions(
        runtime,
        &params,
        &params.preconditions,
        "precondition",
        deadline,
        started,
    )?;
    if let Some(expected_generation) = params.expected_generation {
        let observed = observe(
            runtime,
            ObserveParams {
                device: params.device.clone(),
                mode: "choices".into(),
                ..ObserveParams::default()
            },
        )?;
        let actual = observed.get("g").and_then(Value::as_u64);
        if actual != Some(expected_generation) {
            return Err(AuError::code(
                "E_STALE",
                "plan generation is stale; observe again",
            ));
        }
    }

    // Compile the common find -> tap -> wait/assert shape into the helper's
    // single proof transaction. The semantic contract remains transport
    // neutral; this optimization is deliberately selected only when every
    // operand is a selector, so a stable reference never gets reinterpreted
    // as a selector after a mutation.
    if let Some(args) = proof_args(&params, deadline, started)? {
        match runtime.invoke(args) {
            Ok(result) => {
                if let Err(error) = verify_conditions(
                    runtime,
                    &params,
                    &params.postconditions,
                    "postcondition",
                    deadline,
                    started,
                ) {
                    save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                    return Err(attach_unknown(error, &operation_id));
                }
                let output = json!({
                    "v": CONTRACT_VERSION,
                    "operation_id": operation_id,
                    "committed": true,
                    "unknown_commit": false,
                    "mutations": 1,
                    "compiled": "helper-proof",
                    "steps": [{"index":0,"op":"proof","status":"committed","data":result.data}]
                });
                save_operation(&paths, &operation_id, &binding, &output)?;
                return Ok(output);
            }
            Err(error) => {
                save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                let details = unknown_details(&operation_id, &error);
                return Err(AuError::code(
                    "E_UNKNOWN_COMMIT",
                    format!(
                        "compiled proof outcome is unknown after {}: {}",
                        error.kind(),
                        error.compact_message()
                    ),
                )
                .with_details(details));
            }
        }
    }

    // General semantic-only plans use one authenticated helper frame. This
    // preserves the same host-side identity/generation checks and operation
    // receipt journal while removing one CLI dispatch and socket round trip
    // per step. Launch remains a host/app boundary and therefore does not use
    // this compiler.
    if let Some(compiled) = helper_plan_args(&params, deadline, started)? {
        match runtime.invoke(compiled.args) {
            Ok(result) => {
                crate::plan::validate_receipt(&result.data, &compiled.validated)?;
                if result.data.get("c").and_then(Value::as_bool) == Some(false) {
                    let failed_index = result
                        .data
                        .get("failed_index")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let code = result
                        .data
                        .get("e")
                        .and_then(Value::as_str)
                        .unwrap_or("E_UI");
                    let message = result
                        .data
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("helper plan stopped");
                    let mutations = result.data.get("m").and_then(Value::as_u64).unwrap_or(0);
                    let partial = json!({
                        "v": CONTRACT_VERSION,
                        "operation_id": operation_id,
                        "committed": false,
                        "partial": true,
                        "unknown_commit": false,
                        "failed_index": failed_index,
                        "mutations": mutations,
                        "compiled": "plan-run",
                        "result": result.data
                    });
                    save_operation(&paths, &operation_id, &binding, &partial)?;
                    return Err(AuError::code(
                        "E_PARTIAL",
                        format!(
                            "helper plan stopped at step {failed_index} after {mutations} mutations ({code}): {message}"
                        ),
                    )
                    .with_details(partial));
                }
                if let Err(error) = verify_conditions(
                    runtime,
                    &params,
                    &params.postconditions,
                    "postcondition",
                    deadline,
                    started,
                ) {
                    save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                    return Err(attach_unknown(error, &operation_id));
                }
                let mutations = params.steps.iter().filter(|step| is_mutation(step)).count();
                let output = json!({
                    "v": CONTRACT_VERSION,
                    "operation_id": operation_id,
                    "committed": true,
                    "unknown_commit": false,
                    "mutations": mutations,
                    "compiled": "plan-run",
                    "result": result.data
                });
                save_operation(&paths, &operation_id, &binding, &output)?;
                return Ok(output);
            }
            Err(error) => {
                if params.steps.iter().any(is_mutation) {
                    save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                    let details = unknown_details(&operation_id, &error);
                    return Err(AuError::code(
                        "E_UNKNOWN_COMMIT",
                        format!(
                            "compiled device plan outcome is unknown after {}: {}",
                            error.kind(),
                            error.compact_message()
                        ),
                    )
                    .with_details(details));
                }
                return Err(error);
            }
        }
    }

    let mut receipts = Vec::with_capacity(params.steps.len());
    let mut mutations = 0usize;
    for (index, step) in params.steps.iter().enumerate() {
        let remaining_ms = match remaining_ms(deadline, started) {
            Ok(value) => value,
            Err(error) => {
                if mutations > 0 {
                    save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                    return Err(attach_unknown(error, &operation_id));
                }
                return Err(error);
            }
        };
        if let Some(condition) = step.condition.as_ref() {
            if let Err(error) = verify_conditions(
                runtime,
                &params,
                std::slice::from_ref(condition),
                "step condition",
                deadline,
                started,
            ) {
                if mutations > 0 {
                    save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                    return Err(attach_unknown(error, &operation_id));
                }
                return Err(error);
            }
        }
        let mutating = is_mutation(step);
        let args = match step_args(&params, step, remaining_ms) {
            Ok(value) => value,
            Err(error) => {
                if mutations > 0 {
                    save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                    return Err(attach_unknown(error, &operation_id));
                }
                return Err(error);
            }
        };
        if mutating {
            mutations += 1;
        }
        let result = runtime.invoke(args);
        match result {
            Ok(result) => receipts.push(json!({
                "index": index,
                "op": step.op,
                "status": if mutating {"committed"} else {"proved"},
                "data": result.data
            })),
            Err(error)
                if mutating
                    && matches!(
                        error.kind(),
                        "E_ADB" | "E_DEVICE" | "E_HELPER" | "E_TIMEOUT"
                    ) =>
            {
                save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                let details = unknown_details(&operation_id, &error);
                return Err(AuError::code(
                    "E_UNKNOWN_COMMIT",
                    format!(
                        "mutation outcome is unknown after {}: {}",
                        error.kind(),
                        error.compact_message()
                    ),
                )
                .with_details(details));
            }
            Err(error) if mutations > 0 => {
                save_unknown_operation(&paths, &operation_id, &binding, &error)?;
                return Err(attach_unknown(error, &operation_id));
            }
            Err(error) => return Err(error),
        }
    }
    if let Err(error) = verify_conditions(
        runtime,
        &params,
        &params.postconditions,
        "postcondition",
        deadline,
        started,
    ) {
        if mutations > 0 {
            save_unknown_operation(&paths, &operation_id, &binding, &error)?;
            return Err(attach_unknown(error, &operation_id));
        }
        return Err(error);
    }
    let output = json!({
        "v": CONTRACT_VERSION,
        "operation_id": operation_id,
        "committed": true,
        "unknown_commit": false,
        "mutations": mutations,
        "steps": receipts
    });
    save_operation(&paths, &operation_id, &binding, &output)?;
    Ok(output)
}

fn validate_plan(params: &ExecuteParams) -> Result<()> {
    if params.steps.is_empty() || params.steps.len() > contract::MAX_CONTRACT_STEPS {
        return Err(AuError::code(
            "E_LIMIT",
            "steps must contain 1..32 operations",
        ));
    }
    if params.deadline_ms == 0 || params.deadline_ms > contract::MAX_CONTRACT_DEADLINE_MS {
        return Err(AuError::code("E_LIMIT", "deadline_ms must be 1..600000"));
    }
    if params.max_mutations > contract::MAX_CONTRACT_MUTATIONS {
        return Err(AuError::code(
            "E_LIMIT",
            "max_mutations exceeds contract limit",
        ));
    }
    if params.sensitive == "allow" {
        return Err(AuError::code(
            "E_SENSITIVE",
            "sensitive allow is not enabled in v2",
        ));
    }
    let mutations = params.steps.iter().filter(|step| is_mutation(step)).count();
    if mutations > params.max_mutations {
        return Err(AuError::code("E_LIMIT", "plan exceeds max_mutations"));
    }
    for step in &params.steps {
        if !matches!(
            step.op.as_str(),
            "find"
                | "tap"
                | "long"
                | "set"
                | "scroll"
                | "global"
                | "wait"
                | "assert"
                | "observe"
                | "launch"
        ) {
            return Err(AuError::code(
                "E_ARGS",
                format!("unsupported plan operation {}", step.op),
            ));
        }
    }
    Ok(())
}

fn is_mutation(step: &PlanStep) -> bool {
    matches!(
        step.op.as_str(),
        "tap" | "long" | "set" | "scroll" | "global" | "launch"
    )
}

fn proof_args(
    params: &ExecuteParams,
    deadline: Duration,
    started: Instant,
) -> Result<Option<Vec<String>>> {
    if params.steps.len() != 3
        || params.steps[0].op != "tap"
        || params.steps[1].op != "wait"
        || params.steps[2].op != "assert"
    {
        return Ok(None);
    }
    let tap = &params.steps[0];
    let wait = &params.steps[1];
    let assertion = &params.steps[2];
    let selector = tap
        .target
        .as_ref()
        .and_then(|target| target.selector.clone());
    let postselector = wait
        .target
        .as_ref()
        .and_then(|target| target.selector.clone());
    let asserted = assertion
        .target
        .as_ref()
        .and_then(|target| target.selector.clone());
    if selector.is_none() || postselector.is_none() || asserted != postselector {
        return Ok(None);
    }
    let remaining = remaining_ms(deadline, started)?;
    let timeout = wait
        .timeout_ms
        .unwrap_or(3_000)
        .min(assertion.timeout_ms.unwrap_or(3_000));
    let timeout = timeout.min(remaining).max(1);
    let mut args = vec![
        "--no-daemon".into(),
        "-j".into(),
        "--timeout".into(),
        remaining.to_string(),
    ];
    append_device_selector(&mut args, &params.device)?;
    args.extend([
        "ui".into(),
        "proof".into(),
        selector.expect("checked above"),
        postselector.expect("checked above"),
        timeout.to_string(),
    ]);
    Ok(Some(args))
}

fn helper_plan_args(
    params: &ExecuteParams,
    deadline: Duration,
    started: Instant,
) -> Result<Option<CompiledHelperPlan>> {
    if params.steps.iter().any(|step| {
        step.condition.is_some()
            || !matches!(
                step.op.as_str(),
                "tap" | "set" | "scroll" | "wait" | "assert" | "global"
            )
            || (step.op == "global" && step.key.as_deref() != Some("back"))
    }) {
        return Ok(None);
    }
    let steps = params
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let id = format!("s{index}");
            let mut value = match step.op.as_str() {
                "tap" => json!({
                    "id":id,
                    "op":"tap",
                    "target":step.target.as_ref().ok_or_else(|| AuError::code("E_ARGS", "tap requires target"))?.value()?
                }),
                "set" => json!({
                    "id":id,
                    "op":"text",
                    "target":step.target.as_ref().ok_or_else(|| AuError::code("E_ARGS", "set requires target"))?.value()?,
                    "text":step.text.as_ref().ok_or_else(|| AuError::code("E_ARGS", "set requires text"))?
                }),
                "scroll" => json!({
                    "id":id,
                    "op":"scroll",
                    "target":step.target.as_ref().ok_or_else(|| AuError::code("E_ARGS", "scroll requires target"))?.value()?,
                    "direction":step.direction.as_deref().unwrap_or("forward")
                }),
                "wait" | "assert" => json!({
                    "id":id,
                    "op":if step.op == "wait" { "wait.visible" } else { "assert.visible" },
                    "selector":step.target.as_ref().ok_or_else(|| AuError::code("E_ARGS", format!("{} requires target", step.op)))?.value()?,
                    "timeout_ms":step.timeout_ms.unwrap_or(3_000)
                }),
                "global" => json!({"id":id,"op":"back"}),
                _ => unreachable!("supported plan operations were checked"),
            };
            if index > 0 {
                value["depends_on"] = json!([format!("s{}", index - 1)]);
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;
    let remaining = remaining_ms(deadline, started)?;
    let validated = crate::plan::validate_payload(json!({
        "operations": steps,
        "deadline_ms": remaining.min(crate::plan::MAX_DEADLINE_MS)
    }))?;
    let payload = serde_json::to_string(&validated.payload)?;
    let mut args = vec![
        "--no-daemon".into(),
        "-j".into(),
        "--timeout".into(),
        remaining.to_string(),
    ];
    append_device_selector(&mut args, &params.device)?;
    args.extend(["ui".into(), "run".into(), payload]);
    Ok(Some(CompiledHelperPlan { args, validated }))
}

fn normalize_observation(data: &Value, mode: &str, node_limit: usize, encoding: &str) -> Value {
    if mode == "query" || mode == "expanded" || mode == "context" {
        return redact_compatibility(data);
    }
    if mode == "delta" {
        return normalize_delta_observation(data, node_limit, encoding);
    }
    if let Some(choices) = data.get("choices").and_then(Value::as_array) {
        if encoding == "dense" {
            let nodes = choices
                .iter()
                .take(node_limit)
                .map(|choice| {
                    let choice = redact_compatibility(choice);
                    let flags = u8::from(
                        choice
                            .get("clickable")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    ) | (u8::from(
                        choice
                            .get("enabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    ) << 1)
                        | (u8::from(
                            choice
                                .get("checked")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        ) << 2)
                        | (u8::from(
                            choice
                                .get("scrollable")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        ) << 3)
                        | (u8::from(
                            choice
                                .get("visible")
                                .and_then(Value::as_bool)
                                .unwrap_or(true),
                        ) << 4)
                        | (u8::from(
                            choice
                                .get("redacted")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        ) << 5);
                    json!([
                        choice.get("ref").cloned().unwrap_or(Value::Null),
                        choice.get("label").cloned().unwrap_or_else(|| json!("")),
                        choice.get("role").cloned().unwrap_or_else(|| json!("node")),
                        flags
                    ])
                })
                .collect::<Vec<_>>();
            return json!({
                "c": data.get("complete").cloned().unwrap_or(Value::Bool(true)),
                "n": nodes,
                "r": "a11y"
            });
        }
        return json!({
            "complete": data.get("complete").cloned().unwrap_or(Value::Bool(true)),
            "choices": choices.iter().take(node_limit).map(redact_compatibility).collect::<Vec<_>>(),
            "redaction": data.get("redaction").cloned().unwrap_or_else(|| json!("helper"))
        });
    }
    let nodes = data.get("n").and_then(Value::as_array);
    let mut choices = Vec::new();
    if let Some(nodes) = nodes {
        for node in nodes.iter().take(node_limit) {
            let Some(values) = node.as_array() else {
                continue;
            };
            let Some(id) = values.first().and_then(Value::as_i64) else {
                continue;
            };
            let text = values.get(1).and_then(Value::as_str).unwrap_or_default();
            let description = values.get(2).and_then(Value::as_str).unwrap_or_default();
            let role = values.get(3).and_then(Value::as_str).unwrap_or("node");
            let flags = values.get(4).and_then(Value::as_u64).unwrap_or_default();
            if flags & 1 == 0 && flags & 8 == 0 && text.is_empty() && description.is_empty() {
                continue;
            }
            choices.push(json!({
                "ref":id.to_string(),
                "label":if !text.is_empty() {text} else {description},
                "role":role.to_ascii_lowercase(),
                "enabled":flags & 2 != 0,
                "checked":flags & 4 != 0,
                "scrollable":flags & 8 != 0
            }));
        }
    }
    json!({"complete":data.get("complete").cloned().unwrap_or(Value::Bool(true)),"choices":choices})
}

/// Preserve the helper's index-addressed compact snapshot protocol verbatim.
///
/// A delta cannot be converted into the lossy decision-surface shape used by
/// `choices`: doing so drops removals and the array positions needed to apply
/// upserts. If the caller's node budget cannot hold a complete reset/delta we
/// return an explicit rebase signal instead of a corrupt partial update.
fn normalize_delta_observation(data: &Value, node_limit: usize, encoding: &str) -> Value {
    if data.get("same").and_then(Value::as_bool) == Some(true) {
        return if encoding == "dense" {
            json!({"s":1})
        } else {
            json!({"same":true})
        };
    }

    if let Some(upserts) = data.get("d").and_then(Value::as_array) {
        let deletes = data
            .get("r")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let change_count = upserts.len().saturating_add(deletes.len());
        if change_count > node_limit {
            return delta_rebase(encoding, "node_budget", change_count);
        }
        let valid_upserts = upserts.iter().all(|entry| {
            entry.as_array().is_some_and(|values| {
                values.len() == 2
                    && values.first().and_then(Value::as_u64).is_some()
                    && values.get(1).and_then(Value::as_array).is_some()
            })
        });
        let valid_deletes = deletes.iter().all(|index| index.as_u64().is_some());
        if !valid_upserts || !valid_deletes {
            return delta_rebase(encoding, "invalid_delta", change_count);
        }
        let base = data.get("base").cloned().unwrap_or(Value::Null);
        let complete = data.get("complete").cloned().unwrap_or(Value::Bool(true));
        let upserts = upserts.iter().map(redact_compatibility).collect::<Vec<_>>();
        let deletes = deletes.iter().map(redact_compatibility).collect::<Vec<_>>();
        return if encoding == "dense" {
            json!({"b":base,"c":complete,"u":upserts,"x":deletes})
        } else {
            json!({
                "base_generation": base,
                "complete": complete,
                "upserts": upserts,
                "deletes": deletes
            })
        };
    }

    if let Some(nodes) = data.get("n").and_then(Value::as_array) {
        if nodes.len() > node_limit {
            return delta_rebase(encoding, "node_budget", nodes.len());
        }
        if !nodes.iter().all(|node| node.as_array().is_some()) {
            return delta_rebase(encoding, "invalid_snapshot", nodes.len());
        }
        let nodes = nodes.iter().map(redact_compatibility).collect::<Vec<_>>();
        let complete = data.get("complete").cloned().unwrap_or(Value::Bool(true));
        return if encoding == "dense" {
            json!({"z":1,"c":complete,"n":nodes})
        } else {
            json!({"reset":true,"complete":complete,"nodes":nodes})
        };
    }

    delta_rebase(encoding, "invalid_delta", 0)
}

fn delta_rebase(encoding: &str, reason: &str, needed_nodes: usize) -> Value {
    if encoding == "dense" {
        json!({"q":1,"why":reason,"need":needed_nodes})
    } else {
        json!({
            "rebase_required": true,
            "reason": reason,
            "needed_nodes": needed_nodes
        })
    }
}

fn redact_compatibility(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(redact_compatibility).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    let sensitive = ["password", "token", "secret", "otp", "pairing_code"]
                        .iter()
                        .any(|needle| key.to_ascii_lowercase().contains(needle));
                    (
                        key.clone(),
                        if sensitive {
                            Value::String("[REDACTED]".into())
                        } else {
                            redact_compatibility(value)
                        },
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn step_args(params: &ExecuteParams, step: &PlanStep, remaining_ms: u64) -> Result<Vec<String>> {
    let mut args = vec!["--no-daemon".into(), "-j".into()];
    append_device_selector(&mut args, &params.device)?;
    match step.op.as_str() {
        "find" => {
            args.extend([
                "ui".into(),
                "find".into(),
                step.target
                    .as_ref()
                    .ok_or_else(|| AuError::code("E_ARGS", "find requires target"))?
                    .value()?,
            ]);
        }
        "tap" | "long" | "set" | "scroll" => {
            args.extend(["ui".into(), step.op.clone()]);
            args.push(
                step.target
                    .as_ref()
                    .ok_or_else(|| AuError::code("E_ARGS", format!("{} requires target", step.op)))?
                    .value()?,
            );
            if step.op == "set" {
                args.push(
                    step.text
                        .clone()
                        .ok_or_else(|| AuError::code("E_ARGS", "set requires text"))?,
                );
            }
            if step.op == "scroll" {
                args.push(step.direction.clone().unwrap_or_else(|| "down".into()));
            }
        }
        "global" => args.extend([
            "ui".into(),
            "global".into(),
            step.key
                .clone()
                .ok_or_else(|| AuError::code("E_ARGS", "global requires key"))?,
        ]),
        "wait" | "assert" => {
            args.extend([
                "ui".into(),
                step.op.clone(),
                step.target
                    .as_ref()
                    .ok_or_else(|| AuError::code("E_ARGS", format!("{} requires target", step.op)))?
                    .value()?,
            ]);
            args.push(
                step.timeout_ms
                    .unwrap_or(3_000)
                    .min(remaining_ms)
                    .max(1)
                    .to_string(),
            );
        }
        "observe" => args.extend(["ui".into(), "snap".into(), "--compact".into()]),
        "launch" => args.extend([
            "app".into(),
            "start".into(),
            step.key
                .clone()
                .ok_or_else(|| AuError::code("E_ARGS", "launch requires key package"))?,
        ]),
        _ => unreachable!(),
    }
    Ok(args)
}

fn remaining_ms(deadline: Duration, started: Instant) -> Result<u64> {
    let remaining = deadline
        .checked_sub(started.elapsed())
        .ok_or_else(|| AuError::code("E_TIMEOUT", "execute plan deadline exceeded"))?;
    Ok(remaining.as_millis().max(1).min(u64::MAX as u128) as u64)
}

fn verify_conditions(
    runtime: &mut ContractRuntime,
    params: &ExecuteParams,
    conditions: &[Value],
    phase: &str,
    deadline: Duration,
    started: Instant,
) -> Result<()> {
    for condition in conditions {
        let object = condition
            .as_object()
            .ok_or_else(|| AuError::code("E_ARGS", format!("{phase} must be an object")))?;
        let allowed = [
            "selector",
            "query",
            "present",
            "absent",
            "generation",
            "identity",
        ];
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(AuError::code(
                "E_ARGS",
                format!("unsupported {phase} field"),
            ));
        }
        if let Some(expected) = object.get("identity").and_then(Value::as_str) {
            if !device_identity_matches(&params.device, expected)? {
                return Err(AuError::code(
                    if phase == "precondition" {
                        "E_PRECONDITION"
                    } else {
                        "E_POSTCONDITION"
                    },
                    format!("{phase} identity mismatch"),
                ));
            }
        }
        if let Some(expected) = object.get("generation").and_then(Value::as_u64) {
            let observed = observe(
                runtime,
                ObserveParams {
                    device: params.device.clone(),
                    mode: "choices".into(),
                    ..ObserveParams::default()
                },
            )?;
            if observed.get("g").and_then(Value::as_u64) != Some(expected) {
                return Err(AuError::code(
                    "E_STALE",
                    format!("{phase} generation mismatch"),
                ));
            }
        }
        let selector = object
            .get("selector")
            .or_else(|| object.get("query"))
            .and_then(Value::as_str);
        if let Some(selector) = selector {
            let present = object.get("present").and_then(Value::as_bool).unwrap_or(
                !object
                    .get("absent")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            );
            let _ = remaining_ms(deadline, started)?;
            let mut args = vec!["--no-daemon".into(), "-j".into()];
            append_device_selector(&mut args, &params.device)?;
            args.extend(["ui".into(), "find".into(), selector.into()]);
            let found = match runtime.invoke(args) {
                Ok(_) => true,
                Err(error) if error.kind() == "E_UI" => false,
                Err(error) => return Err(error),
            };
            if found != present {
                return Err(AuError::code(
                    if phase == "precondition" {
                        "E_PRECONDITION"
                    } else {
                        "E_POSTCONDITION"
                    },
                    format!("{phase} selector did not satisfy: {selector}"),
                ));
            }
        } else if !object.contains_key("generation") && !object.contains_key("identity") {
            return Err(AuError::code(
                "E_ARGS",
                format!("{phase} requires selector, generation, or identity"),
            ));
        }
    }
    Ok(())
}

fn append_device_selector(args: &mut Vec<String>, device: &DeviceRef) -> Result<()> {
    if let Some(selector) = device_selector(device)? {
        args.extend(["-s".into(), selector]);
    }
    Ok(())
}

/// `serial` is the enrolled hardware identity (`ro.serialno`), while
/// `endpoint` is the currently reachable ADB selector. Keep them distinct so
/// a Wi-Fi or mDNS endpoint can be used without pretending it is the hardware
/// serial. A serial-only request is intentionally resolved through AU's
/// enrolled-device selection, which re-checks identity before every call.
fn device_selector(device: &DeviceRef) -> Result<Option<String>> {
    if device.remote_id.is_some() {
        return Err(AuError::code(
            "E_REMOTE_NOT_READY",
            "remote device references require the remote broker; local AU does not relay them",
        ));
    }
    if let Some(endpoint) = device.endpoint.as_deref() {
        validate_device_value(endpoint, "device endpoint")?;
        return Ok(Some(endpoint.to_owned()));
    }
    if let Some(serial) = device.serial.as_deref() {
        validate_device_value(serial, "device serial")?;
        let paths = AppPaths::discover()?;
        let config = crate::config::load(&paths)?;
        if config.enrolled_serial() != Some(serial) {
            return Err(AuError::code(
                "E_IDENTITY",
                "device serial does not match the enrolled hardware identity",
            ));
        }
    }
    Ok(None)
}

fn device_identity_matches(device: &DeviceRef, expected: &str) -> Result<bool> {
    validate_device_value(expected, "expected identity")?;
    if let Some(serial) = device.serial.as_deref() {
        validate_device_value(serial, "device serial")?;
        return Ok(serial == expected);
    }
    let paths = AppPaths::discover()?;
    let config = crate::config::load(&paths)?;
    Ok(config.enrolled_serial() == Some(expected))
}

fn validate_device_value(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(AuError::code("E_ARGS", format!("invalid {field}")));
    }
    Ok(())
}

fn artifact(params: &Value) -> Result<Value> {
    let artifact_id = params
        .get("artifact_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AuError::code("E_ARGS", "artifact_id is required"))?;
    if artifact_id.contains('/') || artifact_id.contains('\\') || artifact_id.contains("..") {
        return Err(AuError::code(
            "E_PATH",
            "artifact_id must be a simple AU handle",
        ));
    }
    let paths = AppPaths::discover()?;
    let path = paths.artifacts.join(artifact_id);
    if !path.is_file() {
        return Err(AuError::code(
            "E_ARTIFACT",
            "artifact does not exist in AU artifact storage",
        ));
    }
    let metadata = fs::metadata(&path)?;
    let hash = sha256_file(&path)?;
    Ok(
        json!({"v":CONTRACT_VERSION,"artifact_id":artifact_id,"bytes":metadata.len(),"sha256":hash,"path":path.to_string_lossy()}),
    )
}

fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn new_operation_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    format!(
        "op-{millis}-{}",
        OBSERVATION_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn validate_operation_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AuError::code(
            "E_ARGS",
            "operation_id is not a safe bounded id",
        ));
    }
    Ok(())
}

fn operation_path(paths: &AppPaths, operation_id: &str) -> std::path::PathBuf {
    paths
        .state
        .join("operations")
        .join(format!("{operation_id}.json"))
}

fn load_operation(paths: &AppPaths, operation_id: &str) -> Result<Option<Value>> {
    let path = operation_path(paths, operation_id);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| AuError::code("E_STATE", format!("invalid operation receipt: {error}")))
}

fn operation_binding(runtime: &mut ContractRuntime, params: &ExecuteParams) -> Result<Value> {
    use sha2::{Digest, Sha256};

    let mut canonical = params.clone();
    canonical.operation_id = None;
    let request_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&canonical)?));

    // A receipt is only replayable after a fresh endpoint/identity probe. This
    // deliberately bypasses the daemon selection cache so a recycled Wi-Fi or
    // mDNS address cannot inherit an operation outcome from another device.
    runtime.daemon.selection.invalidate();
    let mut args = vec!["--no-daemon".into(), "-j".into()];
    append_device_selector(&mut args, &params.device)?;
    args.push("st".into());
    let status = runtime.invoke(args)?.data;
    let hardware_serial = status
        .get("hardware_serial")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AuError::code("E_IDENTITY", "live target identity is unavailable"))?;
    let endpoint = status
        .get("endpoint")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AuError::code("E_DEVICE", "live target endpoint is unavailable"))?;
    let target_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!({
            "hardware_serial": hardware_serial,
            "endpoint": endpoint
        }))?)
    );
    Ok(json!({
        "v": 1,
        "request_sha256": request_sha256,
        "target_sha256": target_sha256
    }))
}

fn validate_operation_record(record: &Value, binding: &Value) -> Result<Value> {
    if record.get("binding") != Some(binding) {
        return Err(AuError::code(
            "E_REPLAY_MISMATCH",
            "operation_id was already used for a different request or live device endpoint",
        ));
    }
    record
        .get("receipt")
        .cloned()
        .ok_or_else(|| AuError::code("E_STATE", "operation record is missing its receipt"))
}

fn save_operation(
    paths: &AppPaths,
    operation_id: &str,
    binding: &Value,
    value: &Value,
) -> Result<()> {
    atomic_write(
        &operation_path(paths, operation_id),
        &serde_json::to_vec(&json!({"binding":binding,"receipt":value}))?,
    )
}

fn save_unknown_operation(
    paths: &AppPaths,
    operation_id: &str,
    binding: &Value,
    error: &AuError,
) -> Result<()> {
    save_operation(
        paths,
        operation_id,
        binding,
        &json!({
            "v": CONTRACT_VERSION,
            "operation_id": operation_id,
            "unknown_commit": true,
            "error": {"code":error.kind(),"message":error.compact_message()}
        }),
    )
}

fn unknown_details(operation_id: &str, error: &AuError) -> Value {
    json!({
        "operation_id": operation_id,
        "unknown_commit": true,
        "error": {
            "code": error.kind(),
            "message": error.compact_message()
        },
        "next": "observe before issuing a new operation"
    })
}

fn attach_unknown(error: AuError, operation_id: &str) -> AuError {
    let details = unknown_details(operation_id, &error);
    error.with_details(details)
}

pub fn action(cli: &Cli) -> Result<ActionResult> {
    match cli.command.as_str() {
        "observe" => {
            let mode = cli
                .args
                .first()
                .cloned()
                .unwrap_or_else(|| "choices".into());
            Ok(ActionResult {
                brief: Brief::Ok,
                data: observe(
                    &mut ContractRuntime::new()?,
                    ObserveParams {
                        mode,
                        ..ObserveParams::default()
                    },
                )?,
            })
        }
        "execute" => {
            let input = cli
                .args
                .first()
                .ok_or_else(|| AuError::code("E_ARGS", "execute requires JSON plan or path"))?;
            let text = if PathBuf::from(input).is_file() {
                fs::read_to_string(input)?
            } else {
                input.clone()
            };
            let params: ExecuteParams = serde_json::from_str(&text).map_err(|error| {
                AuError::code("E_ARGS", format!("invalid execute plan: {error}"))
            })?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: execute_plan(params)?,
            })
        }
        "artifact" => {
            let artifact_id = cli
                .args
                .first()
                .ok_or_else(|| AuError::code("E_ARGS", "artifact requires an artifact id"))?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: artifact(&json!({"artifact_id":artifact_id}))?,
            })
        }
        _ => Err(AuError::code("E_ARGS", "unsupported contract CLI command")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Target;

    #[test]
    fn operation_ids_cannot_escape_receipt_directory() {
        assert!(validate_operation_id("op-1_ok").is_ok());
        assert!(validate_operation_id("..\\receipt").is_err());
        assert!(validate_operation_id("").is_err());
    }

    #[test]
    fn replay_records_are_bound_to_the_original_request_and_live_target() {
        let binding = json!({
            "v":1,
            "request_sha256":"request-a",
            "target_sha256":"target-a"
        });
        let record = json!({"binding":binding,"receipt":{"committed":true}});
        assert_eq!(
            validate_operation_record(&record, &binding).expect("matching binding")["committed"],
            true
        );
        let mismatch = json!({
            "v":1,
            "request_sha256":"request-b",
            "target_sha256":"target-a"
        });
        assert_eq!(
            validate_operation_record(&record, &mismatch)
                .expect_err("mismatched request must not replay")
                .kind(),
            "E_REPLAY_MISMATCH"
        );
    }

    #[test]
    fn contract_choices_are_bounded_and_redacted() {
        let data = json!({
            "v": 2,
            "complete": true,
            "choices": [
                {"ref":"s1","label":"Visible","token":"secret"},
                {"ref":"s2","label":"Hidden"}
            ]
        });
        let output = normalize_observation(&data, "choices", 1, "object");
        assert_eq!(output["choices"].as_array().expect("choices").len(), 1);
        assert_eq!(output["choices"][0]["token"], "[REDACTED]");
    }

    #[test]
    fn dense_contract_choices_use_short_redacted_tuples() {
        let data = json!({
            "v": 2,
            "complete": true,
            "choices": [{
                "ref":"s1","label":"Visible","role":"button","clickable":true,
                "enabled":true,"visible":true,"token":"secret"
            }]
        });
        let output = normalize_observation(&data, "choices", 8, "dense");
        assert_eq!(output["n"][0], json!(["s1", "Visible", "button", 19]));
        assert_eq!(output["r"], "a11y");
    }

    #[test]
    fn dense_delta_preserves_upserts_deletes_and_base_generation() {
        let data = json!({
            "v": 1,
            "base": 41,
            "g": 42,
            "complete": true,
            "d": [[2, [7, "Save", "", "button", 3, [0, 0, 10, 10]]]],
            "r": [5]
        });
        let output = normalize_observation(&data, "delta", 8, "dense");
        assert_eq!(output["b"], 41);
        assert_eq!(output["u"], data["d"]);
        assert_eq!(output["x"], data["r"]);
        assert_eq!(output["c"], true);
    }

    #[test]
    fn unchanged_delta_is_a_minimal_token_dense_receipt() {
        let output = normalize_observation(&json!({"v":1,"g":42,"same":true}), "delta", 8, "dense");
        assert_eq!(output, json!({"s":1}));
    }

    #[test]
    fn first_delta_snapshot_is_an_explicit_lossless_reset() {
        let nodes = json!([[7, "Save", "", "button", 3, [0, 0, 10, 10]]]);
        let output = normalize_observation(
            &json!({"v":1,"g":42,"complete":true,"n":nodes}),
            "delta",
            8,
            "object",
        );
        assert_eq!(output["reset"], true);
        assert_eq!(output["nodes"], nodes);
    }

    #[test]
    fn oversized_delta_requests_rebase_instead_of_returning_partial_state() {
        let output = normalize_observation(
            &json!({"v":1,"base":41,"g":42,"d":[[0, []], [1, []]],"r":[2]}),
            "delta",
            2,
            "object",
        );
        assert_eq!(output["rebase_required"], true);
        assert_eq!(output["reason"], "node_budget");
        assert_eq!(output["needed_nodes"], 3);
    }

    #[test]
    fn observation_budget_returns_typed_recovery_details() {
        let error = ensure_observation_budget(101, 100, "expanded")
            .expect_err("oversized observation must fail");
        assert_eq!(error.kind(), "E_OUTPUT_LIMIT");
        assert_eq!(error.details().expect("details")["bytes"], 101);
        assert_eq!(error.details().expect("details")["budget"], 100);
        assert!(ensure_observation_budget(100, 100, "expanded").is_ok());
    }

    #[test]
    fn delta_base_validation_accepts_exact_updates_and_forces_mismatch_rebases() {
        assert!(delta_matches_base(
            &json!({"base":41,"g":42,"d":[],"r":[]}),
            41
        ));
        assert!(delta_matches_base(&json!({"g":41,"same":true}), 41));
        assert!(delta_matches_base(&json!({"g":42,"n":[]}), 41));
        assert!(!delta_matches_base(
            &json!({"base":40,"g":42,"d":[],"r":[]}),
            41
        ));
        assert!(!delta_matches_base(&json!({"g":42,"same":true}), 41));
    }

    #[test]
    fn common_semantic_flow_compiles_to_one_proof_request() {
        let params = ExecuteParams {
            steps: vec![
                PlanStep {
                    op: "tap".into(),
                    target: Some(Target {
                        selector: Some("text=Allow".into()),
                        ..Target::default()
                    }),
                    ..PlanStep::default()
                },
                PlanStep {
                    op: "wait".into(),
                    target: Some(Target {
                        selector: Some("text=Done".into()),
                        ..Target::default()
                    }),
                    ..PlanStep::default()
                },
                PlanStep {
                    op: "assert".into(),
                    target: Some(Target {
                        selector: Some("text=Done".into()),
                        ..Target::default()
                    }),
                    ..PlanStep::default()
                },
            ],
            ..ExecuteParams::default()
        };
        let args = proof_args(&params, Duration::from_secs(1), Instant::now())
            .expect("proof args")
            .expect("compiled proof");
        assert_eq!(args[2], "--timeout");
        assert!(args[3].parse::<u64>().expect("timeout") <= 1_000);
        assert_eq!(&args[4..6], &["ui", "proof"]);
    }

    #[test]
    fn general_semantic_plan_compiles_to_one_device_plan() {
        let params = ExecuteParams {
            max_mutations: 2,
            steps: vec![
                PlanStep {
                    op: "set".into(),
                    target: Some(Target {
                        selector: Some("desc=Editor".into()),
                        ..Target::default()
                    }),
                    text: Some("dense text".into()),
                    ..PlanStep::default()
                },
                PlanStep {
                    op: "assert".into(),
                    target: Some(Target {
                        selector: Some("text=dense text".into()),
                        ..Target::default()
                    }),
                    timeout_ms: Some(500),
                    ..PlanStep::default()
                },
            ],
            ..ExecuteParams::default()
        };
        let compiled = helper_plan_args(&params, Duration::from_secs(1), Instant::now())
            .expect("run args")
            .expect("compiled run");
        let args = compiled.args;
        assert_eq!(&args[4..6], &["ui", "run"]);
        let payload: Value = serde_json::from_str(&args[6]).expect("payload");
        assert_eq!(payload["operations"].as_array().expect("steps").len(), 2);
        assert_eq!(payload["operations"][0]["op"], "text");
        assert_eq!(payload["operations"][0]["text"], "dense text");
        assert_eq!(payload["operations"][1]["depends_on"], json!(["s0"]));
    }

    #[test]
    fn launch_keeps_the_host_boundary() {
        let params = ExecuteParams {
            steps: vec![PlanStep {
                op: "launch".into(),
                key: Some("dev.codex.fixture".into()),
                ..PlanStep::default()
            }],
            ..ExecuteParams::default()
        };
        assert!(
            helper_plan_args(&params, Duration::from_secs(1), Instant::now())
                .expect("run args")
                .is_none()
        );
    }
}
