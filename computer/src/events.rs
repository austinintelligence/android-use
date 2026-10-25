//! Waiting for what happens next without the agent polling: a new notification, and a sign-in
//! code read out of one. Also the screen audit, which lists what makes a screen hard to use.
use crate::{
    api::{normalized, Code, Error, Result},
    format::SemanticRow,
};
use serde_json::Value;
use std::collections::HashMap;

pub(crate) const MAX_WAIT: u64 = 120;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Wait {
    pub from: Option<String>,
    pub seconds: u64,
}

/// `wait for notification [from "APP"] up to N seconds`; `None` when the text is another command.
pub(crate) fn parse_wait(command: &str) -> Option<Result<Wait>> {
    let rest = command.trim().strip_prefix("wait for notification")?;
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    let usage = || Error::new(Code::Args, r#"use: wait for notification [from "APP"] up to N seconds"#);
    let rest = rest.trim();
    let (from, rest) = match rest.strip_prefix("from ") {
        Some(after) => {
            let Some(body) = after.trim_start().strip_prefix('"') else { return Some(Err(usage())) };
            let Some(end) = body.find('"') else { return Some(Err(usage())) };
            (Some(body[..end].to_owned()), body[end + 1..].trim())
        }
        None => (None, rest),
    };
    let words: Vec<&str> = rest.split_whitespace().collect();
    Some(match words.as_slice() {
        ["up", "to", n, "seconds" | "second"] => {
            n.parse::<u64>().ok().filter(|n| (1..=MAX_WAIT).contains(n)).map(|seconds| Wait { from, seconds }).ok_or_else(|| Error::new(Code::Bounds, "wait 1 to 120 seconds"))
        }
        _ => Err(usage()),
    })
}

/// A notification as the helper lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    pub id: String,
    pub time: i64,
    pub package: String,
    pub title: String,
    pub text: String,
}

pub(crate) fn items(value: &Value) -> Vec<Item> {
    let field = |item: &Value, name: &str| item.get(name).and_then(Value::as_str).unwrap_or("").to_owned();
    value
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| Item {
            id: field(item, "id"),
            time: item.get("time").and_then(Value::as_i64).unwrap_or(0),
            package: field(item, "package"),
            title: field(item, "title"),
            text: field(item, "text"),
        })
        .collect()
}

/// Whether a notification is from the app the agent named: its package, a whole dotted segment
/// of the package (`messages` for `com.google.android.apps.messaging` is not guessed), or its title.
pub(crate) fn from_app(item: &Item, wanted: &str) -> bool {
    let wanted = normalized(wanted);
    wanted == item.package.to_lowercase() || normalized(&item.title) == wanted || item.package.to_lowercase().split('.').any(|segment| segment == wanted)
}

/// Polls `read` until a notification that was not there at the start (or was posted again) matches.
/// `sleep` is called between polls, `seconds` times at most.
pub(crate) fn wait_for(wait: &Wait, mut read: impl FnMut() -> Result<Vec<Item>>, mut sleep: impl FnMut()) -> Result<Option<Item>> {
    let baseline: HashMap<String, i64> = read()?.into_iter().map(|item| (item.id, item.time)).collect();
    for _ in 0..wait.seconds {
        sleep();
        let fresh = read()?
            .into_iter()
            .filter(|item| baseline.get(&item.id) != Some(&item.time))
            .filter(|item| wait.from.as_ref().is_none_or(|from| from_app(item, from)))
            .max_by_key(|item| item.time);
        if fresh.is_some() {
            return Ok(fresh);
        }
    }
    Ok(None)
}

pub(crate) fn describe(item: &Item) -> String {
    let title = if item.title.is_empty() { String::new() } else { format!(" \"{}\"", item.title) };
    format!("Notification from {}:{title} {}", item.package, item.text).trim_end().to_owned()
}

/// The first run of 4 to 8 digits; two groups of at least 3 digits joined by one space or dash
/// (`123 456`) count as one, so a date like `2024-05` is not merged.
pub(crate) fn extract_code(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let group = |from: usize| chars[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut at = 0;
    while at < chars.len() {
        if !chars[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let first = group(at);
        let mut digits: String = chars[at..at + first].iter().collect();
        let mut end = at + first;
        if first >= 3 && matches!(chars.get(end), Some(' ' | '-')) {
            let second = group(end + 1);
            if second >= 3 && first + second <= 8 {
                digits.extend(&chars[end + 1..end + 1 + second]);
                end += 1 + second;
            }
        }
        if (4..=8).contains(&digits.len()) {
            return Some(digits);
        }
        at = end;
    }
    None
}

/// What is hard to use on the screen, from the rows the agent would read.
pub(crate) fn audit(rows: &[SemanticRow]) -> String {
    let controls = || rows.iter().filter(|row| !row.kind.is_empty() && row.kind != "heading");
    let nameless: Vec<String> = controls().filter(|row| row.label.trim().is_empty()).map(|row| row.handle.map_or("(no handle)".to_owned(), |h| format!("e{h}"))).collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in controls().filter(|row| !row.label.trim().is_empty()) {
        *counts.entry(normalized(&row.label)).or_default() += 1;
    }
    let mut dupes: Vec<String> = counts.into_iter().filter(|(_, n)| *n > 1).map(|(label, n)| format!("\"{label}\" x{n}")).collect();
    dupes.sort_unstable();
    let disabled = controls().filter(|row| !row.enabled).count();
    let mut lines = Vec::new();
    if !nameless.is_empty() {
        lines.push(format!("{} control(s) have no accessible name: {}. Add a content description.", nameless.len(), nameless.join(", ")));
    }
    if !dupes.is_empty() {
        lines.push(format!("Labels used by more than one control: {}. Give each a distinct name.", dupes.join(", ")));
    }
    if disabled > 0 {
        lines.push(format!("{disabled} control(s) are disabled."));
    }
    lines.push("Not measured: touch-target size and cut-off text.".into());
    if lines.len() == 1 {
        lines.insert(0, format!("No problems found among {} control(s).", controls().count()));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str, time: i64, package: &str, title: &str, text: &str) -> Item {
        Item { id: id.into(), time, package: package.into(), title: title.into(), text: text.into() }
    }

    #[test]
    fn wait_commands_parse_with_and_without_an_app() {
        assert_eq!(parse_wait(r#"wait for notification up to 30 seconds"#).unwrap().unwrap(), Wait { from: None, seconds: 30 });
        assert_eq!(parse_wait(r#"wait for notification from "Bank" up to 1 second"#).unwrap().unwrap(), Wait { from: Some("Bank".into()), seconds: 1 });
        assert!(parse_wait(r#"wait for text "x" up to 5 seconds"#).is_none() && parse_wait("tap \"x\"").is_none());
        for bad in [
            "wait for notification",
            r#"wait for notification from Bank up to 5 seconds"#,
            "wait for notification up to 0 seconds",
            "wait for notification up to 500 seconds",
            "wait for notification up to x seconds",
        ] {
            assert!(parse_wait(bad).unwrap().is_err(), "{bad}");
        }
    }

    #[test]
    fn only_a_new_or_reposted_matching_notification_ends_the_wait() {
        let old = item("a", 1, "com.bank", "Bank", "old 111111");
        let polls = std::cell::RefCell::new(vec![
            vec![old.clone()],
            vec![old.clone()],
            vec![old.clone(), item("b", 5, "com.chat", "Chat", "hi")],
            vec![old.clone(), item("b", 5, "com.chat", "Chat", "hi"), item("c", 9, "com.bank", "Bank", "Your code is 482915")],
        ]);
        let mut slept = 0;
        let wait = Wait { from: Some("bank".into()), seconds: 10 };
        let got = wait_for(&wait, || Ok(polls.borrow_mut().remove(0)), || slept += 1).unwrap().unwrap();
        assert_eq!((got.id.as_str(), slept), ("c", 3), "a non-matching new item does not end the wait");
        // The same key posted again later counts as new.
        let again = std::cell::RefCell::new(vec![vec![old.clone()], vec![item("a", 2, "com.bank", "Bank", "again")]]);
        assert_eq!(wait_for(&Wait { from: None, seconds: 5 }, || Ok(again.borrow_mut().remove(0)), || {}).unwrap().unwrap().text, "again");
        // Nothing arrives: the wait ends after its window with no item.
        let mut polls = 0;
        assert!(wait_for(
            &Wait { from: None, seconds: 3 },
            || {
                polls += 1;
                Ok(vec![old.clone()])
            },
            || {}
        )
        .unwrap()
        .is_none());
        assert_eq!(polls, 4, "one baseline read and one per second");
    }

    #[test]
    fn apps_match_by_package_segment_or_title_not_by_substring() {
        let i = item("a", 1, "com.google.android.apps.messaging", "Alex", "lunch?");
        assert!(from_app(&i, "com.google.android.apps.messaging") && from_app(&i, "Messaging") && from_app(&i, "alex"));
        assert!(from_app(&i, "google") && !from_app(&i, "message"), "segments are whole words");
    }

    #[test]
    fn codes_are_the_first_four_to_eight_digits() {
        for (text, code) in [("Your code is 482915", "482915"), ("G-123 456 is your code", "123456"), ("Call 555 or enter 7788", "7788"), ("order 12 of 2024-05", "2024")] {
            assert_eq!(extract_code(text).as_deref(), Some(code), "{text}");
        }
        for text in ["no digits", "pin 123", "id 123456789012", ""] {
            assert_eq!(extract_code(text), None, "{text:?}");
        }
    }

    #[test]
    fn items_read_the_helper_shape() {
        let parsed = items(&json!({"enabled":true,"items":[{"id":"k","time":7,"package":"com.x","title":"T","text":"B","actions":[]}]}));
        assert_eq!(parsed, [item("k", 7, "com.x", "T", "B")]);
        assert_eq!(describe(&parsed[0]), "Notification from com.x: \"T\" B");
    }

    #[test]
    fn the_audit_names_nameless_duplicate_and_disabled_controls() {
        let row = |label: &str, kind: &str, handle: Option<u16>, enabled: bool| SemanticRow {
            label: label.into(),
            value: String::new(),
            kind: kind.into(),
            state: String::new(),
            enabled,
            selected: false,
            handle,
        };
        let rows = vec![
            row("Details", "button", None, true),
            row("Details", "button", None, true),
            row("", "button", Some(4), true),
            row("Save", "button", None, false),
            row("Title", "heading", None, true),
        ];
        let out = audit(&rows);
        assert!(
            out.contains("1 control(s) have no accessible name: e4") && out.contains("\"details\" x2") && out.contains("1 control(s) are disabled") && out.contains("Not measured"),
            "{out}"
        );
        assert!(audit(&[row("OK", "button", None, true)]).starts_with("No problems found among 1 control(s)."));
    }
}
