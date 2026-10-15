//! Every example command in the specification and the docs parses to exactly what the 1.2.0
//! parser produced, recorded in `fixtures/command-corpus-1.2.0.json`. A command that used to be an
//! error may start to work (that is the point of lenient parsing); one that parsed must not change.
//! The fixture is the 1.2.0 baseline and is never re-recorded; a deliberate difference goes in `INTENDED`.
use au::api::{parse_act_command, parse_read_command};
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path};

const FIXTURE: &str = "tests/fixtures/command-corpus-1.2.0.json";
/// Commands whose result is intentionally different from 1.2.0. A command that used to be rejected
/// and now works is allowed on its own and is not listed.
const INTENDED: &[&str] = &[
    // 1.3.0 adds an optional action label; without one, a notification with several actions runs nothing.
    r#"run notification action "Messages""#,
];
const VERBS: &[&str] = &[
    "tap",
    "toggle",
    "type",
    "scroll",
    "press",
    "open",
    "wait",
    "verify",
    "page",
    "hold",
    "new",
    "select",
    "close",
    "capture",
    "take",
    "record",
    "swipe",
    "crop",
    "dismiss",
    "run",
    "notification",
    "notifications",
    "screen",
    "find",
    "status",
    "help",
    "browser",
    "location",
    "capabilities",
    "image",
];

fn collect(path: &Path, out: &mut Vec<std::path::PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        entries.iter().for_each(|entry| collect(entry, out));
    } else if matches!(path.extension().and_then(|e| e.to_str()), Some("md" | "txt")) {
        out.push(path.to_path_buf());
    }
}

/// Backtick spans, and the quoted argument of `au act '…'` / `au read '…'`, that start with a command verb.
fn commands() -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for name in ["docs", "skills", "README.md", "AGENTS.md", "llms.txt"] {
        collect(&root.join(name), &mut files);
    }
    let mut found: BTreeSet<String> = au::spec::FORMS.iter().flat_map(|form| form.examples.iter().map(|e| (*e).to_owned())).collect();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let mut spans: Vec<String> = text.split('`').skip(1).step_by(2).map(str::to_owned).collect();
        for marker in ["au act '", "au read '"] {
            spans.extend(text.match_indices(marker).filter_map(|(at, _)| text[at + marker.len()..].split_once('\'').map(|(command, _)| command.to_owned())));
        }
        found.extend(spans.into_iter().map(|span| span.trim().to_owned()).filter(|span| !span.contains('\n') && span.split(' ').next().is_some_and(|verb| VERBS.contains(&verb))));
    }
    found
}

fn outcome(command: &str) -> Value {
    let show = |result: Result<String, String>| result.unwrap_or_else(|error| format!("error: {error}"));
    json!({
        "act": show(parse_act_command(command).map(|a| format!("{a:?}")).map_err(|e| e.message.to_string())),
        "read": show(parse_read_command(command).map(|r| format!("{r:?}")).map_err(|e| e.message.to_string())),
    })
}

#[test]
fn every_documented_command_parses_as_it_did_in_1_2_0() {
    let commands = commands();
    assert!(commands.len() > 80, "the corpus found only {} commands", commands.len());
    let current: serde_json::Map<String, Value> = commands.iter().map(|command| (command.clone(), outcome(command))).collect();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let recorded: serde_json::Map<String, Value> = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (command, before) in &recorded {
        if INTENDED.contains(&command.as_str()) {
            continue;
        }
        // A command whose example was reworded out of the docs is no longer checked.
        let Some(now) = current.get(command) else { continue };
        for tool in ["act", "read"] {
            let was = before[tool].as_str().unwrap();
            // A form that was rejected may now work; one that worked must be identical.
            assert!(was.starts_with("error: ") || now[tool] == before[tool], "{tool} {command:?} changed:\n was {was}\n now {}", now[tool]);
        }
    }
}
