use std::fs;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::actions::{ActionResult, Brief};
use crate::adb::Adb;
use crate::cli::Cli;
use crate::config::{atomic_write, save, AppPaths, Config};
use crate::device::{DeviceInventory, Endpoint};
use crate::error::{AuError, Result};
use crate::process::text;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    HostInstalled,
    PlatformToolsReady,
    DeviceDetected,
    DeviceAuthorized,
    DeviceEnrolled,
    BridgeInstalled,
    SemanticAccessEnabled,
    AgentConfigured,
    Ready,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Journal {
    pub schema: u32,
    pub run_id: String,
    pub state: Option<State>,
    pub target_identity: Option<String>,
    pub endpoint: Option<String>,
    pub last_error: Option<Value>,
    pub verification: Value,
    pub updated_at_ms: u128,
}

pub fn action(cli: &Cli, paths: &AppPaths, config: &mut Config, adb: &Adb) -> Result<ActionResult> {
    let setup = cli.command == "setup";
    let mut journal = load(paths)?;
    journal.run_id = if journal.run_id.is_empty() {
        format!("setup-{}", now_ms())
    } else {
        journal.run_id.clone()
    };
    journal.last_error = None;

    let result = if cli.command == "setup" && cli.args.iter().any(|arg| arg == "--wait") {
        evaluate_until_ready(cli, paths, config, adb, &mut journal)
    } else {
        evaluate(cli, paths, config, adb, &mut journal)
    };
    journal.updated_at_ms = now_ms();
    match result {
        Ok(data) => {
            save_journal(paths, &journal)?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: json!({"setup":setup,"ready":matches!(journal.state, Some(State::Ready)),"state":journal.state,"data":data,"journal":journal}),
            })
        }
        Err(error) => {
            journal.last_error =
                Some(json!({"code":error.kind(),"message":error.compact_message()}));
            save_journal(paths, &journal)?;
            if cli.command == "ready" {
                Ok(ActionResult {
                    brief: Brief::Ok,
                    data: json!({"ready":false,"state":journal.state,"error":journal.last_error,"journal":journal}),
                })
            } else {
                Err(error)
            }
        }
    }
}

pub fn without_adb(cli: &Cli, paths: &AppPaths, error: &AuError) -> Result<ActionResult> {
    let mut journal = load(paths)?;
    journal.schema = 2;
    if journal.run_id.is_empty() {
        journal.run_id = format!("setup-{}", now_ms());
    }
    journal.state = Some(State::HostInstalled);
    journal.last_error = Some(json!({
        "code": error.kind(),
        "message": error.compact_message(),
        "next": "PLATFORM_TOOLS_READY"
    }));
    journal.updated_at_ms = now_ms();
    save_journal(paths, &journal)?;
    if cli.command == "ready" {
        return Ok(ActionResult {
            brief: Brief::Ok,
            data: json!({
                "ready": false,
                "state": journal.state,
                "error": journal.last_error,
                "journal": journal
            }),
        });
    }
    Err(AuError::code(
        "E_ADB",
        "Android platform-tools are not installed; install the verified platform-tools asset and resume setup",
    ))
}

fn evaluate_until_ready(
    cli: &Cli,
    paths: &AppPaths,
    config: &mut Config,
    adb: &Adb,
    journal: &mut Journal,
) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match evaluate(cli, paths, config, adb, journal) {
            Ok(_value)
                if !matches!(journal.state, Some(State::Ready))
                    && matches!(journal.state, Some(State::BridgeInstalled))
                    && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_secs(1));
            }
            Ok(value) => return Ok(value),
            Err(error)
                if matches!(error.kind(), "E_DEVICE" | "E_WAITING_FOR_AUTH")
                    && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_secs(1));
            }
            Err(error) => return Err(error),
        }
    }
}

fn evaluate(
    cli: &Cli,
    paths: &AppPaths,
    config: &mut Config,
    adb: &Adb,
    journal: &mut Journal,
) -> Result<Value> {
    journal.state = Some(State::HostInstalled);
    if !adb.path().is_file() {
        return Err(AuError::code(
            "E_ADB",
            "managed adb executable is not present",
        ));
    }

    let version = adb
        .global(&["version".into()])
        .map(|result| text(&result.stdout))
        .unwrap_or_default();
    journal.verification = json!({"adb_path":adb.path().to_string_lossy(),"adb_version":version});
    journal.state = Some(State::PlatformToolsReady);

    let inventory = DeviceInventory::discover(adb)?;
    if inventory.endpoints.is_empty() {
        return Err(AuError::code("E_DEVICE", "no Android endpoint detected"));
    }
    journal.state = Some(State::DeviceDetected);

    let endpoint = select_endpoint(cli, config, &inventory)?;
    if endpoint.state != "device" {
        return Err(AuError::code(
            "E_WAITING_FOR_AUTH",
            format!(
                "endpoint {} is {}; accept the Android RSA prompt",
                endpoint.endpoint, endpoint.state
            ),
        ));
    }
    journal.state = Some(State::DeviceAuthorized);

    let identity = endpoint
        .hardware_serial
        .clone()
        .or_else(|| {
            adb.device(
                &endpoint.endpoint,
                &["shell".into(), "getprop".into(), "ro.serialno".into()],
            )
            .ok()
            .map(|result| text(&result.stdout))
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AuError::code(
                "E_IDENTITY",
                "authorized endpoint did not report ro.serialno",
            )
        })?;
    let mut config_changed = false;
    if let Some(enrolled) = config.enrolled_serial() {
        if enrolled != identity {
            return Err(AuError::code(
                "E_IDENTITY",
                "detected endpoint does not match enrolled hardware identity",
            ));
        }
    } else {
        config.hardware_serial = identity.clone();
        config_changed = true;
    }
    if config.selected_endpoint.as_deref() != Some(endpoint.endpoint.as_str()) {
        config.selected_endpoint = Some(endpoint.endpoint.clone());
        config_changed = true;
    }
    if config_changed {
        save(paths, config)?;
    }
    journal.target_identity = Some(identity.clone());
    journal.endpoint = Some(endpoint.endpoint.clone());
    journal.state = Some(State::DeviceEnrolled);

    // The ADB endpoint is the transport selector. The hardware identity is
    // recorded separately and must never be used as a substitute for a
    // Wi-Fi/mDNS endpoint.
    let helper = crate::helper::capability(adb, &endpoint.endpoint);
    let mut semantic_enabled = false;
    let helper_data = match helper {
        Ok(data) => {
            journal.state = Some(State::BridgeInstalled);
            semantic_enabled = data
                .get("accessibility")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || data
                    .get("semantic")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            if semantic_enabled {
                journal.state = Some(State::SemanticAccessEnabled);
            }
            data
        }
        Err(error) => {
            json!({"ready":false,"code":error.kind(),"message":error.compact_message(),"next":"BRIDGE_INSTALLED"})
        }
    };

    let mut agent_configured = paths.state.join("agent.json").is_file();
    if cli.command == "setup" {
        let requested = cli
            .args
            .windows(2)
            .find(|pair| pair[0] == "--agent")
            .map(|pair| pair[1].as_str())
            .unwrap_or("auto");
        crate::agent::action(&["configure".into(), requested.into()])?;
        agent_configured = true;
    }
    if agent_configured && semantic_enabled {
        journal.state = Some(State::AgentConfigured);
    }
    if matches!(journal.state, Some(State::AgentConfigured)) && agent_configured {
        journal.state = Some(State::Ready);
    }
    if cli.repair {
        journal.verification["repair"] = json!("only AU-owned host state was repaired");
    }
    Ok(json!({
        "identity":identity,
        "endpoint":endpoint,
        "helper":helper_data,
        "agent_configured":agent_configured,
        "next":next_state(journal.state.as_ref())
    }))
}

fn select_endpoint(cli: &Cli, config: &Config, inventory: &DeviceInventory) -> Result<Endpoint> {
    if let Some(requested) = cli.serial.as_deref() {
        return inventory.resolve(config, Some(requested));
    }
    if let Some(endpoint) = config.selected_endpoint.as_deref() {
        if let Ok(found) = inventory.resolve(config, Some(endpoint)) {
            return Ok(found);
        }
    }
    let authorized = inventory
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.state == "device")
        .collect::<Vec<_>>();
    match authorized.as_slice() {
        [endpoint] => Ok((*endpoint).clone()),
        [] => inventory
            .endpoints
            .first()
            .cloned()
            .ok_or_else(|| AuError::code("E_DEVICE", "no endpoint detected")),
        _ => Err(AuError::code(
            "E_DEVICE",
            "multiple authorized devices; pass --serial ENDPOINT",
        )),
    }
}

fn next_state(state: Option<&State>) -> &'static str {
    match state {
        None => "HOST_INSTALLED",
        Some(State::HostInstalled) => "PLATFORM_TOOLS_READY",
        Some(State::PlatformToolsReady) => "DEVICE_DETECTED",
        Some(State::DeviceDetected) => "DEVICE_AUTHORIZED",
        Some(State::DeviceAuthorized) => "DEVICE_ENROLLED",
        Some(State::DeviceEnrolled) => "BRIDGE_INSTALLED",
        Some(State::BridgeInstalled) => "SEMANTIC_ACCESS_ENABLED",
        Some(State::SemanticAccessEnabled) => "AGENT_CONFIGURED",
        Some(State::AgentConfigured) => "READY",
        Some(State::Ready) => "READY",
    }
}

fn journal_path(paths: &AppPaths) -> std::path::PathBuf {
    paths.state.join("setup.json")
}

fn load(paths: &AppPaths) -> Result<Journal> {
    let path = journal_path(paths);
    if !path.exists() {
        return Ok(Journal {
            schema: 2,
            ..Journal::default()
        });
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|error| AuError::code("E_SETUP", format!("invalid setup journal: {error}")))
}

fn save_journal(paths: &AppPaths, journal: &Journal) -> Result<()> {
    atomic_write(&journal_path(paths), &serde_json::to_vec(journal)?)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}
