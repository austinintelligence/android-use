//! `ask phone "QUESTION" [then STEPS]`: a person answers on the phone, in a prompt the agent cannot
//! see or tap. The prompt also shows the exact steps that will run. An approval is good only for
//! the screen it was asked on: if the labels changed while the person decided, nothing runs.
use crate::{
    api::{normalized, Code, Error, Result},
    engine::Engine,
    format::SemanticRow,
};
use serde_json::{json, Value};
use std::{thread, time::Duration};

const DEFAULT_MS: u64 = 120_000;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Ask {
    pub question: String,
    /// The steps to run once allowed (empty: just the answer).
    pub then: String,
}

/// `None` when the text is another command.
pub(crate) fn parse(command: &str) -> Option<Result<Ask>> {
    let rest = command.trim().strip_prefix("ask phone ")?.trim_start();
    let usage = || Error::new(Code::Args, r#"use: ask phone "QUESTION" then STEPS"#);
    let Some(body) = rest.strip_prefix('"') else { return Some(Err(usage())) };
    let Some(end) = body.find('"') else { return Some(Err(usage())) };
    let (question, tail) = (&body[..end], body[end + 1..].trim());
    let then = match tail.strip_prefix("then ") {
        Some(steps) if !steps.trim().is_empty() => steps.trim().to_owned(),
        None if tail.is_empty() => String::new(),
        _ => return Some(Err(usage())),
    };
    if question.trim().is_empty() || question.chars().count() > 300 || question.chars().any(char::is_control) {
        return Some(Err(Error::new(Code::Bounds, "the question is 1 to 300 characters")));
    }
    Some(Ok(Ask { question: question.to_owned(), then }))
}

fn labels(rows: &[SemanticRow]) -> Vec<String> {
    rows.iter().filter(|row| !row.label.is_empty()).map(|row| normalized(&row.label)).collect()
}

/// Asks and waits. `Ok(())` only when a person allowed it and the screen is the one they were asked about.
pub(crate) fn ask(engine: &mut Engine, ask: &Ask) -> Result<()> {
    let before = labels(&engine.semantic_rows("")?);
    // What the phone shows is what will run; typed text in secret-looking fields is already hidden.
    let detail = if ask.then.is_empty() { "nothing; this is a question".to_owned() } else { crate::trace::redact(&ask.then) };
    let started = engine.bridge()?.query("approve", json!({"text": ask.question, "detail": detail, "ms": DEFAULT_MS}));
    match started {
        Ok(_) => {}
        Err(e) if e.code == Code::Unsupported => return Err(Error::plain(Code::Unsupported, "This helper cannot ask on the phone. Run `au update`, then resend.")),
        Err(e) if e.code == Code::Bounds => return Err(Error::plain(Code::Blocked, "Another approval is already waiting on the phone.")),
        Err(e) => return Err(e),
    }
    let give_up = std::time::Instant::now() + Duration::from_millis(DEFAULT_MS + 5_000);
    let answer = loop {
        thread::sleep(Duration::from_millis(500));
        let state: Value = engine.bridge()?.query("approval", Value::Null)?;
        match state["state"].as_str().unwrap_or("none") {
            "pending" if std::time::Instant::now() < give_up => continue,
            "pending" | "timeout" | "none" => break "timeout",
            other => break if other == "allow" { "allow" } else { "deny" },
        }
    };
    match answer {
        "allow" => {
            // The prompt covered the app; give the screen a moment to come back before comparing.
            thread::sleep(Duration::from_millis(700));
            if labels(&engine.semantic_rows("")?) == before {
                Ok(())
            } else {
                Err(Error::plain(Code::Denied, "The screen changed while the phone was asking, so the approval no longer applies. Nothing was run; read the screen and ask again."))
            }
        }
        "deny" => Err(Error::plain(Code::Denied, "The person denied it on the phone. Nothing was run; do not try another way to do the same thing.")),
        _ => Err(Error::plain(Code::Denied, "Nobody answered on the phone in time. Nothing was run.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_question_and_steps_parse_and_everything_else_is_left_alone() {
        assert_eq!(parse(r#"ask phone "Pay $25 to Alex?" then tap "Pay $25""#).unwrap().unwrap(), Ask { question: "Pay $25 to Alex?".into(), then: r#"tap "Pay $25""#.into() });
        assert_eq!(parse(r#"ask phone "Is this you?""#).unwrap().unwrap().then, "");
        assert!(parse(r#"tap "ask phone""#).is_none() && parse("ask phones").is_none());
        for bad in [r#"ask phone Pay?"#, r#"ask phone "unclosed"#, r#"ask phone "q" tap "x""#, r#"ask phone "q" then "#, r#"ask phone "" then tap "x""#] {
            assert!(parse(bad).unwrap().is_err(), "{bad}");
        }
        assert_eq!(parse(&format!(r#"ask phone "{}" then tap "x""#, "q".repeat(301))).unwrap().unwrap_err().code, Code::Bounds);
    }
}
