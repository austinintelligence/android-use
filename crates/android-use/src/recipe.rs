use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::actions::{ActionResult, Brief};
use crate::config::AppPaths;
use crate::contract::{ExecuteParams, PlanStep};
use crate::error::{AuError, Result};

const RECIPE_SCHEMA: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<PlanStep>,
    #[serde(default = "default_max_mutations")]
    pub max_mutations: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeCall {
    pub name: String,
    #[serde(default)]
    pub device: crate::contract::DeviceRef,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub inputs: Value,
}

pub fn action(paths: &AppPaths, args: &[String]) -> Result<ActionResult> {
    let operation = args.first().map(String::as_str).unwrap_or("list");
    match operation {
        "list" => Ok(ActionResult {
            brief: Brief::Ok,
            data: json!({"recipes":list(paths)?}),
        }),
        "show" => {
            let name = args
                .get(1)
                .ok_or_else(|| AuError::code("E_ARGS", "recipe show requires a name"))?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: serde_json::to_value(load(paths, name)?)?,
            })
        }
        "run" => {
            let name = args
                .get(1)
                .ok_or_else(|| AuError::code("E_ARGS", "recipe run requires a name"))?;
            let recipe = load(paths, name)?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: crate::runtime::execute_plan(ExecuteParams {
                    steps: recipe.steps,
                    max_mutations: recipe.max_mutations,
                    ..ExecuteParams::default()
                })?,
            })
        }
        _ => Err(AuError::code("E_ARGS", "recipe expects list, show, or run")),
    }
}

pub fn contract_call(params: &Value) -> Result<Value> {
    let call: RecipeCall = serde_json::from_value(params.clone())
        .map_err(|error| AuError::code("E_ARGS", format!("invalid recipe request: {error}")))?;
    let paths = AppPaths::discover()?;
    let recipe = load(&paths, &call.name)?;
    let steps = materialize_steps(&recipe.steps, &call.inputs)?;
    if call.dry_run {
        return Ok(
            json!({"name":recipe.name,"description":recipe.description,"steps":steps,"dry_run":true,"inputs":call.inputs}),
        );
    }
    crate::runtime::execute_plan(ExecuteParams {
        device: call.device,
        steps,
        max_mutations: recipe.max_mutations,
        ..ExecuteParams::default()
    })
}

fn materialize_steps(steps: &[PlanStep], inputs: &Value) -> Result<Vec<PlanStep>> {
    let values = inputs.as_object().cloned().unwrap_or_default();
    steps
        .iter()
        .map(|step| {
            let mut value = serde_json::to_value(step)?;
            substitute(&mut value, &values)?;
            serde_json::from_value(value).map_err(|error| {
                AuError::code("E_RECIPE", format!("invalid materialized step: {error}"))
            })
        })
        .collect()
}

fn substitute(value: &mut Value, inputs: &serde_json::Map<String, Value>) -> Result<()> {
    match value {
        Value::Array(values) => {
            for value in values {
                substitute(value, inputs)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                substitute(value, inputs)?;
            }
        }
        Value::String(text) => {
            let mut result = text.clone();
            let mut cursor = 0usize;
            while let Some(start) = result[cursor..].find("${") {
                let start = cursor + start;
                let Some(end_offset) = result[start + 2..].find('}') else {
                    return Err(AuError::code(
                        "E_RECIPE",
                        "unterminated recipe input placeholder",
                    ));
                };
                let end = start + 2 + end_offset;
                let key = &result[start + 2..end];
                let input = inputs.get(key).ok_or_else(|| {
                    AuError::code("E_RECIPE", format!("missing recipe input {key}"))
                })?;
                let replacement = match input {
                    Value::String(value) => value.clone(),
                    Value::Number(value) => value.to_string(),
                    Value::Bool(value) => value.to_string(),
                    _ => {
                        return Err(AuError::code(
                            "E_RECIPE",
                            format!("recipe input {key} must be scalar"),
                        ))
                    }
                };
                result.replace_range(start..=end, &replacement);
                cursor = start + replacement.len();
            }
            *text = result;
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn default_max_mutations() -> usize {
    8
}

fn recipe_dir(paths: &AppPaths) -> PathBuf {
    paths.root.join("recipes")
}

fn list(paths: &AppPaths) -> Result<Vec<String>> {
    let directory = recipe_dir(paths);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                names.push(stem.to_owned());
            }
        }
    }
    names.sort();
    Ok(names)
}

fn load(paths: &AppPaths, name: &str) -> Result<Recipe> {
    validate_name(name)?;
    let path = recipe_dir(paths).join(format!("{name}.json"));
    let text = fs::read_to_string(&path)
        .map_err(|error| AuError::code("E_RECIPE", format!("read recipe {name}: {error}")))?;
    let recipe: Recipe = serde_json::from_str(&text)
        .map_err(|error| AuError::code("E_RECIPE", format!("invalid recipe {name}: {error}")))?;
    validate(&recipe)?;
    Ok(recipe)
}

fn validate(recipe: &Recipe) -> Result<()> {
    if recipe.schema != RECIPE_SCHEMA {
        return Err(AuError::code("E_RECIPE", "unsupported recipe schema"));
    }
    if recipe.name.is_empty()
        || recipe.steps.is_empty()
        || recipe.steps.len() > crate::contract::MAX_CONTRACT_STEPS
    {
        return Err(AuError::code(
            "E_RECIPE",
            "recipe name and 1..32 steps are required",
        ));
    }
    if recipe.max_mutations > crate::contract::MAX_CONTRACT_MUTATIONS {
        return Err(AuError::code(
            "E_RECIPE",
            "recipe mutation limit is too high",
        ));
    }
    for step in &recipe.steps {
        if matches!(step.op.as_str(), "raw" | "shell" | "adb" | "file") {
            return Err(AuError::code(
                "E_RECIPE",
                "recipes cannot contain raw or unrestricted operations",
            ));
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 96
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Err(AuError::code(
            "E_RECIPE",
            "recipe name is not a safe local name",
        ));
    }
    Ok(())
}

#[allow(dead_code)]
fn _recipe_path(paths: &AppPaths, name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(recipe_dir(paths).join(format!("{name}.json")))
}
