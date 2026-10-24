//! The one place a command text enters the engine, for `au serve --mcp` and for `au act` and
//! `au read`: pause and stop from `au watch`, routines, policy, then the normal parser and engine.
//! Everything here only restricts or repeats; the engine still checks every plan itself.
use crate::{
    api::{parse_act_command, parse_read_command, Code, Error, Result},
    ask,
    command::{Action, AndroidAction, BrowserAction},
    engine::{Engine, ModelResponse},
    events, files,
    policy::Policy,
    resolve, routine,
    secret::{self, OsStore, SecretStore},
    trace::Trace,
};
use serde_json::Value;
use std::{fs, path::Path};

/// Per-process session state, opened with the engine.
pub struct Session {
    pub(crate) policy: Policy,
    /// Steps already used against `actions.max_per_session`.
    used: u32,
    /// Opened by the first command, so a session that runs none leaves no file.
    trace: std::cell::OnceCell<Option<Trace>>,
    pub(crate) secrets: Box<dyn SecretStore>,
}

impl Session {
    pub(crate) fn open(root: &Path) -> Result<Self> {
        let policy = Policy::load(root)?;
        resolve::set_extra_risky(&policy.risky_extra);
        Ok(Self { policy, used: 0, trace: std::cell::OnceCell::new(), secrets: Box::new(OsStore::at(root)) })
    }
}

/// What `au watch` asks of a running session, read from `control.json` before every command.
fn control(root: &Path) -> (bool, bool) {
    let value: Value = fs::read_to_string(root.join("control.json")).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);
    (value["paused"] == true, value["stop"] == true)
}

fn check_control(root: &Path, acts: bool) -> Result<()> {
    match control(root) {
        (_, true) => Err(Error::plain(Code::Stopped, "A person pressed Stop in au watch. Do not continue; ask the user.")),
        (true, _) if acts => Err(Error::plain(Code::Paused, "The session is paused from au watch. Nothing was run; wait, then resend.")),
        _ => Ok(()),
    }
}

/// Whether a step taps a control that answers a prompt (Allow, OK, Accept, ...).
fn answers_prompt(action: &Action) -> bool {
    matches!(action, Action::Android(AndroidAction::Tap(target) | AndroidAction::Hold(target)) if resolve::confirms(&target.label))
}

fn media(action: &Action) -> bool {
    matches!(
        action,
        Action::Android(AndroidAction::CaptureScreen | AndroidAction::Camera { .. } | AndroidAction::Microphone(_) | AndroidAction::ScreenRecord(_))
            | Action::Browser(BrowserAction::Screenshot)
    )
}

/// Policy checks that need only the parsed command; app names are checked where they resolve.
fn check_policy(engine: &mut Engine, actions: &[Action]) -> Result<()> {
    if actions.iter().any(media) {
        engine.guard.policy.capture_ok()?;
    }
    // Android's own permission and install prompts are answered by a person, never by the agent.
    if actions.iter().any(answers_prompt) && engine.system_prompt_open() {
        return Err(Error::plain(Code::Blocked, "Android is showing its own permission or install prompt. Only a person answers it; ask the user. Nothing was run."));
    }
    if let Some(max) = engine.guard.policy.max_actions {
        let steps = actions.len() as u32;
        if engine.guard.used + steps > max {
            return Err(Error::plain(Code::Blocked, format!("Blocked by policy.toml: actions.max_per_session is {max} and {} are used. Nothing was run.", engine.guard.used)));
        }
        engine.guard.used += steps;
    }
    Ok(())
}

/// Runs one tool call's command text.
pub fn call(engine: &mut Engine, read: bool, command: &str, identity: Option<&str>) -> Result<ModelResponse> {
    check_control(engine.root(), !read)?;
    let result = if read {
        read_command(engine, command)
    } else {
        if let Some(meta) = routine::parse(command) {
            meta.and_then(|meta| run_routine(engine, meta, identity))
        } else if let Some(wait) = events::parse_wait(command) {
            wait.and_then(|wait| wait_notification(engine, &wait))
        } else if let Some(transfer) = files::parse(command) {
            transfer.and_then(|transfer| files::run(engine, &transfer)).map(|text| ModelResponse { text, status: "done", ..Default::default() })
        } else if let Some(question) = ask::parse(command) {
            question.and_then(|question| ask_phone(engine, &question, identity))
        } else {
            act(engine, command, identity)
        }
    };
    let root = engine.root().to_path_buf();
    if let Some(trace) = engine.guard.trace.get_or_init(|| Trace::open(&root, &std::process::id().to_string())) {
        match &result {
            Ok(r) => trace.record(read, command, &Ok((if r.status.is_empty() { "ok" } else { r.status }, &r.text, r.image.is_some()))),
            Err(e) => trace.record(read, command, &Err((e.code.wire(), &crate::engine::plain_error(e)))),
        }
    }
    result
}

fn read_command(engine: &mut Engine, command: &str) -> Result<ModelResponse> {
    let text = match command.trim().to_ascii_lowercase().as_str() {
        "routines" => routine::list(engine.root()),
        "policy" => engine.guard.policy.describe(),
        "secrets" => secret::list(engine.root()),
        "audit" => events::audit(&engine.semantic_rows("")?),
        lower if lower.starts_with("code from notification ") => return read_code(engine, command.trim()["code from notification ".len()..].trim()),
        _ => return engine.model_read(parse_read_command(command)?),
    };
    Ok(ModelResponse { text, ..Default::default() })
}

/// A person's yes on the phone, then the steps it covers.
fn ask_phone(engine: &mut Engine, question: &ask::Ask, identity: Option<&str>) -> Result<ModelResponse> {
    ask::ask(engine, question)?;
    if question.then.is_empty() {
        return Ok(ModelResponse { text: "Allowed on the phone.".into(), status: "done", ..Default::default() });
    }
    act(engine, &question.then, identity)
}

fn notification_items(engine: &mut Engine) -> Result<Vec<events::Item>> {
    engine.read(crate::api::Read::Notifications).map(|value| events::items(&value))
}

fn wait_notification(engine: &mut Engine, wait: &events::Wait) -> Result<ModelResponse> {
    let found = events::wait_for(wait, || notification_items(engine), || std::thread::sleep(std::time::Duration::from_secs(1)))?;
    Ok(match found {
        Some(item) => ModelResponse { text: events::describe(&item), status: "done", ..Default::default() },
        None => ModelResponse {
            text: format!("No {}notification arrived within {} seconds.", wait.from.as_ref().map_or(String::new(), |from| format!("\"{from}\" ")), wait.seconds),
            status: "not_found",
            ..Default::default()
        },
    })
}

/// `read code from notification "APP OR TITLE"`: the newest matching notification's code, only for
/// apps `codes.apps` lists.
fn read_code(engine: &mut Engine, wanted: &str) -> Result<ModelResponse> {
    let wanted = wanted.trim().trim_matches('"');
    let mut items: Vec<events::Item> = notification_items(engine)?.into_iter().filter(|item| events::from_app(item, wanted)).collect();
    items.sort_by_key(|item| item.time);
    let item = items.pop().ok_or_else(|| Error::plain(Code::Missing, format!("No notification from \"{wanted}\".")))?;
    if !engine.guard.policy.code_ok(&item.package, &item.title) {
        return Err(Error::plain(Code::Blocked, format!("Blocked by policy.toml: codes.apps does not list {}.", item.package)));
    }
    let code = events::extract_code(&item.text)
        .or_else(|| events::extract_code(&item.title))
        .ok_or_else(|| Error::plain(Code::Missing, "The newest notification has no 4 to 8 digit code."))?;
    Ok(ModelResponse { text: format!("Code: {code} (from {})", item.package), ..Default::default() })
}

/// Runs an act command and remembers it when it ran to the end (a clean miss is not success).
fn act(engine: &mut Engine, command: &str, identity: Option<&str>) -> Result<ModelResponse> {
    let secret = match secret::find(command)? {
        Some(step) => {
            let value = secret_value(engine, &step)?;
            Some((step.name.clone(), secret::substitute(command, &step, &value), value))
        }
        None => None,
    };
    let (run, hidden) = match &secret {
        Some((name, run, value)) => (run.as_str(), Some((name.as_str(), value.as_str()))),
        None => (command, None),
    };
    let scrub_text = |text: String| hidden.map_or(text.clone(), |(name, value)| secret::scrub(&text, name, value));
    let result = parse_act_command(run).and_then(|actions| {
        check_policy(engine, &actions)?;
        engine.model_act(&actions, identity)
    });
    let response = match result {
        Ok(mut response) => {
            response.text = scrub_text(response.text);
            response
        }
        Err(mut error) => {
            error.message = scrub_text(error.message);
            return Err(error);
        }
    };
    if !response.failed && response.status == "done" {
        engine.last_act = Some(command.to_owned());
    }
    Ok(response)
}

/// The value for a `type secret` step, once the target is known to be a password field of an
/// app the secret was saved for. Nothing is typed otherwise.
fn secret_value(engine: &mut Engine, step: &secret::Step) -> Result<String> {
    let apps =
        secret::allowed_apps(engine.root(), &step.name).ok_or_else(|| Error::plain(Code::Missing, format!("No secret named \"{}\"; `read secrets` lists them.", step.name)))?;
    let package = engine.ensure_scene()?.package;
    if !apps.iter().any(|app| app.as_str() == &*package) {
        return Err(Error::plain(Code::Blocked, format!("Secret \"{}\" is not allowed in {package}. Nothing was typed.", step.name)));
    }
    let field = crate::api::normalized(&step.field);
    let handle = field.strip_prefix('e').and_then(|n| n.parse::<u16>().ok());
    let rows = engine.semantic_rows("")?;
    let mut hits = rows.iter().filter(|row| if handle.is_some() { row.handle == handle } else { crate::api::normalized(&row.label) == field });
    let password = match (hits.next(), hits.next()) {
        (Some(row), None) => row.kind == "text field" && row.value.contains("password"),
        _ => false,
    };
    if !password {
        return Err(Error::plain(Code::Blocked, format!("\"{}\" is not a single password field on this screen, so secret \"{}\" was not typed.", step.field, step.name)));
    }
    engine
        .guard
        .secrets
        .get(&crate::api::normalized(&step.name))?
        .ok_or_else(|| Error::plain(Code::Missing, format!("Secret \"{}\" has no value in the system store; add it again with `au secret add`.", step.name)))
}

fn run_routine(engine: &mut Engine, meta: routine::Meta, identity: Option<&str>) -> Result<ModelResponse> {
    match meta {
        routine::Meta::Save { name, params } => {
            let last = engine.last_act.clone().ok_or_else(|| Error::new(Code::Args, "nothing to save yet: run a command that works first"))?;
            Ok(ModelResponse { text: routine::save(engine.root(), &name, &last, &params)?, ..Default::default() })
        }
        routine::Meta::Run { name, args } => {
            let command = routine::expand(&name, &routine::load(engine.root(), &name)?, &args)?;
            act(engine, &command, identity)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_reads_pause_and_stop_and_ignores_garbage() {
        let d = tempfile::tempdir().unwrap();
        assert!(check_control(d.path(), true).is_ok());
        fs::write(d.path().join("control.json"), "not json").unwrap();
        assert!(check_control(d.path(), true).is_ok());
        fs::write(d.path().join("control.json"), r#"{"paused":true}"#).unwrap();
        assert_eq!(check_control(d.path(), true).unwrap_err().code, Code::Paused);
        assert!(check_control(d.path(), false).is_ok(), "a pause stops changes, not reads");
        fs::write(d.path().join("control.json"), r#"{"paused":true,"stop":true}"#).unwrap();
        assert_eq!((check_control(d.path(), true).unwrap_err().code, check_control(d.path(), false).unwrap_err().code), (Code::Stopped, Code::Stopped));
    }
}
