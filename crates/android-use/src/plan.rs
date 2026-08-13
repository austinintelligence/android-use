//! Host-side gate for the device-resident plan.run executor.
//!
//! The device performs the execution and owns the mutation receipts. This
//! module mirrors the small, forward-only plan grammar at the host boundary
//! so malformed or over-budget plans cannot reach the device. It deliberately
//! has no install, package-manager, shell, filesystem, or retry operation.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::error::{AuError, Result};

pub const MAX_STEPS: usize = 128;
pub const MAX_DEADLINE_MS: u64 = 30_000;
pub const DEFAULT_DEADLINE_MS: u64 = MAX_DEADLINE_MS;
pub const MAX_SELECTOR_BYTES: usize = 1_024;
pub const MAX_TEXT_BYTES: usize = 8_192;
pub const MAX_ID_BYTES: usize = 64;
pub const MAX_PLAN_BYTES: usize = 256 * 1024;
pub const MAX_RECEIPT_BYTES: usize = 256 * 1024;

const DEFAULT_OPERATION_TIMEOUT_MS: u64 = 3_000;

#[derive(Clone, Debug)]
pub struct ValidatedPlan {
    pub payload: Value,
    pub ids: Vec<String>,
    pub deadline_ms: u64,
}

/// Validate and canonicalize a device-resident plan before transport.
pub fn validate_payload(payload: Value) -> Result<ValidatedPlan> {
    let encoded_size = serde_json::to_vec(&payload)?.len();
    if encoded_size > MAX_PLAN_BYTES {
        return Err(AuError::code(
            "E_LIMIT",
            format!("plan payload exceeds {MAX_PLAN_BYTES} bytes"),
        ));
    }

    let object = payload
        .as_object()
        .ok_or_else(|| AuError::code("E_ARGS", "plan.run payload must be an object"))?;
    reject_unknown(object, &["operations", "deadline_ms", "diagnostic"], "plan")?;

    let deadline_ms = match object.get("deadline_ms") {
        None => DEFAULT_DEADLINE_MS,
        Some(value) => bounded_u64(value, "deadline_ms", 1, MAX_DEADLINE_MS)?,
    };
    if let Some(value) = object.get("diagnostic") {
        if !value.is_boolean() {
            return Err(AuError::code("E_ARGS", "diagnostic must be a boolean"));
        }
    }

    let encoded_operations = object
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| AuError::code("E_ARGS", "plan.run requires an operations array"))?;
    if encoded_operations.is_empty() || encoded_operations.len() > MAX_STEPS {
        return Err(AuError::code(
            "E_LIMIT",
            format!("plan.run requires 1..{MAX_STEPS} operations"),
        ));
    }

    let mut ids = Vec::with_capacity(encoded_operations.len());
    let mut indexes = HashMap::with_capacity(encoded_operations.len());
    let mut operations = Vec::with_capacity(encoded_operations.len());
    let mut dependencies = Vec::with_capacity(encoded_operations.len());

    for (index, encoded) in encoded_operations.iter().enumerate() {
        let source = encoded.as_object().ok_or_else(|| {
            AuError::code(
                "E_ARGS",
                format!("plan operation {index} must be an object"),
            )
        })?;
        let id = required_string(source, "id")?;
        validate_id(&id, "operation id")?;
        if indexes.insert(id.clone(), index).is_some() {
            return Err(AuError::code(
                "E_ARGS",
                format!("duplicate plan operation id {id}"),
            ));
        }

        let raw_kind = required_string(source, "op")?;
        let kind = canonical_kind(&raw_kind)?;
        reject_unknown(source, fields_for(&kind), &format!("plan operation {id}"))?;

        let mut operation = Map::new();
        operation.insert("id".into(), Value::String(id.clone()));
        operation.insert("op".into(), Value::String(kind.clone()));

        let deps = match source.get("depends_on") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or_else(|| AuError::code("E_ARGS", "depends_on must be an array"))?
                .iter()
                .map(|value| {
                    let dependency = value
                        .as_str()
                        .ok_or_else(|| AuError::code("E_ARGS", "dependency ids must be strings"))?;
                    validate_id(dependency, "dependency id")?;
                    Ok(dependency.to_owned())
                })
                .collect::<Result<Vec<_>>>()?,
        };
        let mut seen_dependencies = HashSet::with_capacity(deps.len());
        for dependency in &deps {
            if !seen_dependencies.insert(dependency) {
                return Err(AuError::code(
                    "E_ARGS",
                    format!("duplicate dependency {dependency}"),
                ));
            }
        }
        if !deps.is_empty() {
            operation.insert(
                "depends_on".into(),
                Value::Array(deps.iter().cloned().map(Value::String).collect()),
            );
        }
        dependencies.push(deps);

        match kind.as_str() {
            "tap" => {
                let target = required_string(source, "target")?;
                validate_target(&target)?;
                operation.insert("target".into(), Value::String(target));
            }
            "text" => {
                let target = required_string(source, "target")?;
                validate_target(&target)?;
                let text = required_string_allow_empty(source, "text")?;
                if text.len() > MAX_TEXT_BYTES {
                    return Err(AuError::code(
                        "E_LIMIT",
                        format!("text exceeds {MAX_TEXT_BYTES} bytes"),
                    ));
                }
                operation.insert("target".into(), Value::String(target));
                operation.insert("text".into(), Value::String(text));
            }
            "scroll" => {
                let target = required_string(source, "target")?;
                validate_target(&target)?;
                let direction = optional_string(source, "direction")?;
                let direction = if direction.is_empty() {
                    "forward".to_owned()
                } else if matches!(direction.as_str(), "forward" | "backward") {
                    direction
                } else {
                    return Err(AuError::code(
                        "E_ARGS",
                        "scroll direction must be forward or backward",
                    ));
                };
                operation.insert("target".into(), Value::String(target));
                operation.insert("direction".into(), Value::String(direction));
            }
            "back" | "stop" => {}
            "wait.visible" | "assert.visible" => {
                let selector = required_string(source, "selector")?;
                validate_selector(&selector)?;
                let timeout_ms = operation_timeout(source)?;
                operation.insert("selector".into(), Value::String(selector));
                operation.insert("timeout_ms".into(), Value::from(timeout_ms));
            }
            "if" => {
                let selector = required_string(source, "selector")?;
                validate_selector(&selector)?;
                let then_id = optional_string(source, "then")?;
                let else_id = optional_string(source, "else")?;
                if then_id.is_empty() && else_id.is_empty() {
                    return Err(AuError::code(
                        "E_ARGS",
                        "if requires then and/or else target",
                    ));
                }
                operation.insert("selector".into(), Value::String(selector));
                if !then_id.is_empty() {
                    operation.insert("then".into(), Value::String(then_id));
                }
                if !else_id.is_empty() {
                    operation.insert("else".into(), Value::String(else_id));
                }
            }
            _ => unreachable!("canonical_kind validates operation kind"),
        }
        ids.push(id);
        operations.push(Value::Object(operation));
    }

    for (index, operation) in operations.iter().enumerate() {
        let source = operation
            .as_object()
            .expect("canonical operation is an object");
        for dependency in &dependencies[index] {
            let dependency_index = indexes.get(dependency).ok_or_else(|| {
                AuError::code("E_ARGS", format!("dependency does not exist: {dependency}"))
            })?;
            if *dependency_index >= index {
                return Err(AuError::code(
                    "E_ARGS",
                    format!("dependency must reference an earlier operation: {dependency}"),
                ));
            }
        }
        for field in ["then", "else"] {
            let Some(target) = source.get(field).and_then(Value::as_str) else {
                continue;
            };
            validate_forward_target(target, field, index, &indexes)?;
        }
    }

    let mut canonical = Map::new();
    canonical.insert("operations".into(), Value::Array(operations));
    canonical.insert("deadline_ms".into(), Value::from(deadline_ms));
    if let Some(diagnostic) = object.get("diagnostic") {
        canonical.insert("diagnostic".into(), diagnostic.clone());
    }

    Ok(ValidatedPlan {
        payload: Value::Object(canonical),
        ids,
        deadline_ms,
    })
}

/// Validate the compact or diagnostic receipt returned by plan.run.
pub fn validate_receipt(receipt: &Value, plan: &ValidatedPlan) -> Result<()> {
    if serde_json::to_vec(receipt)?.len() > MAX_RECEIPT_BYTES {
        return Err(AuError::code(
            "E_LIMIT",
            format!("plan receipts exceed {MAX_RECEIPT_BYTES} bytes"),
        ));
    }
    let object = receipt
        .as_object()
        .ok_or_else(|| AuError::code("E_PROTOCOL", "plan receipt must be an object"))?;
    if object.get("v").and_then(Value::as_u64) != Some(1) {
        return Err(AuError::code(
            "E_PROTOCOL",
            "unsupported plan receipt version",
        ));
    }
    let receipts = object
        .get("r")
        .and_then(Value::as_array)
        .ok_or_else(|| AuError::code("E_PROTOCOL", "plan receipt is missing r"))?;
    if receipts.len() != plan.ids.len() || receipts.len() > MAX_STEPS {
        return Err(AuError::code(
            "E_PROTOCOL",
            "plan receipt count is not bounded to the plan",
        ));
    }
    for (index, item) in receipts.iter().enumerate() {
        let (id, status) = if let Some(tuple) = item.as_array() {
            if tuple.len() < 3 {
                return Err(AuError::code(
                    "E_PROTOCOL",
                    "compact plan receipt is truncated",
                ));
            }
            (
                tuple[0]
                    .as_str()
                    .ok_or_else(|| AuError::code("E_PROTOCOL", "receipt id must be a string"))?,
                tuple[2].as_str().ok_or_else(|| {
                    AuError::code("E_PROTOCOL", "receipt status must be a string")
                })?,
            )
        } else if let Some(entry) = item.as_object() {
            (
                entry
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AuError::code("E_PROTOCOL", "receipt id is missing"))?,
                entry
                    .get("status")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AuError::code("E_PROTOCOL", "receipt status is missing"))?,
            )
        } else {
            return Err(AuError::code(
                "E_PROTOCOL",
                "plan receipt entry must be an object or tuple",
            ));
        };
        if id != plan.ids[index]
            || !matches!(
                status,
                "accepted" | "committed" | "observed" | "failed" | "skipped"
            )
        {
            return Err(AuError::code(
                "E_PROTOCOL",
                "plan receipt order or status is invalid",
            ));
        }
    }
    for field in ["m", "failed", "skipped"] {
        if object.get(field).and_then(Value::as_u64).unwrap_or(0) > MAX_STEPS as u64 {
            return Err(AuError::code(
                "E_PROTOCOL",
                "plan receipt count exceeds the step bound",
            ));
        }
    }
    Ok(())
}

fn canonical_kind(kind: &str) -> Result<String> {
    let canonical = match kind {
        "tap" | "text" | "scroll" | "back" | "wait.visible" | "assert.visible" | "if" | "stop" => {
            kind
        }
        "assert" => "assert.visible",
        _ => {
            return Err(AuError::code(
                "E_ARGS",
                format!("unsupported plan operation {kind}"),
            ))
        }
    };
    Ok(canonical.into())
}

fn fields_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "tap" => &["id", "op", "depends_on", "target"],
        "text" => &["id", "op", "depends_on", "target", "text"],
        "scroll" => &["id", "op", "depends_on", "target", "direction"],
        "wait.visible" | "assert.visible" => &["id", "op", "depends_on", "selector", "timeout_ms"],
        "if" => &["id", "op", "depends_on", "selector", "then", "else"],
        "back" | "stop" => &["id", "op", "depends_on"],
        _ => &[],
    }
}

fn reject_unknown(object: &Map<String, Value>, allowed: &[&str], label: &str) -> Result<()> {
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(AuError::code(
            "E_ARGS",
            format!("{label} has unknown field {field}"),
        ));
    }
    Ok(())
}

fn required_string(object: &Map<String, Value>, field: &str) -> Result<String> {
    let value = object
        .get(field)
        .ok_or_else(|| AuError::code("E_ARGS", format!("{field} is required")))?;
    let value = value
        .as_str()
        .ok_or_else(|| AuError::code("E_ARGS", format!("{field} must be a string")))?;
    if value.is_empty() {
        return Err(AuError::code(
            "E_ARGS",
            format!("{field} must not be empty"),
        ));
    }
    Ok(value.into())
}

fn required_string_allow_empty(object: &Map<String, Value>, field: &str) -> Result<String> {
    let value = object
        .get(field)
        .ok_or_else(|| AuError::code("E_ARGS", format!("{field} is required")))?;
    value
        .as_str()
        .map(Into::into)
        .ok_or_else(|| AuError::code("E_ARGS", format!("{field} must be a string")))
}

fn optional_string(object: &Map<String, Value>, field: &str) -> Result<String> {
    match object.get(field) {
        None => Ok(String::new()),
        Some(value) => value
            .as_str()
            .map(Into::into)
            .ok_or_else(|| AuError::code("E_ARGS", format!("{field} must be a string"))),
    }
}

fn bounded_u64(value: &Value, field: &str, minimum: u64, maximum: u64) -> Result<u64> {
    let number = value
        .as_u64()
        .ok_or_else(|| AuError::code("E_ARGS", format!("{field} must be an unsigned integer")))?;
    if !(minimum..=maximum).contains(&number) {
        return Err(AuError::code(
            "E_LIMIT",
            format!("{field} must be {minimum}..{maximum}"),
        ));
    }
    Ok(number)
}

fn operation_timeout(object: &Map<String, Value>) -> Result<u64> {
    object
        .get("timeout_ms")
        .map(|value| bounded_u64(value, "timeout_ms", 1, MAX_DEADLINE_MS))
        .unwrap_or(Ok(DEFAULT_OPERATION_TIMEOUT_MS))
}

fn validate_id(id: &str, label: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > MAX_ID_BYTES
        || !id.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && byte.is_ascii_alphanumeric())
                || (index > 0 && (byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)))
        })
    {
        return Err(AuError::code(
            "E_ARGS",
            format!("{label} has invalid characters"),
        ));
    }
    Ok(())
}

fn validate_target(target: &str) -> Result<()> {
    if target.len() > MAX_SELECTOR_BYTES {
        return Err(AuError::code(
            "E_LIMIT",
            format!("semantic target exceeds {MAX_SELECTOR_BYTES} bytes"),
        ));
    }
    if target.bytes().all(|byte| byte.is_ascii_digit()) {
        if target.len() > 20 || target.parse::<u64>().is_err() {
            return Err(AuError::code(
                "E_ARGS",
                "numeric semantic target is out of range",
            ));
        }
        return Ok(());
    }
    if let Some(session_id) = target.strip_prefix('s') {
        if (1..=32).contains(&session_id.len())
            && session_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return Ok(());
        }
    }
    validate_selector(target)
}

fn validate_selector(selector: &str) -> Result<()> {
    if selector.len() > MAX_SELECTOR_BYTES {
        return Err(AuError::code(
            "E_LIMIT",
            format!("selector exceeds {MAX_SELECTOR_BYTES} bytes"),
        ));
    }
    crate::selector::Selector::parse(selector)
        .map(|_| ())
        .map_err(|error| AuError::code("E_ARGS", error.compact_message()))
}

fn validate_forward_target(
    target: &str,
    field: &str,
    source_index: usize,
    indexes: &HashMap<String, usize>,
) -> Result<()> {
    validate_id(target, field)?;
    let target_index = indexes.get(target).ok_or_else(|| {
        AuError::code("E_ARGS", format!("{field} target does not exist: {target}"))
    })?;
    if *target_index <= source_index {
        return Err(AuError::code(
            "E_ARGS",
            format!("{field} target must be forward-only: {target}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan(operations: Value) -> Value {
        json!({"operations": operations})
    }

    fn op(id: &str, kind: &str) -> Value {
        json!({"id": id, "op": kind})
    }

    #[test]
    fn accepts_all_safe_operations_and_canonicalizes_assert() {
        let value = plan(json!([
            {"id":"tap", "op":"tap", "target":"text=Go"},
            {"id":"text", "op":"text", "target":"s1", "text":""},
            {"id":"scroll", "op":"scroll", "target":"scrollable=true#0", "direction":"forward"},
            {"id":"back", "op":"back", "depends_on":["tap"]},
            {"id":"wait", "op":"wait.visible", "selector":"text=Ready"},
            {"id":"assert", "op":"assert", "selector":"text=Ready"},
            {"id":"choose", "op":"if", "selector":"text=Ready", "then":"stop"},
            {"id":"stop", "op":"stop"}
        ]));
        let validated = validate_payload(value).expect("safe plan");
        assert_eq!(validated.ids.len(), 8);
        assert_eq!(validated.deadline_ms, MAX_DEADLINE_MS);
        assert_eq!(validated.payload["operations"][5]["op"], "assert.visible");
    }

    #[test]
    fn rejects_limits_unknown_operations_and_destructive_operations() {
        let too_many = (0..=MAX_STEPS)
            .map(|index| op(&format!("op{index}"), "back"))
            .collect::<Vec<_>>();
        assert_eq!(
            validate_payload(plan(json!(too_many)))
                .expect_err("limit")
                .kind(),
            "E_LIMIT"
        );
        assert_eq!(
            validate_payload(plan(json!([op("one", "install")])))
                .expect_err("unsafe")
                .kind(),
            "E_ARGS"
        );
        assert_eq!(
            validate_payload(json!({"operations":[], "deadline_ms":MAX_DEADLINE_MS + 1}))
                .expect_err("deadline")
                .kind(),
            "E_LIMIT"
        );
    }

    #[test]
    fn rejects_backward_edges_missing_dependencies_and_cycles_before_device() {
        let backward = plan(json!([
            {"id":"first", "op":"back"},
            {"id":"branch", "op":"if", "selector":"text=Ready", "then":"first"}
        ]));
        assert_eq!(
            validate_payload(backward).expect_err("backward").kind(),
            "E_ARGS"
        );

        let missing = plan(json!([
            {"id":"later", "op":"back", "depends_on":["missing"]}
        ]));
        assert_eq!(
            validate_payload(missing).expect_err("missing").kind(),
            "E_ARGS"
        );

        let cycle = plan(json!([
            {"id":"a", "op":"back", "depends_on":["b"]},
            {"id":"b", "op":"back", "depends_on":["a"]}
        ]));
        assert_eq!(validate_payload(cycle).expect_err("cycle").kind(), "E_ARGS");
    }

    #[test]
    fn accepts_bounded_compact_receipts_and_rejects_reordered_or_extra_receipts() {
        let validated = validate_payload(plan(json!([
            {"id":"tap", "op":"tap", "target":"s1"},
            {"id":"stop", "op":"stop"}
        ])))
        .expect("plan");
        let compact = json!({
            "v": 1,
            "m": 1,
            "r": [
                ["tap", "tap", "committed", 3, 1, null, null, null, null],
                ["stop", "stop", "accepted", 1, 0, null, null, null, ""]
            ]
        });
        validate_receipt(&compact, &validated).expect("receipt");

        let mut reordered = compact.clone();
        reordered["r"][0][0] = json!("stop");
        assert_eq!(
            validate_receipt(&reordered, &validated)
                .expect_err("reordered")
                .kind(),
            "E_PROTOCOL"
        );
        let mut extra = compact;
        extra["r"] = json!([]);
        assert_eq!(
            validate_receipt(&extra, &validated)
                .expect_err("unbounded")
                .kind(),
            "E_PROTOCOL"
        );
    }
}
