use std::fs;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adb::Adb;
use crate::config::{atomic_write, AppPaths};
use crate::error::{AuError, Result};
use crate::helper::{self, HELPER_PACKAGE};
use crate::process::text;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LocationJournal {
    pub serial: String,
    pub app_op_before: String,
    pub provider_state_before: String,
    pub created_at_ms: u128,
}

#[derive(Clone, Debug)]
struct RoutePoint {
    latitude: f64,
    longitude: f64,
    delay_ms: u64,
}

pub fn execute(adb: &Adb, paths: &AppPaths, serial: &str, args: &[String]) -> Result<Value> {
    let operation = args.first().map(String::as_str).unwrap_or("status");
    match operation {
        "status" => status(
            adb,
            paths,
            serial,
            args.iter().any(|value| value == "--expanded"),
        ),
        "get" => get(adb, paths, serial),
        "enable" => enable(adb, paths, serial),
        "disable" => disable(adb, paths, serial),
        "set" => set(adb, paths, serial, args),
        "clear" => clear(adb, paths, serial),
        "route" => route(adb, paths, serial, args),
        _ => Err(AuError::code(
            "E_ARGS",
            format!("unknown loc operation {operation}"),
        )),
    }
}

pub fn status(adb: &Adb, paths: &AppPaths, serial: &str, expanded: bool) -> Result<Value> {
    let app_op = app_op(adb, serial)?;
    let providers = provider_state(adb, serial)?;
    let helper_state = match helper::call(adb, paths, serial, "location.status", json!({})) {
        Ok(value) => json!({"available":true,"state":public_helper_state(value)}),
        Err(error) => {
            json!({"available":false,"code":error.kind(),"message":error.compact_message()})
        }
    };
    let journal = load_journal(paths)?;
    Ok(json!({
        "serial":serial,
        "app_op":app_op,
        "providers":if expanded { limit(providers, 12_000) } else { summarize_providers(&providers) },
        "helper":helper_state,
        "journal":journal.as_ref().map(public_journal),
        "uncleared":journal.as_ref().is_some_and(|entry| entry.serial == serial)
    }))
}

fn get(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<Value> {
    helper::call(adb, paths, serial, "location.get", json!({}))
}

fn enable(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<Value> {
    ensure_journal(adb, paths, serial)?;
    set_app_op(adb, serial, "allow")?;
    Ok(json!({"package":HELPER_PACKAGE,"mock_location":"allow","journaled":true}))
}

fn disable(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<Value> {
    if load_journal(paths)?.is_some() {
        return clear(adb, paths, serial);
    }
    set_app_op(adb, serial, "default")?;
    Ok(json!({"package":HELPER_PACKAGE,"mock_location":"default","journaled":false}))
}

fn set(adb: &Adb, paths: &AppPaths, serial: &str, args: &[String]) -> Result<Value> {
    let latitude = coordinate(args, 1, "latitude")?;
    let longitude = coordinate(args, 2, "longitude")?;
    validate_coordinates(latitude, longitude)?;
    ensure_journal(adb, paths, serial)?;
    set_app_op(adb, serial, "allow")?;
    let helper = helper::call(
        adb,
        paths,
        serial,
        "location.set",
        json!({"latitude":latitude,"longitude":longitude}),
    )?;
    Ok(json!({"latitude":latitude,"longitude":longitude,"persistent":true,"helper":helper}))
}

pub fn clear(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<Value> {
    let Some(journal) = load_journal(paths)? else {
        return Ok(json!({"cleared":false,"reason":"no journal"}));
    };
    if journal.serial != serial {
        return Err(AuError::code(
            "E_LOCATION",
            format!("journal belongs to {}, not {serial}", journal.serial),
        ));
    }
    // Do not delete the journal unless both the helper's owned providers and the
    // original app-op have been restored. It is intentionally recovery evidence.
    let helper = helper::call(adb, paths, serial, "location.clear", json!({}))?;
    let restored_app_op = restore_app_op(adb, serial, &journal.app_op_before)?;
    fs::remove_file(&paths.location_journal)?;
    Ok(json!({
        "cleared":true,
        "restored_app_op":restored_app_op,
        "provider_state_before":summarize_providers(&journal.provider_state_before),
        "provider_state_restored":true,
        "helper":helper
    }))
}

fn route(adb: &Adb, paths: &AppPaths, serial: &str, args: &[String]) -> Result<Value> {
    let path = args
        .get(1)
        .ok_or_else(|| AuError::code("E_ARGS", "loc route CSV_OR_GPX [--speed N] [--loop]"))?;
    let speed = parse_speed(args)?;
    let looping = args.iter().any(|value| value == "--loop");
    let points = parse_route(path)?;
    if points.is_empty() {
        return Err(AuError::code(
            "E_LOCATION",
            "route contains no valid points",
        ));
    }
    ensure_journal(adb, paths, serial)?;
    set_app_op(adb, serial, "allow")?;
    let iterations = if looping { 2 } else { 1 };
    for _ in 0..iterations {
        for point in &points {
            validate_coordinates(point.latitude, point.longitude)?;
            helper::call(
                adb,
                paths,
                serial,
                "location.set",
                json!({"latitude":point.latitude,"longitude":point.longitude}),
            )?;
            let wait = (point.delay_ms as f64 / speed).round() as u64;
            thread::sleep(Duration::from_millis(wait.min(10_000)));
        }
    }
    Ok(json!({"points":points.len(),"looped":looping,"speed":speed,"persistent":true}))
}

fn ensure_journal(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<()> {
    if let Some(existing) = load_journal(paths)? {
        if existing.serial != serial {
            return Err(AuError::code(
                "E_LOCATION",
                format!("existing location journal belongs to {}", existing.serial),
            ));
        }
        return Ok(());
    }
    let journal = LocationJournal {
        serial: serial.into(),
        app_op_before: app_op(adb, serial)?,
        provider_state_before: provider_state(adb, serial)?,
        created_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    };
    atomic_write(&paths.location_journal, &serde_json::to_vec(&journal)?)
}

fn app_op(adb: &Adb, serial: &str) -> Result<String> {
    shell_text(
        adb,
        serial,
        &["appops", "get", HELPER_PACKAGE, "android:mock_location"],
    )
}

fn provider_state(adb: &Adb, serial: &str) -> Result<String> {
    // Android 13 exposes `cmd location providers` as a command family, not a
    // standalone status query; calling it without a subcommand fails with
    // "Argument expected after providers". `dumpsys location` is the stable
    // read-only diagnostic surface and records the master switch plus every
    // provider's enabled/test state for the restoration journal.
    shell_text(adb, serial, &["dumpsys", "location"])
}

fn summarize_providers(input: &str) -> String {
    let mut lines = Vec::new();
    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.contains("last location")
            || line.contains("Location[")
            || line.contains("latitude")
            || line.contains("longitude")
        {
            continue;
        }
        if line.contains("Location Setting")
            || line.contains("provider:")
            || line.contains("enabled=")
            || line.contains("allowed=")
            || line.starts_with("Location Manager State")
        {
            lines.push(line);
        }
        if lines.len() >= 16 {
            break;
        }
    }
    let summary = lines.join(";");
    limit(summary, 1_600)
}

fn set_app_op(adb: &Adb, serial: &str, mode: &str) -> Result<()> {
    adb.device(
        serial,
        &[
            "shell".into(),
            "appops".into(),
            "set".into(),
            HELPER_PACKAGE.into(),
            "android:mock_location".into(),
            mode.into(),
        ],
    )?;
    Ok(())
}

fn restore_app_op(adb: &Adb, serial: &str, before: &str) -> Result<&'static str> {
    if before.contains("No operations.") || before.contains("Default mode:") {
        adb.device(
            serial,
            &[
                "shell".into(),
                "appops".into(),
                "reset".into(),
                "--user".into(),
                "0".into(),
                HELPER_PACKAGE.into(),
            ],
        )?;
        return Ok("reset");
    }
    let mode = extract_app_op_mode(before);
    set_app_op(adb, serial, mode)?;
    Ok(mode)
}

fn shell_text(adb: &Adb, serial: &str, command: &[&str]) -> Result<String> {
    let mut args = vec!["shell".into()];
    args.extend(command.iter().map(|part| (*part).into()));
    let result = adb.device(serial, &args)?;
    Ok(text(&result.stdout))
}

fn coordinate(args: &[String], index: usize, name: &str) -> Result<f64> {
    args.get(index)
        .ok_or_else(|| AuError::code("E_ARGS", format!("loc set requires {name}")))?
        .parse()
        .map_err(|_| AuError::code("E_ARGS", format!("invalid {name}")))
}

fn validate_coordinates(latitude: f64, longitude: f64) -> Result<()> {
    if (-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude) {
        Ok(())
    } else {
        Err(AuError::code("E_ARGS", "latitude/longitude out of range"))
    }
}

fn extract_app_op_mode(value: &str) -> &'static str {
    if value.contains("No operations.") || value.contains("Default mode:") {
        return "default";
    }
    if value.contains("allow") {
        "allow"
    } else if value.contains("ignore") {
        "ignore"
    } else if value.contains("deny") {
        "deny"
    } else {
        "default"
    }
}

fn load_journal(paths: &AppPaths) -> Result<Option<LocationJournal>> {
    if !paths.location_journal.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&paths.location_journal)?;
    Ok(Some(serde_json::from_str(&text).map_err(|error| {
        AuError::code("E_LOCATION", format!("corrupt location journal: {error}"))
    })?))
}

fn parse_speed(args: &[String]) -> Result<f64> {
    let Some(index) = args.iter().position(|value| value == "--speed") else {
        return Ok(1.0);
    };
    let speed: f64 = args
        .get(index + 1)
        .ok_or_else(|| AuError::code("E_ARGS", "--speed requires a value"))?
        .parse()
        .map_err(|_| AuError::code("E_ARGS", "invalid --speed"))?;
    if !(0.05..=100.0).contains(&speed) {
        return Err(AuError::code("E_ARGS", "--speed must be 0.05..100"));
    }
    Ok(speed)
}

fn parse_route(path: &str) -> Result<Vec<RoutePoint>> {
    let text = fs::read_to_string(path)?;
    if path.to_ascii_lowercase().ends_with(".gpx") {
        return parse_gpx(&text);
    }
    let points = text
        .lines()
        .filter_map(|line| {
            let values = line.split(',').map(str::trim).collect::<Vec<_>>();
            if values.len() < 2 || values[0].eq_ignore_ascii_case("latitude") {
                return None;
            }
            let latitude = values[0].parse().ok()?;
            let longitude = values[1].parse().ok()?;
            let delay_ms = values
                .get(2)
                .and_then(|value| value.parse().ok())
                .unwrap_or(1_000);
            Some(RoutePoint {
                latitude,
                longitude,
                delay_ms,
            })
        })
        .collect();
    Ok(points)
}

fn parse_gpx(text: &str) -> Result<Vec<RoutePoint>> {
    let mut points = Vec::new();
    for segment in text.split("<trkpt").skip(1) {
        let latitude = attribute(segment, "lat")
            .ok_or_else(|| AuError::code("E_LOCATION", "GPX trkpt missing lat"))?;
        let longitude = attribute(segment, "lon")
            .ok_or_else(|| AuError::code("E_LOCATION", "GPX trkpt missing lon"))?;
        points.push(RoutePoint {
            latitude: latitude
                .parse()
                .map_err(|_| AuError::code("E_LOCATION", "invalid GPX latitude"))?,
            longitude: longitude
                .parse()
                .map_err(|_| AuError::code("E_LOCATION", "invalid GPX longitude"))?,
            delay_ms: 1_000,
        });
    }
    Ok(points)
}

fn attribute<'a>(segment: &'a str, name: &str) -> Option<&'a str> {
    let start = segment.find(&format!("{name}=\""))? + name.len() + 2;
    let value = &segment[start..];
    value.split('"').next()
}

fn limit(text: String, max: usize) -> String {
    text.chars().take(max).collect()
}

fn public_journal(journal: &LocationJournal) -> Value {
    json!({
        "serial": journal.serial,
        "app_op_before": journal.app_op_before,
        "provider_state_before": summarize_providers(&journal.provider_state_before),
        "created_at_ms": journal.created_at_ms
    })
}

fn public_helper_state(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("latitude");
        object.remove("longitude");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{extract_app_op_mode, parse_gpx, summarize_providers, validate_coordinates};

    #[test]
    fn restores_a_real_app_op_mode() {
        assert_eq!(extract_app_op_mode("android:mock_location: deny"), "deny");
        assert_eq!(extract_app_op_mode("No operations."), "default");
        assert_eq!(
            extract_app_op_mode("No operations.\nDefault mode: deny"),
            "default"
        );
    }

    #[test]
    fn parses_minimal_gpx_route() {
        let route = parse_gpx("<gpx><trkpt lat=\"1.5\" lon=\"2.5\"/></gpx>").expect("gpx");
        assert_eq!(route.len(), 1);
    }

    #[test]
    fn coordinates_are_bounded_before_location_state_changes() {
        assert!(validate_coordinates(1.0, 2.0).is_ok());
        assert!(validate_coordinates(91.0, 2.0).is_err());
    }

    #[test]
    fn provider_summary_excludes_last_coordinates() {
        let summary = summarize_providers(
            "Location Manager State:\nLocation Setting: true\n  gps provider:\n    last location=Location[gps 36.1,-95.8]\n    enabled=true",
        );
        assert!(summary.contains("Location Setting: true"));
        assert!(summary.contains("enabled=true"));
        assert!(!summary.contains("mock provider"));
        assert!(!summary.contains("36.1"));
        assert!(!summary.contains("Location["));
    }

    #[test]
    fn public_helper_state_excludes_coordinates() {
        let value = super::public_helper_state(serde_json::json!({
            "latitude": 1.0,
            "longitude": 2.0,
            "owned_providers": ["au_gps"]
        }));
        assert!(value.get("latitude").is_none());
        assert!(value.get("longitude").is_none());
        assert!(value.get("owned_providers").is_some());
    }
}
