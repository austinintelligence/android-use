#![forbid(unsafe_code)]

use au::{
    adapter,
    api::{parse_act_command, parse_read_command, Code, Error, Read, Result},
    clients::{self, Client, Dirs, Options},
    emulator,
    engine::{plain_error, Engine, ModelResponse},
    install,
};
use serde_json::Value;
use std::{env, io::IsTerminal, path::Path};

fn main() {
    select_target();
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
/// `--target NAME` anywhere on the command line selects a target for this process: it is moved to
/// `AU_TARGET`, which `Paths::discover` reads, so no command parses it.
fn select_target() {
    let args: Vec<String> = env::args().collect();
    if let Some(at) = args.iter().position(|arg| arg == "--target") {
        if let Some(name) = args.get(at + 1) {
            env::set_var("AU_TARGET", name);
        }
    }
}

fn run(human: bool) -> Result<Value> {
    let mut a: Vec<String> = env::args().skip(1).filter(|arg| arg != "--json" && arg != "--human").collect();
    if let Some(at) = a.iter().position(|arg| arg == "--target") {
        if a.get(at + 1).is_none_or(|name| name.starts_with("--")) {
            return Err(Error::new(Code::Args, "--target needs a target name; see au targets"));
        }
        a.drain(at..at + 2);
    }
    let out = match a.first().map(String::as_str).unwrap_or("help") {
        "emulator" => emulator(&a)?,
        "targets" => targets()?,
        "serve" => {
            let paths = au::device::Paths::discover()?;
            let _lock = paths.lock_for_serving()?;
            let e = Engine::open()?;
            match a.get(1).map(String::as_str) {
                Some("--mcp") => return adapter::mcp(e).map(|_| Value::Null),
                _ => return Err(Error::new(Code::Args, "serve requires --mcp")),
            }
        }
        "status" => Engine::open()?.read(Read::Status)?,
        "devices" => install::devices()?,
        "read" => {
            let mut engine = Engine::open()?;
            engine.load_image_space();
            let response = parse_read_command(&a[1..].join(" ")).and_then(|command| engine.model_read(command)).map_err(|e| Error::plain(e.code, plain_error(&e)))?;
            engine.save_image_space();
            model_value(&engine, response)?
        }
        "act" => {
            let mut engine = Engine::open()?;
            engine.load_image_space();
            let response = parse_act_command(&a[1..].join(" ")).and_then(|actions| engine.model_act(&actions, None));
            engine.save_image_space(); // also on failure: a failed command may still have changed the screen
            let response = response.map_err(|e| Error::plain(e.code, plain_error(&e)))?;
            model_value(&engine, response)?
        }
        "enroll" => install::enroll(a.get(1).ok_or_else(|| Error::new(Code::Args, "enroll requires an ADB endpoint"))?)?,
        "install" | "uninstall" if a.get(1).is_some_and(|name| !name.starts_with("--")) || a[0] == "install" => agents(&a)?,
        "setup" | "repair" => {
            let mut value = install::setup(a.get(1).filter(|arg| !arg.starts_with("--")).map(Path::new), human)?;
            if value["ready"] == 1 && !a.iter().any(|arg| arg == "--no-agents") {
                // One command from a fresh machine to a working agent: register with every
                // agent found, then prove it with a real read.
                value["agents"] = clients::detect_and_install(&Dirs::from_env()?, Options::default());
                value["check"] = check_device();
            }
            value
        }
        "update" => install::update(a.get(1).map(Path::new), human)?,
        "doctor" => install::doctor()?,
        "uninstall" => install::uninstall()?,
        "version" | "--version" => serde_json::json!({"version":env!("CARGO_PKG_VERSION")}),
        "help" | "--help" | "-h" => {
            serde_json::json!({"commands":["devices","setup [APK]","status","doctor","update [APK]","uninstall","enroll ENDPOINT","repair [APK]","serve --mcp","read COMMAND","act COMMAND","install [AGENT|all] [--dry-run] [--force]","uninstall AGENT","emulator doctor|install|images|create|start|stop|status|list|snapshot|destroy","targets","--target NAME (any command)"]})
        }
        _ => return Err(Error::new(Code::Args, format!("unknown command: {}", a[0]))),
    };
    Ok(out)
}
/// `au emulator doctor|install|images|create|start|stop|status|list|snapshot|destroy`.
fn emulator(a: &[String]) -> Result<Value> {
    let name = || a.get(2).filter(|arg| !arg.starts_with("--")).ok_or_else(|| Error::new(Code::Args, "this command needs an emulator NAME"));
    let flag = |flag: &str| a.iter().any(|arg| arg == flag);
    let value = |flag: &str| a.iter().position(|arg| arg == flag).and_then(|at| a.get(at + 1)).map(String::as_str);
    match a.get(1).map(String::as_str) {
        Some("doctor") => emulator::doctor(),
        Some("install") => emulator::install(),
        Some("images") => emulator::list_images(),
        Some("create") => emulator::create(name()?, value("--image"), if value("--profile") == Some("standard") { emulator::Profile::Standard } else { emulator::Profile::Small }),
        Some("start") => emulator::start(name()?, flag("--window")),
        Some("stop") => emulator::stop(name()?),
        Some("status") => emulator::status(name()?),
        Some("list") => emulator::list(),
        Some("destroy") => emulator::destroy(name()?, flag("--yes")),
        Some("snapshot") => emulator::snapshot(name()?, a.get(3).map_or("", String::as_str), a.get(4).map_or("", String::as_str)),
        _ => Err(Error::new(
            Code::Args,
            "use au emulator doctor, install, images, create NAME, start NAME, stop NAME, status NAME, list, snapshot NAME save|load TAG, or destroy NAME --yes",
        )),
    }
}

/// `au targets`: the default target and every named one, with the endpoint each is bound to.
fn targets() -> Result<Value> {
    let base = au::device::Paths::base()?;
    let bound = |dir: &Path| {
        std::fs::read(dir.join("device.json")).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()).and_then(|v| v["endpoint"].as_str().map(str::to_owned))
    };
    let mut rows = vec![serde_json::json!({"name":"default","kind":"device","endpoint":bound(&base)})];
    let mut names: Vec<_> = std::fs::read_dir(base.join("targets")).map(|dir| dir.flatten().filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
    names.sort();
    for name in names {
        let dir = base.join("targets").join(&name);
        let kind = std::fs::read(dir.join("target.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|v| v["kind"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "device".into());
        rows.push(serde_json::json!({"name":name,"kind":kind,"endpoint":bound(&dir)}));
    }
    Ok(serde_json::json!({"ok":1,"targets":rows}))
}

/// `au install [AGENT|all] [--dry-run] [--force]` and `au uninstall AGENT`: register or remove the
/// MCP server (and skill) for the named agents, or for every agent found.
fn agents(a: &[String]) -> Result<Value> {
    let (dirs, options) = (Dirs::from_env()?, Options { dry_run: a.iter().any(|arg| arg == "--dry-run"), force: a.iter().any(|arg| arg == "--force") });
    let named: Vec<&String> = a[1..].iter().filter(|arg| !arg.starts_with("--")).collect();
    let targets: Vec<Client> = if named.is_empty() || named.iter().any(|name| name.as_str() == "all") {
        clients::ALL.into_iter().filter(|client| client.present(&dirs)).collect()
    } else {
        named
            .iter()
            .map(|name| {
                Client::parse(name)
                    .ok_or_else(|| Error::new(Code::Args, format!("unknown agent {name}; use claude, codex, cursor, gemini, vscode, claude-desktop, opencode, or all")))
            })
            .collect::<Result<_>>()?
    };
    let results: Vec<Value> = targets
        .into_iter()
        .map(|client| clients::report(if a[0] == "install" { clients::install(client, &dirs, options) } else { clients::uninstall(client, &dirs, options) }, client))
        .collect();
    let failed = results.iter().any(|result| result["action"] == "error");
    Ok(serde_json::json!({"ok": u8::from(!failed), "agents": results, "found": results.len()}))
}

/// A live read after setup: the status line and the first lines of the screen.
fn check_device() -> Value {
    let mut engine = match Engine::open() {
        Ok(engine) => engine,
        Err(error) => return Value::String(error.message),
    };
    let status = engine.model_read(parse_read_command("status").unwrap_or(au::api::CommandRead::Status)).map(|r| r.text);
    let screen = engine.model_read(au::api::CommandRead::Screen { full: false, matching: None, delta: false }).map(|r| r.text.lines().take(6).collect::<Vec<_>>().join("\n"));
    Value::String(format!("{}\n{}", status.unwrap_or_else(|e| e.message), screen.unwrap_or_else(|e| e.message)))
}

/// Plain-command results for the CLI. An attached image is saved to the
/// private artifact folder and its path is returned instead of inline bytes.
fn model_value(engine: &Engine, response: ModelResponse) -> Result<Value> {
    let mut value = serde_json::json!({"ok":1,"text":response.text});
    if let Some(image) = response.image {
        value["image"] = serde_json::json!(engine.export(&image.bytes)?);
    }
    Ok(value)
}
fn human_value(command: &str, value: &Value) -> String {
    match command {
        "help" | "--help" | "-h" => "Android Use — Give AI an Android device.\n\nGet connected:\n  au devices      List connected Android devices\n  au setup        Prepare and remember one device\n  au status       Check whether Android Use is ready\n  au doctor       Explain anything that needs attention\n\nUse the device (same commands as the MCP tools):\n  au read screen                 List what is on screen\n  au read 'find \"Wi-Fi\"'          Look for one item\n  au read page                   Read the current Chrome page\n  au act 'tap \"Settings\"'         Tap a visible label\n  au act 'open app \"Chrome\" then page open \"https://example.com\"'\n\nConnect an agent:\n  au install [AGENT]     Register with Claude Code, Codex, Cursor, ... (au install --dry-run first)\n  au serve --mcp         Model Context Protocol over stdio\n\nTry it on an emulator instead of your phone:\n  au emulator doctor     Check the Android SDK (au emulator install fetches what is missing)\n  au targets             List targets; add --target NAME to any command\n\nMaintenance:\n  au update       Update the Android helper\n  au uninstall    Remove Android Use and its local state\n\nRun `au help --json` for the complete machine-readable command list.".into(),
        "version" | "--version" => format!("Android Use {}", value.get("version").and_then(Value::as_str).unwrap_or(env!("CARGO_PKG_VERSION"))),
        "status" => match (value.get("ok").and_then(Value::as_u64) == Some(1), value.get("cap").and_then(Value::as_u64).unwrap_or(0) != 0) {
            (true, true) => "Android Use is ready\n\n✓ Android helper connected\n✓ Accessibility control is on".into(),
            (true, false) => "Android Use is almost ready\n\n✓ Android helper connected\n! Accessibility control is off\n\nOn Android, open Settings → Accessibility → Android-Use and turn it on.\nThen run: au status".into(),
            _ => "Android Use is not ready.\nRun: au doctor".into(),
        },
        "read" | "act" if value.get("text").is_some() => {
            let mut out = value["text"].as_str().unwrap_or_default().to_owned();
            if let Some(path) = value.get("image").and_then(Value::as_str) {
                out.push_str(&format!("\n\nImage saved to {path}"));
            }
            out
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
            if let Some(agents) = value.get("agents").and_then(Value::as_array) {
                for row in agents {
                    out.push_str(&format!("\n{} {}: {}", if row["action"] == "error" || row["action"] == "manual" { "!" } else { "✓" }, row["client"].as_str().unwrap_or(""), row["action"].as_str().unwrap_or("")));
                }
            }
            if let Some(check) = value.get("check").and_then(Value::as_str) {
                out.push_str(&format!("\n\nLive check:\n{check}"));
            }
            out
        }
        "doctor" => human_doctor(value),
        "install" | "uninstall" if value.get("agents").is_some() => {
            let rows = value["agents"].as_array().map(|rows| rows.iter().map(|row| format!("{} {}: {}{}", if row["action"] == "error" || row["action"] == "manual" { "!" } else { "✓" }, row["client"].as_str().unwrap_or(""), row["action"].as_str().unwrap_or(""), row["detail"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" ({d})")).unwrap_or_default())).collect::<Vec<_>>()).unwrap_or_default();
            if rows.is_empty() { "No supported agent found. Name one: au install claude|codex|cursor|gemini|vscode|claude-desktop|opencode".into() } else { rows.join("\n") }
        }
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
    if error.plain {
        return error.message.clone();
    }
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
