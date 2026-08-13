use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AuError, Result};
use crate::CONTRACT_VERSION;

pub const MAX_CONTRACT_STEPS: usize = 32;
pub const MAX_CONTRACT_MUTATIONS: usize = 16;
pub const MAX_CONTRACT_DEADLINE_MS: u64 = 600_000;
pub const MAX_CONTRACT_MESSAGE_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub v: u16,
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Response {
    pub v: u16,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatusParams {
    #[serde(default)]
    pub device: DeviceRef,
    #[serde(default)]
    pub fresh: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chars: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveParams {
    #[serde(default)]
    pub device: DeviceRef,
    #[serde(default = "default_observe_mode")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_observation: Option<String>,
    #[serde(default)]
    pub budget: Budget,
    /// `object` preserves the descriptive v2 shape. `dense` returns the same
    /// redacted frontier as compact tuples for model loops that value tokens
    /// and wire time over self-description. Dense is the agent-safe default;
    /// callers can opt into object encoding when they need field names.
    #[serde(default = "default_observe_encoding")]
    pub encoding: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
}

impl Target {
    pub fn value(&self) -> Result<String> {
        self.ref_id
            .clone()
            .or_else(|| self.selector.clone())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AuError::code("E_ARGS", "target requires ref_id or selector"))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteParams {
    #[serde(default)]
    pub device: DeviceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_generation: Option<u64>,
    #[serde(default = "default_deadline_ms")]
    pub deadline_ms: u64,
    #[serde(default = "default_mutation_limit")]
    pub max_mutations: usize,
    #[serde(default)]
    pub sensitive: String,
    #[serde(default)]
    pub preconditions: Vec<Value>,
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub postconditions: Vec<Value>,
}

pub fn default_observe_mode() -> String {
    "choices".into()
}

pub fn default_observe_encoding() -> String {
    "dense".into()
}

pub fn default_deadline_ms() -> u64 {
    8_000
}

pub fn default_mutation_limit() -> usize {
    8
}

pub fn parse_request(line: &str) -> Result<Request> {
    if line.len() > MAX_CONTRACT_MESSAGE_BYTES {
        return Err(AuError::code(
            "E_LIMIT",
            format!(
                "contract message exceeds {} bytes",
                MAX_CONTRACT_MESSAGE_BYTES
            ),
        ));
    }
    let request: Request = serde_json::from_str(line).map_err(|error| {
        AuError::code("E_PROTOCOL", format!("invalid contract request: {error}"))
    })?;
    validate_request(&request)?;
    Ok(request)
}

pub fn validate_request(request: &Request) -> Result<()> {
    if request.v != CONTRACT_VERSION {
        return Err(AuError::code(
            "E_PROTOCOL",
            format!(
                "unsupported contract version {}; expected {CONTRACT_VERSION}",
                request.v
            ),
        ));
    }
    if request.id.is_empty() || request.id.len() > 128 {
        return Err(AuError::code("E_LIMIT", "request id must be 1..128 bytes"));
    }
    if request.method.len() > 96
        || !matches!(
            request.method.as_str(),
            "android.status"
                | "android.observe"
                | "android.execute"
                | "android.artifact"
                | "android.recipe"
        )
    {
        return Err(AuError::code(
            "E_PROTOCOL",
            "method is not part of the android-use contract",
        ));
    }
    Ok(())
}

pub fn ok(id: impl Into<String>, result: Value) -> Response {
    Response {
        v: CONTRACT_VERSION,
        id: id.into(),
        ok: true,
        result: Some(result),
        error: None,
    }
}

pub fn error(id: impl Into<String>, error: &AuError) -> Response {
    Response {
        v: CONTRACT_VERSION,
        id: id.into(),
        ok: false,
        result: None,
        error: Some(ErrorBody {
            code: error.kind().into(),
            message: error.compact_message(),
            details: error.details().cloned(),
            retryable: Some(matches!(
                error.kind(),
                "E_TIMEOUT" | "E_CANCELLED" | "E_STALE" | "E_PARTIAL"
            )),
        }),
    }
}

pub fn schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://android-use.dev/schema/agent-contract-v2.json",
        "title": "Android Use Agent Contract",
        "type": "object",
        "required": ["v", "id", "method", "params"],
        "properties": {
            "v": {"const": CONTRACT_VERSION},
            "id": {"type": "string", "minLength": 1, "maxLength": 128},
            "method": {"enum": ["android.status", "android.observe", "android.execute", "android.artifact", "android.recipe"]},
            "params": {"type": "object"}
        },
        "limits": {
            "max_steps": MAX_CONTRACT_STEPS,
            "max_mutations": MAX_CONTRACT_MUTATIONS,
            "max_deadline_ms": MAX_CONTRACT_DEADLINE_MS,
            "max_message_bytes": MAX_CONTRACT_MESSAGE_BYTES
        }
    })
}

pub fn mcp_tools() -> Value {
    json!({"tools":[
        {"name":"android.status","title":"Android status","description":"Return exact device, transport, helper, capability, and readiness state.","inputSchema":{"type":"object","properties":{"device":{"type":"object","properties":{"serial":{"type":"string"},"endpoint":{"type":"string"},"remote_id":{"type":"string"}}},"fresh":{"type":"boolean","default":false}}},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}},
        {"name":"android.observe","title":"Observe Android UI","description":"Return bounded, redacted semantic UI evidence; dense encoding is the default.","inputSchema":{"type":"object","properties":{"device":{"type":"object","properties":{"serial":{"type":"string"},"endpoint":{"type":"string"},"remote_id":{"type":"string"}}},"mode":{"enum":["choices","frontier","delta","expanded","context","query"]},"query":{"type":"string"},"base_observation":{"type":"string"},"budget":{"type":"object","properties":{"bytes":{"type":"integer","minimum":1},"nodes":{"type":"integer","minimum":1},"chars":{"type":"integer","minimum":1}}},"encoding":{"enum":["object","dense"],"default":"dense"}}},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}},
        {"name":"android.execute","title":"Execute Android plan","description":"Execute a bounded semantic plan with identity, generation, postconditions, and typed recovery receipts.","inputSchema":{"type":"object","required":["steps"],"properties":{"device":{"type":"object","properties":{"serial":{"type":"string"},"endpoint":{"type":"string"},"remote_id":{"type":"string"}}},"operation_id":{"type":"string","maxLength":128},"expected_identity":{"type":"string"},"expected_generation":{"type":"integer","minimum":0},"deadline_ms":{"type":"integer","minimum":1,"maximum":MAX_CONTRACT_DEADLINE_MS,"default":8000},"max_mutations":{"type":"integer","minimum":0,"maximum":MAX_CONTRACT_MUTATIONS,"default":8},"sensitive":{"type":"string"},"preconditions":{"type":"array","maxItems":MAX_CONTRACT_STEPS},"steps":{"type":"array","minItems":1,"maxItems":MAX_CONTRACT_STEPS},"postconditions":{"type":"array","maxItems":MAX_CONTRACT_STEPS}}},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false}},
        {"name":"android.artifact","title":"Read Android artifact","description":"Read a bounded AU-owned artifact by handle.","inputSchema":{"type":"object","required":["artifact_id"]},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}},
        {"name":"android.recipe","title":"Run Android recipe","description":"Run a validated declarative semantic recipe.","inputSchema":{"type":"object","required":["name"]},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false}}
    ]})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_v2_requests() {
        let request = parse_request(r#"{"v":2,"id":"1","method":"android.status","params":{}}"#)
            .expect("request");
        assert_eq!(request.method, "android.status");
    }

    #[test]
    fn rejects_non_contract_methods() {
        let error = parse_request(r#"{"v":2,"id":"1","method":"shell","params":{}}"#)
            .expect_err("unsafe method");
        assert_eq!(error.kind(), "E_PROTOCOL");
    }

    #[test]
    fn rejects_unknown_android_methods() {
        let error = parse_request(r#"{"v":2,"id":"1","method":"android.shell","params":{}}"#)
            .expect_err("unknown method");
        assert_eq!(error.kind(), "E_PROTOCOL");
    }

    #[test]
    fn observe_defaults_to_dense_for_agent_loops() {
        let params: ObserveParams = serde_json::from_str("{}").expect("defaults");
        assert_eq!(params.mode, "choices");
        assert_eq!(params.encoding, "dense");
    }

    #[test]
    fn status_params_are_typed_and_reject_unknown_fields() {
        let params: StatusParams = serde_json::from_str(r#"{"fresh":true}"#).expect("status");
        assert!(params.fresh);
        let error = serde_json::from_str::<StatusParams>(r#"{"stale":true}"#)
            .expect_err("unknown status field");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn contract_errors_preserve_recovery_details() {
        let au_error = AuError::code("E_PARTIAL", "step failed")
            .with_details(json!({"failed_index":2,"next":"observe"}));
        let response = super::error("op-1", &au_error);
        let body = response.error.expect("error body");
        assert_eq!(body.code, "E_PARTIAL");
        assert_eq!(body.retryable, Some(true));
        assert_eq!(body.details.expect("details")["failed_index"], 2);
    }
}
