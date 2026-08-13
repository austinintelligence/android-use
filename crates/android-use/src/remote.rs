//! Strict remote-mode boundary.
//!
//! The local helper remains the high-authority component. This module only
//! defines the versioned, allowlisted envelope that a future outbound broker
//! may carry; it deliberately does not open sockets, expose ADB, or enable a
//! relay as a side effect of installing android-use.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::actions::{ActionResult, Brief};
use crate::config::{atomic_write, AppPaths};
use crate::contract::ExecuteParams;
use crate::error::{AuError, Result};
use crate::{CONTRACT_VERSION, PROTOCOL_VERSION};

pub const REMOTE_PROTOCOL_VERSION: u16 = 1;
pub const MAX_REMOTE_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_REMOTE_STEPS: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairingEnvelope {
    pub v: u16,
    pub host_id: String,
    pub host_public_key: String,
    pub pairing_secret: String,
    pub expires_at_ms: u64,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteOperation {
    pub v: u16,
    pub operation_id: String,
    pub deadline_ms: u64,
    pub expected_generation: Option<u64>,
    pub plan: ExecuteParams,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteFrame {
    pub v: u16,
    pub session_id: String,
    pub sequence: u64,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct RemoteState {
    schema: u32,
    enabled: bool,
    paired_hosts: Vec<String>,
    revoked_hosts: Vec<String>,
    pending_pairing: Option<PendingPairing>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PendingPairing {
    code: String,
    expires_at_ms: u64,
}

pub fn action(args: &[String]) -> Result<ActionResult> {
    let paths = AppPaths::discover()?;
    let operation = args.first().map(String::as_str).unwrap_or("status");
    let mut state = load(&paths)?;
    let result = match operation {
        "status" => json!({
            "v": REMOTE_PROTOCOL_VERSION,
            "enabled": state.enabled,
            "available": false,
            "paired_hosts": state.paired_hosts,
            "revoked_hosts": state.revoked_hosts,
            "transport": "outbound-encrypted-broker-not-configured",
            "local_high_authority_bridge": "separate-no-internet-helper"
        }),
        "protocol" => protocol_schema(),
        "pair" => {
            return Err(AuError::code(
                "E_REMOTE_NOT_READY",
                "remote broker is not configured; local observe/execute remains available",
            ));
        }
        "enable" => {
            if state.paired_hosts.is_empty() {
                return Err(AuError::code(
                    "E_REMOTE_PAIRING",
                    "pair a host before enabling remote access",
                ));
            }
            state.enabled = true;
            save(&paths, &state)?;
            json!({"enabled":true,"paired_hosts":state.paired_hosts})
        }
        "disable" => {
            state.enabled = false;
            save(&paths, &state)?;
            json!({"enabled":false})
        }
        "revoke" => {
            let host = args
                .get(1)
                .ok_or_else(|| AuError::code("E_ARGS", "remote revoke HOST_ID"))?;
            validate_id(host, "host_id")?;
            state.paired_hosts.retain(|value| value != host);
            if !state.revoked_hosts.iter().any(|value| value == host) {
                state.revoked_hosts.push(host.clone());
            }
            if state.paired_hosts.is_empty() {
                state.enabled = false;
            }
            save(&paths, &state)?;
            json!({"revoked":host,"enabled":state.enabled})
        }
        _ => {
            return Err(AuError::code(
                "E_ARGS",
                "remote expects status, protocol, pair, enable, disable, or revoke",
            ))
        }
    };
    Ok(ActionResult {
        brief: Brief::Ok,
        data: result,
    })
}

pub fn validate_operation(operation: &RemoteOperation) -> Result<()> {
    if operation.v != REMOTE_PROTOCOL_VERSION {
        return Err(AuError::code(
            "E_PROTOCOL",
            "unsupported remote operation version",
        ));
    }
    if operation.operation_id.is_empty() || operation.operation_id.len() > 128 {
        return Err(AuError::code(
            "E_LIMIT",
            "remote operation id must be 1..128 bytes",
        ));
    }
    if operation.deadline_ms == 0 || operation.deadline_ms > 600_000 {
        return Err(AuError::code(
            "E_LIMIT",
            "remote deadline must be 1..600000 ms",
        ));
    }
    if operation.plan.steps.len() > MAX_REMOTE_STEPS {
        return Err(AuError::code("E_LIMIT", "remote plan has too many steps"));
    }
    for step in &operation.plan.steps {
        if matches!(
            step.op.as_str(),
            "raw" | "shell" | "adb" | "file" | "camera" | "microphone"
        ) {
            return Err(AuError::code(
                "E_REMOTE_POLICY",
                "remote plans cannot contain unrestricted or hidden capability operations",
            ));
        }
    }
    Ok(())
}

pub fn protocol_schema() -> Value {
    json!({
        "v": REMOTE_PROTOCOL_VERSION,
        "contract": CONTRACT_VERSION,
        "local_protocol": PROTOCOL_VERSION,
        "frame": {"encrypted": true, "sequence": "strictly_increasing", "nonce": "unique", "max_bytes": MAX_REMOTE_FRAME_BYTES},
        "operations": ["observe", "execute", "artifact_metadata", "pair", "revoke"],
        "forbidden": ["public_adb", "raw_shell", "raw_adb", "arbitrary_filesystem", "unbounded_media", "blind_replay"],
        "execution": "send_complete_bounded_plan_to_phone; return proof_or_minimal_delta"
    })
}

fn validate_id(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AuError::code("E_ARGS", format!("invalid {field}")));
    }
    Ok(())
}

fn path(paths: &AppPaths) -> std::path::PathBuf {
    paths.state.join("remote.json")
}

fn load(paths: &AppPaths) -> Result<RemoteState> {
    let path = path(paths);
    if !path.exists() {
        return Ok(RemoteState {
            schema: 1,
            ..RemoteState::default()
        });
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|error| AuError::code("E_REMOTE", format!("invalid remote state: {error}")))
}

fn save(paths: &AppPaths, state: &RemoteState) -> Result<()> {
    atomic_write(&path(paths), &serde_json::to_vec(state)?)
}

#[allow(dead_code)]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_schema_forbids_public_adb() {
        assert!(protocol_schema()["forbidden"]
            .as_array()
            .expect("forbidden")
            .iter()
            .any(|value| value == "public_adb"));
    }

    #[test]
    fn remote_ids_are_bounded_and_safe() {
        validate_id("desktop-1", "host_id").expect("valid id");
        assert!(validate_id("../host", "host_id").is_err());
    }
}
