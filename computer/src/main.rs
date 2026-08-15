#![forbid(unsafe_code)]

use au::{
    adapter,
    api::{BrowserPlan, Code, Error, Plan, Range, Result, VisualPlan},
    engine::Engine,
    install,
};
use serde_json::Value;
use std::{env, io::IsTerminal, path::Path};

fn main() {
    let human = env::args().any(|arg| arg == "--human") || (!env::args().any(|arg| arg == "--json") && std::io::stdout().is_terminal());
    let command = env::args().skip(1).find(|arg| arg != "--json" && arg != "--human").unwrap_or_else(|| "help".into());
    match run(human) {
        Ok(value) => {
            if human {
                println!("{}", human_value(&command, &value));
            } else {
                println!("{}", serde_json::to_string(&value).unwrap_or_else(|_| "{\"ok\":0,\"e\":\"internal\"}".into()));
            }
        }
        Err(e) => {
            if human {
                eprintln!("{}", human_error(&e));
            } else {
                println!("{}", e.json());
            }
            std::process::exit(1)
        }
    }
}
fn run(human: bool) -> Result<Value> {
    let a: Vec<String> = env::args().skip(1).filter(|arg| arg != "--json" && arg != "--human").collect();
    let out = match a.first().map(String::as_str).unwrap_or("help") {
        "serve" => {
            let e = Engine::open()?;
            match a.get(1).map(String::as_str) {
                Some("--mcp") => return adapter::mcp(e).map(|_| Value::Null),
                Some("--jsonl") => return adapter::jsonl(e).map(|_| Value::Null),
                _ => return Err(Error::new(Code::Args, "serve requires --mcp or --jsonl")),
            }
        }
        "status" => adapter::one_read(&mut Engine::open()?, "status", None, 0, None, None)?,
        "devices" => install::devices()?,
        "observe" => {
            let base = a.get(1).filter(|s| s.as_str() != "--detail").map(String::as_str);
            let detail = u8::from(a.iter().any(|s| s == "--detail"));
            adapter::one_read(&mut Engine::open()?, "observe", base, detail, None, None)?
        }
        "browser" => {
            let op = a.get(1).map(String::as_str).unwrap_or("observe");
            adapter::one_read(&mut Engine::open()?, "browser", Some(op), 0, None, None)?
        }
        "capabilities" | "location" | "notifications" => adapter::one_read(&mut Engine::open()?, a[0].as_str(), None, 0, None, None)?,
        "visual" => {
            let mut engine = Engine::open()?;
            match a.get(1).map(String::as_str) {
                Some("hash") => engine.read(au::api::Read::Visual(au::api::VisualRead::Hash(
                    a.get(2).ok_or_else(|| Error::new(Code::Args, "visual hash requires an artifact id"))?.clone().into_boxed_str(),
                )))?,
                Some("diff") => engine.read(au::api::Read::Visual(au::api::VisualRead::Diff(
                    a.get(2).ok_or_else(|| Error::new(Code::Args, "visual diff requires two artifact ids"))?.clone().into_boxed_str(),
                    a.get(3).ok_or_else(|| Error::new(Code::Args, "visual diff requires two artifact ids"))?.clone().into_boxed_str(),
                )))?,
                _ => return Err(Error::new(Code::Args, "visual requires hash ID or diff ID ID")),
            }
        }
        "act" => {
            let raw = a.get(1).ok_or_else(|| Error::new(Code::Args, "act requires one JSON object"))?;
            let v: Value = serde_json::from_str(raw)?;
            let mut engine = Engine::open()?;
            match v.get("target").and_then(Value::as_str) {
                Some("browser") => engine.browser_act(BrowserPlan::parse(v)?)?,
                Some("visual") => engine.visual_act(VisualPlan::parse(v)?)?,
                _ => engine.act(Plan::parse(v)?)?,
            }
        }
        "artifact" => {
            let id = a.get(1).ok_or_else(|| Error::new(Code::Args, "artifact requires an id"))?;
            let range = if a.len() == 4 { Some(Range { start: parse(&a[2])?, end: parse(&a[3])? }) } else { None };
            adapter::one_read(&mut Engine::open()?, "artifact", None, 0, Some(id), range)?
        }
        "enroll" => install::enroll(a.get(1).ok_or_else(|| Error::new(Code::Args, "enroll requires an ADB endpoint"))?)?,
        "setup" | "repair" => install::setup(a.get(1).map(Path::new), human)?,
        "update" => install::update(a.get(1).map(Path::new), human)?,
        "doctor" => install::doctor()?,
        "uninstall" => install::uninstall()?,
        "version" | "--version" => serde_json::json!({"version":env!("CARGO_PKG_VERSION")}),
        "help" | "--help" | "-h" => {
            serde_json::json!({"commands":["devices","setup [APK]","status","doctor","update [APK]","uninstall","enroll ENDPOINT","repair [APK]","serve --mcp|--jsonl","observe [BASE] [--detail]","browser tabs|observe|text","capabilities","location","notifications","visual hash|diff","act JSON","artifact ID [START END]"]})
        }
        _ => return Err(Error::new(Code::Args, format!("unknown command: {}", a[0]))),
    };
    Ok(out)
}
fn parse(s: &str) -> Result<u64> {
    s.parse().map_err(|_| Error::new(Code::Args, "range values must be unsigned integers"))
}

fn human_value(command: &str, value: &Value) -> String {
    match command {
        "help" | "--help" | "-h" => "Android Use — Give AI an Android device.\n\nGet connected:\n  au devices      List connected Android devices\n  au setup        Prepare and remember one device\n  au status       Check whether Android Use is ready\n  au doctor       Explain anything that needs attention\n\nUse the device:\n  au observe      Read the visible interface\n  au browser      Read Chrome tabs or page state\n  au capabilities Show optional device capabilities\n  au notifications Read available notifications\n  au location     Read the current location\n\nConnect an agent:\n  au serve --mcp   Model Context Protocol over stdio\n  au serve --jsonl Typed JSON Lines over stdio\n\nMaintenance:\n  au update       Update the Android helper\n  au uninstall    Remove Android Use and its local state\n\nRun `au help --json` for the complete machine-readable command list.".into(),
        "version" | "--version" => format!("Android Use {}", value.get("version").and_then(Value::as_str).unwrap_or(env!("CARGO_PKG_VERSION"))),
        "status" => {
            if value.get("ok").and_then(Value::as_u64) == Some(1) {
                format!("Android Use is ready\n\n✓ Android helper connected\n✓ UI generation {}\n✓ Capability mask {}", value["g"], value["cap"])
            } else {
                "Android Use is not ready.\nRun: au doctor".into()
            }
        }
        "devices" => {
            let devices = value.get("devices").and_then(Value::as_array);
            match devices {
                Some(items) if !items.is_empty() => {
                    let mut out = format!("Connected Android devices ({})", items.len());
                    for item in items {
                        out.push_str("\n\n✓ ");
                        out.push_str(item.get("endpoint").and_then(Value::as_str).unwrap_or("Android device"));
                        if let Some(state) = item.get("state").and_then(Value::as_str) {
                            out.push_str(" — ");
                            out.push_str(state);
                        }
                    }
                    out
                }
                _ => "No ready Android device found.\n\nConnect and unlock one device, enable USB debugging, then approve this computer on Android.".into(),
            }
        }
        "setup" | "repair" | "update" => {
            let ready = value.get("ready").and_then(Value::as_u64) == Some(1);
            let mut out = if ready { "Android Use is ready".to_owned() } else { "Android Use is almost ready".to_owned() };
            out.push_str("\n\n✓ Android device connected\n✓ Android Use helper installed");
            if value.get("enrolled").and_then(Value::as_u64) == Some(1) {
                out.push_str("\n✓ Device remembered");
            }
            if value.get("accessibility").and_then(Value::as_u64) == Some(1) {
                out.push_str("\n✓ Accessibility enabled");
            } else {
                out.push_str("\n! Accessibility still needs your approval");
            }
            if value.get("settings_opened").and_then(Value::as_u64) == Some(1) && !ready {
                out.push_str("\n✓ Opened the right Android settings screen");
            }
            out.push_str(&human_next_step(value));
            out
        }
        "doctor" => human_doctor(value),
        "uninstall" => "Android Use was removed from the enrolled device.\n\nLocal Android Use state was removed too.".into(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| "Android Use returned an unreadable result.".into()),
    }
}

fn human_doctor(value: &Value) -> String {
    let mut out = String::from("Android Use Doctor\n");
    if let Some(checks) = value.get("checks").and_then(Value::as_object) {
        for (name, check) in checks {
            let state = check.get("state").and_then(Value::as_str).unwrap_or("attention");
            let mark = match state {
                "ready" => "✓",
                "optional" => "○",
                "broken" => "✗",
                _ => "!",
            };
            let message = check.get("message").and_then(Value::as_str).unwrap_or("");
            out.push_str(&format!("\n{mark} {} — {message}", title(name)));
        }
    } else if value.get("error").is_some() {
        out.push_str("\n\n✗ Android device tools need attention.");
    }
    out.push_str(&human_next_step(value));
    out.push_str(if value.get("ready").and_then(Value::as_u64) == Some(1) { "\n\nOverall: Ready" } else { "\n\nOverall: Needs attention" });
    out
}

fn human_next_step(value: &Value) -> String {
    let Some(step) = value.get("next_step").and_then(Value::as_object) else {
        return value.get("next").and_then(Value::as_str).map(|next| format!("\n\nAction needed:\n{next}")).unwrap_or_default();
    };
    let mut out = String::from("\n\nNext step:");
    if let Some(title) = step.get("title").and_then(Value::as_str) {
        out.push_str(&format!("\n{title}"));
    }
    if let Some(steps) = step.get("steps").and_then(Value::as_array) {
        for (index, item) in steps.iter().enumerate() {
            if let Some(item) = item.as_str() {
                out.push_str(&format!("\n  {}. {item}", index + 1));
            }
        }
    }
    if let Some(resume) = step.get("resume").and_then(Value::as_str) {
        out.push_str(&format!("\nResume: {resume}"));
    }
    out
}

fn title(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

fn human_error(error: &Error) -> String {
    let detail = match error.code {
        Code::Device => "Android device tools or the USB connection needs attention.",
        Code::Identity => "Android Use cannot find the enrolled device. Run au doctor.",
        Code::Permission => "Android Use needs permission on the Android device.",
        Code::Timeout => "The Android device took too long to respond. Run au doctor and try again.",
        Code::Ambiguous => "More than one Android device is connected. Connect one device or use au enroll.",
        Code::Args => "That command needs a small correction. Run au --help for examples.",
        _ => "Android Use could not finish that request. Run au doctor for details.",
    };
    format!("{detail}\n\nDetails: {}", error.message)
}
