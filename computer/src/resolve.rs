//! Target resolution: turns a label the agent read (plus an optional
//! `number N`) into exactly one control, tab, app, or notification.
//!
//! Every surface uses the same rule so agents only learn it once: exact
//! (case- and whitespace-insensitive) matches win over substring matches,
//! `number N` picks among the winners in reading order, and more than one
//! winner without a number is reported as ambiguous instead of guessed.
use crate::{
    api::{normalized, Code, Error, Result, Scene, Target},
    format::display,
};
use serde_json::Value;

/// One resolvable thing and every label it can be called by.
pub(crate) struct Candidate<T> {
    pub value: T,
    pub labels: Vec<String>,
}

impl<T> Candidate<T> {
    pub fn new(value: T, labels: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self { value, labels: labels.into_iter().map(|label| normalized(label.as_ref())).filter(|label| !label.is_empty()).collect() }
    }
}

pub(crate) fn pick<T: Clone + PartialEq>(target: &Target, candidates: &[Candidate<T>], surface: &str) -> Result<T> {
    let needle = normalized(&target.label);
    if needle.is_empty() {
        return Err(missing(&target.label, surface));
    }
    let mut matches = matching(candidates, |label| *label == needle);
    if matches.is_empty() {
        matches = matching(candidates, |label| label.contains(&needle));
    }
    choose(target, matches, surface)
}

/// Applies the ordinal/ambiguity rule to an already-ordered match list.
pub(crate) fn choose<T: Clone>(target: &Target, matches: Vec<T>, surface: &str) -> Result<T> {
    if matches.is_empty() {
        return Err(missing(&target.label, surface));
    }
    match target.ordinal {
        Some(n) => matches.get(usize::from(n).saturating_sub(1)).cloned().ok_or_else(|| Error::plain(Code::Ambiguous, ambiguity(&target.label, matches.len()))),
        None if matches.len() > 1 => Err(Error::plain(Code::Ambiguous, ambiguity(&target.label, matches.len()))),
        None => Ok(matches[0].clone()),
    }
}

fn matching<T: Clone + PartialEq>(candidates: &[Candidate<T>], test: impl Fn(&String) -> bool) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for candidate in candidates.iter().filter(|candidate| candidate.labels.iter().any(&test)) {
        if !out.contains(&candidate.value) {
            out.push(candidate.value.clone());
        }
    }
    out
}

pub(crate) fn missing(label: &str, surface: &str) -> Error {
    Error::plain(Code::Missing, format!("No item labeled \"{}\" is on the current {surface}", display(label)))
}

pub(crate) fn ambiguity(label: &str, count: usize) -> String {
    let label = display(label);
    if count > 16 {
        return format!("\"{label}\" matches {count} items. Use a more specific label, or number 1 to 16 for the first 16 in reading order.");
    }
    format!("\"{label}\" matches {count} items. Add number 1 to {count} after the quoted label, in reading order, for example \"{label}\" number 2.")
}

/// Labels a raw accessibility node can be called by: its text, plus a
/// readable form of a resource id such as `com.example:id/submit_button`.
pub(crate) fn semantic_labels(value: &str) -> Vec<String> {
    let mut labels = vec![normalized(value)];
    if let Some((_, tail)) = value.split_once(":id/") {
        labels.push(normalized(&tail.replace(['_', '-'], " ")));
    }
    labels
}

/// Host-side fallback for helpers that cannot resolve targets themselves.
pub(crate) fn scene_target(scene: &Scene, target: &Target) -> Result<u16> {
    let candidates: Vec<_> = scene.nodes.iter().map(|node| Candidate::new(node.id, semantic_labels(&node.label))).collect();
    pick(target, &candidates, "screen")
}

/// Resolves a Chrome page control from a compact page observation.
pub(crate) fn page_target(observed: &Value, target: &Target) -> Result<u16> {
    let rows = observed.get("n").and_then(Value::as_array).ok_or_else(|| Error::new(Code::Protocol, "Chrome observation omitted its controls"))?;
    let candidates: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let row = row.as_array()?;
            Some(Candidate::new(u16::try_from(row.first()?.as_u64()?).ok()?, [row.get(1)?.as_str()?]))
        })
        .collect();
    pick(target, &candidates, "page")
}

/// Resolves a launcher app by display name from the helper's app list.
pub(crate) fn app(rows: &Value, name: &str) -> Result<String> {
    let rows = rows.as_array().ok_or_else(|| Error::new(Code::Protocol, "app discovery returned an invalid list"))?;
    let candidates: Vec<_> = rows
        .iter()
        .filter_map(|row| match row.as_array().map(Vec::as_slice) {
            Some([package, label]) => Some(Candidate::new(package.as_str()?.to_owned(), [label.as_str()?])),
            _ => None,
        })
        .collect();
    let target = Target { label: name.into(), ordinal: None, handle: None };
    pick(&target, &candidates, "device").map_err(|error| match error.code {
        Code::Missing => Error::plain(Code::Missing, format!("No installed app is named \"{}\". Use the name shown under its launcher icon.", display(name))),
        Code::Ambiguous => Error::plain(Code::Ambiguous, format!("\"{}\" matches more than one app. Use the app's full display name.", display(name))),
        _ => error,
    })
}

/// Notifications in the order agents see them: newest first.
pub(crate) fn notification_items(value: &Value) -> Vec<&Value> {
    value.get("items").and_then(Value::as_array).map(|items| items.iter().rev().collect()).unwrap_or_default()
}

/// Resolves a notification by its title (or text) to the helper's opaque key.
/// The label a notification is listed and targeted by: its title, else its text, else a placeholder.
pub(crate) fn notification_label(item: &Value) -> String {
    let field = |name: &str| item.get(name).and_then(Value::as_str).unwrap_or("").trim().to_owned();
    let label = [field("title"), field("text")].into_iter().find(|label| !label.is_empty());
    label.unwrap_or_else(|| "(no title or text)".into())
}

pub(crate) fn notification(value: &Value, target: &Target) -> Result<Box<str>> {
    if value.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Err(Error::plain(Code::Permission, crate::format::NOTIFICATION_ACCESS_OFF));
    }
    let candidates: Vec<_> = notification_items(value)
        .into_iter()
        .filter_map(|item| {
            let key = item.get("id").and_then(Value::as_str).unwrap_or("");
            (!key.is_empty()).then(|| Candidate::new(Box::<str>::from(key), [notification_label(item)]))
        })
        .collect();
    pick(target, &candidates, "notification list")
}

/// The label and number a screen row names: `sw Wi-Fi=on` is Wi-Fi, `Save#2` is Save number 2,
/// `edit Email="x"`, `Inbox: 3 new` and a trailing `[disabled]` or `[selected]` are dropped.
/// `None` when the text is not shaped like a row, so a plain label is never reinterpreted.
pub(crate) fn from_row(text: &str) -> Option<(String, Option<u16>)> {
    let mut row = text.trim();
    while let Some(rest) = [" [disabled]", " [selected]"].iter().find_map(|suffix| row.strip_suffix(suffix)) {
        row = rest;
    }
    for tag in ["sw ", "edit ", "chk ", "radio ", "tab ", "link ", "list ", "slider ", "btn "] {
        row = row.strip_prefix(tag).unwrap_or(row);
    }
    row = row.split_once(": ").map_or(row, |(name, _)| name);
    row = row.split_once("=\"").map_or(row, |(name, _)| name);
    row = row.strip_suffix("=on").or_else(|| row.strip_suffix("=off")).unwrap_or(row);
    let (name, number) = match row.rsplit_once('#') {
        Some((name, digits)) if (1..=2).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()) => (name, digits.parse().ok().filter(|n| (1..=16).contains(n))),
        _ => (row, None),
    };
    (!name.trim().is_empty() && (name != text || number.is_some())).then(|| (name.trim().to_owned(), number))
}

/// Words that make an extra label word unsafe to guess at: `tap "Save"` may tap "Save draft",
/// but never "Save and delete all".
const RISKY: [&str; 20] = [
    "delete",
    "remove",
    "erase",
    "reset",
    "pay",
    "buy",
    "send",
    "submit",
    "confirm",
    "all",
    "clear",
    "wipe",
    "sign",
    "log",
    "logout",
    "purchase",
    "order",
    "install",
    "uninstall",
    "factory",
];

/// Whether any word of a label is one that makes acting on a guess unsafe.
pub(crate) fn risky(label: &str) -> bool {
    normalized(label).split(' ').any(|word| RISKY.contains(&word))
}

/// Whether the only control whose label merely contains `wanted` is plainly what the agent meant:
/// the wanted words lead or trail the label, at most two other words come with them, and none of
/// those is destructive.
pub(crate) fn near_miss(wanted: &str, actual: &str) -> bool {
    let (wanted, actual) = (normalized(wanted), normalized(actual));
    let aligned = actual.starts_with(&format!("{wanted} ")) || actual.ends_with(&format!(" {wanted}"));
    let extra: Vec<&str> = actual.split(' ').filter(|word| !wanted.split(' ').any(|w| w == *word)).collect();
    aligned && extra.len() <= 2 && !extra.iter().any(|word| RISKY.contains(word))
}

/// The label a screen miss was about.
pub(crate) fn missed_label(error: &Error) -> Option<&str> {
    error.message.strip_prefix("No item labeled \"")?.split_once("\" is on the current screen").map(|(label, _)| label)
}

/// Up to three labels on screen close to a missing one (a shared word, a typo, a different
/// spelling), best first.
pub(crate) fn near_labels<'a>(wanted: &str, labels: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let squash = |text: &str| normalized(text).chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let (spoken, wanted) = (normalized(wanted), squash(wanted));
    let pairs = |text: &str| -> Vec<(char, char)> { text.chars().zip(text.chars().skip(1)).collect() };
    let closeness = |label: &str| {
        let (a, b) = (pairs(&wanted), pairs(label));
        let common = a.iter().filter(|pair| b.contains(pair)).count();
        if label.contains(&wanted) || wanted.contains(label) {
            0.9
        } else {
            (2 * common) as f32 / (a.len() + b.len()).max(1) as f32
        }
    };
    let mut scored: Vec<(f32, &str)> = Vec::new();
    for label in labels {
        let (key, folded) = (normalized(label), squash(label));
        if !folded.is_empty() && key != spoken && !scored.iter().any(|(_, seen)| normalized(seen) == key) && closeness(&folded) >= 0.4 {
            scored.push((closeness(&folded), label));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(3).map(|(_, label)| label).collect()
}

/// A label missing from the Android screen (not a page, app, or
/// notification): the case where showing the screen helps the agent most.
pub(crate) fn is_screen_miss(error: &Error) -> bool {
    error.code == Code::Missing && error.message.ends_with("on the current screen")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn target(label: &str, ordinal: Option<u16>) -> Target {
        Target { label: label.into(), ordinal, handle: None }
    }

    #[test]
    fn exact_matches_beat_substrings_and_ordinals_pick_in_order() {
        let candidates = [Candidate::new(1u16, ["Save draft"]), Candidate::new(2, ["Save"]), Candidate::new(3, ["save "])];
        assert_eq!(pick(&target("save", Some(2)), &candidates, "screen").unwrap(), 3);
        assert_eq!(pick(&target("draft", None), &candidates, "screen").unwrap(), 1);
        let error = pick(&target("Save", None), &candidates, "screen").unwrap_err();
        assert_eq!(error.code, Code::Ambiguous);
        assert!(error.message.contains("number 2"));
        assert_eq!(pick(&target("Delete", None), &candidates, "screen").unwrap_err().code, Code::Missing);
        assert_eq!(pick(&target("Save", Some(9)), &candidates, "screen").unwrap_err().code, Code::Ambiguous);
    }

    #[test]
    fn labels_match_as_displayed_and_broad_matches_say_how_to_narrow() {
        let candidates = [Candidate::new(1u16, ["Inbox [3]"])];
        assert_eq!(pick(&target("Inbox 3", None), &candidates, "screen").unwrap(), 1);
        assert_eq!(pick(&target("[]", None), &candidates, "screen").unwrap_err().code, Code::Missing);
        assert!(ambiguity("Photo", 40).contains("more specific label"));
    }

    #[test]
    fn near_misses_are_guessed_only_when_safe() {
        assert!(near_miss("Save", "Save draft"));
        assert!(near_miss("Wi-Fi", "Wi-Fi settings"));
        assert!(!near_miss("Save", "Save and delete all"), "destructive words are never guessed");
        assert!(!near_miss("Save", "Autosave"), "word boundaries");
        assert!(!near_miss("Save", "Save changes to this document and close"), "too many extra words");
        assert!(!near_miss("Open", "Open and send"), "send is risky");
    }

    #[test]
    fn near_labels_are_the_few_closest_and_never_the_label_itself() {
        let screen = ["Wi-Fi", "Wi-Fi settings", "Bluetooth", "Save draft", "Network & internet", "wifi calling"];
        assert_eq!(near_labels("WiFi", screen), vec!["Wi-Fi", "Wi-Fi settings", "wifi calling"]);
        assert_eq!(near_labels("Save", screen), vec!["Save draft"]);
        assert!(near_labels("Zebra", screen).is_empty());
        let miss = missing("Say \"hi\"", "screen");
        assert_eq!(missed_label(&miss), Some("Say \"hi\""));
    }

    #[test]
    fn rows_copied_from_the_screen_name_their_control() {
        assert_eq!(from_row("sw Wi-Fi=on"), Some(("Wi-Fi".into(), None)));
        assert_eq!(from_row("Save#2"), Some(("Save".into(), Some(2))));
        assert_eq!(from_row("edit Email=\"a@b.c\" [disabled]"), Some(("Email".into(), None)));
        assert_eq!(from_row("Network: T-Mobile"), Some(("Network".into(), None)));
        assert_eq!(from_row("Wi-Fi"), None, "a plain label stays itself");
    }

    #[test]
    fn duplicate_values_count_once() {
        let candidates = [Candidate::new(4u16, ["Wi-Fi"]), Candidate::new(4, ["Wi-Fi"])];
        assert_eq!(pick(&target("wi-fi", None), &candidates, "screen").unwrap(), 4);
    }

    #[test]
    fn notifications_resolve_by_their_listed_label() {
        let value = json!({"enabled":true,"active":true,"items":[
            {"id":"k-old","title":"Messages","text":"Lunch?"},
            {"id":"k-new","title":"Messages","text":"Running late"},
            {"id":"k-cal","title":"Calendar","text":"Standup in 5 minutes"},
            {"id":"k-bare","title":"","text":"Backup finished"},
            {"id":"k-empty","title":"","text":" "}
        ]});
        assert_eq!(notification(&value, &target("(no title or text)", None)).unwrap().as_ref(), "k-empty", "listed items are targetable");
        assert_eq!(notification(&value, &target("Calendar", None)).unwrap().as_ref(), "k-cal");
        assert_eq!(notification(&value, &target("Messages", Some(1))).unwrap().as_ref(), "k-new");
        assert_eq!(notification(&value, &target("Backup finished", None)).unwrap().as_ref(), "k-bare", "untitled items use their text");
        assert_eq!(notification(&value, &target("Messages", None)).unwrap_err().code, Code::Ambiguous);
        assert_eq!(notification(&value, &target("Standup", None)).unwrap_err().code, Code::Missing, "text of a titled item is not its label");
        assert_eq!(notification(&json!({"enabled":false,"items":[]}), &target("x", None)).unwrap_err().code, Code::Permission);
    }

    #[test]
    fn labels_match_across_unicode_spaces_and_truncation() {
        assert_eq!(normalized("Nom\u{a0}:"), "nom :");
        assert_eq!(normalized("Loading\u{2026}"), "loading");
        let candidates = [Candidate::new(1u16, ["Ouvrir\u{202f}:"])];
        assert_eq!(pick(&target("Ouvrir :", None), &candidates, "screen").unwrap(), 1);
        assert_eq!(pick(&target("Ouvrir\u{2026}", None), &candidates, "screen").unwrap(), 1);
    }

    #[test]
    fn apps_resolve_by_display_name() {
        let rows = json!([["com.google.android.apps.maps", "Maps"], ["com.example.mail", "Mail"], ["com.google.android.gm", "Gmail"]]);
        assert_eq!(app(&rows, "maps").unwrap(), "com.google.android.apps.maps");
        assert_eq!(app(&rows, "Mail").unwrap(), "com.example.mail");
        assert_eq!(app(&rows, "ail").unwrap_err().code, Code::Ambiguous);
        assert_eq!(app(&rows, "Camera").unwrap_err().code, Code::Missing);
    }
}
