//! Plain-language rendering for agents. Everything here is a pure function
//! from device data to short, bounded text with no wire syntax in it.
use crate::api::{normalized, Action, AndroidAction, BrowserAction, Code, Error, Receipt, Result};
use serde_json::Value;
use std::collections::HashMap;

/// Byte budget for a normal `screen` read and for the screen summary that
/// follows an action. Roughly 300 tokens: enough for a typical app screen
/// so agents rarely need a second `screen full` round trip.
pub(crate) const SCREEN_BYTES: usize = 1200;
pub(crate) const FULL_SCREEN_BYTES: usize = 4000;
pub(crate) const NOTIFICATION_ACCESS_OFF: &str =
    "Notification access is off for Android-Use. Ask the user to allow it on the device (Settings → Notifications → Device & app notifications → Android-Use), then retry.";

/// One conceptual row compiled by the helper's semantic compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemanticRow {
    pub label: String,
    pub value: String,
    pub kind: String,
    pub state: String,
    pub enabled: bool,
    pub selected: bool,
    /// Position in the helper's full listing: what `e12` means. Only unlabeled rows print it.
    pub handle: Option<u16>,
}

pub(crate) fn parse_semantic_rows(value: &Value) -> Result<Vec<SemanticRow>> {
    let rows = value.as_array().ok_or_else(|| Error::new(Code::Protocol, "semantic screen response must be an array"))?;
    if rows.len() > 256 {
        return Err(Error::new(Code::Bounds, "semantic screen response exceeds 256 rows"));
    }
    rows.iter()
        .map(|row| {
            let row =
                row.as_array().filter(|row| (6..=7).contains(&row.len())).ok_or_else(|| Error::new(Code::Protocol, "semantic screen row must contain six or seven values"))?;
            let text = |index: usize| match row[index].as_str() {
                Some(value) if value.len() <= 512 => Ok(value.to_owned()),
                Some(_) => Err(Error::new(Code::Bounds, "semantic screen text is too long")),
                None => Err(Error::new(Code::Protocol, "semantic screen text must be a string")),
            };
            let flag = |index: usize| row[index].as_bool().ok_or_else(|| Error::new(Code::Protocol, "semantic screen flag must be boolean"));
            Ok(SemanticRow {
                label: text(0)?,
                value: text(1)?,
                kind: text(2)?,
                state: text(3)?,
                enabled: flag(4)?,
                selected: flag(5)?,
                handle: row.get(6).and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()),
            })
        })
        .collect()
}

pub(crate) fn display(value: &str) -> String {
    // Line breaks, tabs and bidi controls would fake rows or reorder text, so they become spaces.
    let spaced = |c: char| matches!(c, '\n' | '\r' | '\t' | '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    let out: String = value
        .chars()
        .filter(|&c| !matches!(c, '{' | '}' | '[' | ']') && (spaced(c) || !(c.is_control() || crate::api::invisible(c))))
        .map(|c| if spaced(c) { ' ' } else { c })
        .collect();
    out.trim().to_owned()
}

pub(crate) fn display_n(value: &str, max_chars: usize) -> String {
    let mut out = display(value);
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
    }
    out
}

pub(crate) fn bounded_output(value: String, max_bytes: usize) -> String {
    bounded_output_with_tail(value, max_bytes, " More text was omitted.")
}

pub(crate) fn bounded_output_with_tail(value: String, max_bytes: usize, tail: &str) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut out = String::new();
    for ch in value.chars() {
        if out.len() + ch.len_utf8() + tail.len() > max_bytes {
            break;
        }
        out.push(ch);
    }
    out.push_str(tail);
    out
}

fn capitalize(value: String) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else { return value };
    first.to_uppercase().collect::<String>() + chars.as_str()
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", display_n(value, 64))
}

/// Optional-setting warnings from a status or env object; empty when the helper did not report them.
pub fn env_notes(value: &Value) -> Vec<&'static str> {
    let mut notes = Vec::new();
    if value.get("battery_exempt").and_then(Value::as_bool) == Some(false) {
        notes.push("Battery optimization is on for Android-Use, so Android may stop the helper in the background. Exempt it under Settings → Battery.");
    }
    if value.get("talkback").and_then(Value::as_bool) == Some(true) {
        notes.push("TalkBack is on and can change what the screen reports.");
    }
    notes
}

pub(crate) fn format_status(value: &Value) -> String {
    let connected = value.get("ok").and_then(Value::as_u64) == Some(1);
    let control = value.get("cap").and_then(Value::as_u64).unwrap_or(0) != 0;
    match (connected, control) {
        (true, true) => "Ready. Android Use is connected and can read and control the screen.".into(),
        (true, false) => "Not ready: the Android Use helper is running, but its Accessibility service is off. Ask the user to turn on Android-Use under Settings → Accessibility on the device, then retry.".into(),
        _ => "Android Use is not ready. Ask the user to run au doctor on the computer, then retry.".into(),
    }
}

// ---- Android screen ------------------------------------------------------

/// How many rows a screen read lists before pointing at `find`.
const SCREEN_ROWS: usize = 64;
const FULL_SCREEN_ROWS: usize = 256;

/// Kind tags for controls that are not simply tapped; buttons and plain text have none.
fn kind_tag(kind: &str) -> &'static str {
    match kind {
        "switch" => "sw ",
        "text field" => "edit ",
        "checkbox" => "chk ",
        "radio" => "radio ",
        "tab" => "tab ",
        "link" => "link ",
        "scroll area" => "list ",
        "slider" => "slider ",
        _ => "",
    }
}

/// Whether a row's secondary text is short enough for the compact screen: a state or a value
/// (`Off`, `T-Mobile`), not a descriptive subtitle, which `screen full` keeps.
fn short_value(value: &str) -> bool {
    value.chars().count() <= 28 && value.split_whitespace().count() <= 3 && !value.contains(',')
}

/// The number each row would need if it shares its label with another: `#N`, in reading order,
/// the order the resolver counts in. Headings are never targets, and plain text is skipped when a
/// control carries the same words (the resolver prefers the control), so a listed number is always
/// one the resolver accepts.
pub(crate) fn duplicate_numbers(rows: &[SemanticRow]) -> Vec<Option<usize>> {
    let counted = |row: &SemanticRow, controls_exist: bool| row.kind != "heading" && !row.label.is_empty() && (!controls_exist || !row.kind.is_empty());
    let mut controls: HashMap<String, bool> = HashMap::new();
    for row in rows.iter().filter(|row| !row.kind.is_empty() && row.kind != "heading") {
        controls.insert(normalized(&row.label), true);
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let key = normalized(&row.label);
        if counted(row, controls.contains_key(&key)) {
            *counts.entry(key).or_default() += 1;
        }
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    rows.iter()
        .map(|row| {
            let key = normalized(&row.label);
            (counted(row, controls.contains_key(&key)) && counts.get(&key).copied().unwrap_or(0) > 1).then(|| {
                let number = seen.entry(key).or_default();
                *number += 1;
                *number
            })
        })
        .collect()
}

/// Renders compiled rows, one per line: `btn Save`, `sw Wi-Fi=on`, `edit Email="a@b.c"`, plain text as
/// is, and `Label#2` where a label repeats (send `tap "Label" number 2`).
pub(crate) fn format_semantic_rows(rows: &[SemanticRow], full: bool, focus: &str) -> String {
    if rows.is_empty() {
        return if focus.is_empty() {
            "The screen has no readable controls. Read screen again after the view changes, or capture screen to see it.".into()
        } else {
            format!("Nothing on the screen matches {}. Read screen for the current view.", quoted(focus))
        };
    }
    let numbers = duplicate_numbers(rows);
    let limit = if full { FULL_SCREEN_ROWS } else { SCREEN_ROWS };
    let collapsed = if full { Vec::new() } else { crate::collapse::runs(rows) };
    let lines: Vec<String> = if collapsed.is_empty() {
        rows.iter().zip(&numbers).take(limit).map(|(row, number)| semantic_line(row, *number, full)).filter(|line| !line.is_empty()).collect()
    } else {
        collapsed.iter().take(limit).map(|part| part.line(rows, &numbers)).filter(|line| !line.is_empty()).collect()
    };
    let omitted = if collapsed.is_empty() { rows.len().saturating_sub(limit) } else { collapsed.len().saturating_sub(limit) };
    // The count comes first so cutting the body to its byte budget can never remove it.
    let mut text = String::new();
    if omitted > 0 {
        text.push_str(&format!("{omitted} more rows not shown; use find \"TEXT\"{}.\n", if full { "" } else { " or screen full" }));
    }
    text.push_str(&bounded_output_with_tail(lines.join("\n"), if full { FULL_SCREEN_BYTES } else { SCREEN_BYTES }, "\n… cut to fit; use find \"TEXT\"."));
    text
}

/// Renders raw semantic rows from the helper as the agent sees `screen` (or `screen full`). Used by
/// the token-budget gate and the golden-screen tools.
pub fn render_screen(rows: &Value, full: bool) -> Result<String> {
    Ok(format_semantic_rows(&parse_semantic_rows(rows)?, full, ""))
}

/// What changed between two reads of what is plainly the same screen, as `+` added, `~` changed
/// and `-` removed rows. `None` when the screen is a different one (under 60% of the rows carry
/// over) or so much changed that listing it whole is shorter.
pub(crate) fn screen_changes(previous: &[SemanticRow], current: &[SemanticRow]) -> Option<String> {
    let key = |rows: &[SemanticRow]| {
        let mut seen: HashMap<(String, String), usize> = HashMap::new();
        rows.iter()
            .map(|row| {
                let nth = seen.entry((normalized(&row.label), row.kind.clone())).or_default();
                *nth += 1;
                (normalized(&row.label), row.kind.clone(), *nth)
            })
            .collect::<Vec<_>>()
    };
    let (before, after) = (key(previous), key(current));
    let kept = after.iter().filter(|k| before.contains(k)).count();
    if kept * 10 < previous.len().max(current.len()) * 6 {
        return None;
    }
    let numbers = duplicate_numbers(current);
    let mut lines = Vec::new();
    for ((row, number), k) in current.iter().zip(&numbers).zip(&after) {
        let marker = match before.iter().position(|b| b == k) {
            None => "+",
            Some(at) if previous[at] != *row => "~",
            Some(_) => continue,
        };
        let line = semantic_line(row, *number, false);
        if !line.is_empty() {
            lines.push(format!("{marker} {line}"));
        }
    }
    for (row, k) in previous.iter().zip(&before) {
        if !after.contains(k) && !row.label.is_empty() {
            lines.push(format!("- {}", display_n(&row.label, 64)));
        }
    }
    (lines.len() <= 12).then(|| if lines.is_empty() { "Unchanged.".into() } else { lines.join("\n") })
}

pub(crate) fn semantic_delta(previous: &[SemanticRow], current: &[SemanticRow]) -> Vec<SemanticRow> {
    current.iter().filter(|row| !previous.contains(row)).cloned().collect()
}

pub(crate) fn format_semantic_delta(rows: &[SemanticRow]) -> String {
    if rows.is_empty() {
        return "No semantic screen changes.".into();
    }
    let lines = rows.iter().map(|row| semantic_line(row, None, false)).filter(|line| !line.is_empty()).collect::<Vec<_>>();
    bounded_output_with_tail(lines.join("\n"), SCREEN_BYTES, "\nRead screen for the complete current state.")
}

/// One row as an agent reads it. The label is always verbatim, so it can be quoted back exactly.
pub(crate) fn semantic_line(row: &SemanticRow, number: Option<usize>, full: bool) -> String {
    let mut label = display_n(&row.label, 96);
    // An icon-only control has no words to quote; its handle is the name.
    if label.is_empty() && !row.kind.is_empty() {
        label = row.handle.map(|handle| format!("e{handle}")).unwrap_or_default();
    }
    if label.is_empty() || row.kind == "heading" {
        return label;
    }
    let name = number.map_or_else(|| label.clone(), |n| format!("{label}#{n}"));
    let tag = kind_tag(&row.kind);
    let value = display_n(&row.value, if full { 128 } else { 40 });
    let state = display_n(&row.state, 32);
    let on = |checked: bool| if checked { "on" } else { "off" };
    let mut line = match row.kind.as_str() {
        "switch" => format!("{tag}{name}={}", on(matches!(state.as_str(), "checked" | "on"))),
        "checkbox" => format!("{tag}{name}={}", on(state == "checked")),
        "radio" | "tab" => format!("{tag}{name}{}", if row.selected || state == "checked" { "=on" } else { "" }),
        // A field showing its own hint ("Search" in an empty search box) is empty.
        "text field" => format!("{tag}{name}=\"{}\"", if normalized(&value) == normalized(&label) { "" } else { value.as_str() }),
        _ if !value.is_empty() && (full || short_value(&value)) => format!("{tag}{name}: {value}"),
        _ if row.kind.is_empty() && !state.is_empty() && state != "selected" => format!("{name}: {state}"),
        _ => format!("{tag}{name}"),
    };
    if !row.enabled {
        line.push_str(" [disabled]");
    }
    if row.selected && !matches!(row.kind.as_str(), "radio" | "tab") {
        line.push_str(" [selected]");
    }
    line
}

// ---- Chrome ---------------------------------------------------------------

pub(crate) fn format_browser_tabs(value: &Value) -> String {
    // Only pages are tabs: service workers and frames cannot be selected or closed by title.
    let pages: Vec<&Value> =
        value.get("tabs").and_then(Value::as_array).map(|tabs| tabs.iter().filter(|tab| tab.get("type").and_then(Value::as_str) == Some("page")).collect()).unwrap_or_default();
    if pages.is_empty() {
        return "Chrome has no open tabs. Use page open \"URL\" to open one.".into();
    }
    let names: Vec<_> = pages.iter().take(12).map(|tab| display_n(tab.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("untitled"), 80)).collect();
    let more = if pages.len() > names.len() { format!(", and {} more", pages.len() - names.len()) } else { String::new() };
    let truncated = if value.get("truncated").and_then(Value::as_bool) == Some(true) { " More tabs may be open." } else { "" };
    bounded_output(format!("Chrome has {} tabs: {}{more}.{truncated}", pages.len(), names.join("; ")), 640)
}

/// Page controls use the same tags as the Android screen.
fn role_tag(role: u8) -> &'static str {
    match role as char {
        'i' => "edit ",
        'c' => "chk ",
        's' => "list ",
        'a' => "link ",
        _ => "",
    }
}

pub(crate) fn format_browser_page(value: &Value) -> String {
    let title = display_n(value.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("untitled"), 96);
    let host = value.get("url").and_then(Value::as_str).and_then(|url| url.split("://").nth(1)).and_then(|rest| rest.split('/').next()).unwrap_or("");
    let rows = value.get("n").and_then(Value::as_array).cloned().unwrap_or_default();
    let controls: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let row = row.as_array()?;
            let label = row.get(1).and_then(Value::as_str).filter(|label| !label.trim().is_empty())?;
            let role = row.get(2).and_then(Value::as_str).and_then(|s| s.as_bytes().first().copied()).unwrap_or(b'm');
            Some(format!("{}{}", role_tag(role), display_n(label, 72)))
        })
        .take(30)
        .collect();
    let page = if host.is_empty() { title } else { format!("{title} ({})", display_n(host, 64)) };
    if controls.is_empty() {
        format!("Page: {page}. No readable controls; use page text for its content.")
    } else {
        bounded_output_with_tail(format!("Page: {page}\n{}", controls.join("\n")), SCREEN_BYTES, "\n… cut to fit; use page text.")
    }
}

pub(crate) fn format_page_text(value: &Value, matching: bool) -> String {
    let text = display(value.get("text").and_then(Value::as_str).unwrap_or(""));
    match (text.is_empty(), matching) {
        (true, true) => return "No matching page text was found. Read page text for a broader view.".into(),
        (true, false) => return "The page has no readable text yet. Read the page again after it loads.".into(),
        _ => {}
    }
    let limit = if matching { 900 } else { 2400 };
    let excerpt: String = text.chars().take(limit).collect();
    let omitted = text.chars().count() > limit;
    format!("Page text: {excerpt}{}", if omitted { "\nMore text was omitted; use page text matching \"TEXT\" to jump to a part of the page." } else { "" })
}

// ---- Device state -------------------------------------------------------

pub(crate) fn format_capabilities(value: &Value) -> String {
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let hardware = |present: bool, granted: bool| match (present, granted) {
        (false, _) => "not available",
        (true, true) => "ready",
        (true, false) => "needs permission",
    };
    let parts = [
        format!("screenshots {}", if flag("screen_capture") { "ready" } else { "not available" }),
        format!("camera {}", hardware(flag("camera"), flag("camera_permission"))),
        format!("microphone {}", hardware(flag("microphone"), flag("microphone_permission"))),
        format!("location {}", if flag("location_permission") { "ready" } else { "needs permission" }),
        format!("screen recording {}", if flag("screen_record") { "ready" } else { "needs approval" }),
        format!("notifications {}", if flag("notifications") { "ready" } else { "need notification access" }),
    ];
    let mut text = format!("Capabilities: {}.", parts.join("; "));
    if parts.iter().any(|part| part.contains("need")) {
        text.push_str(" The user grants permissions and approvals in the Android-Use app on the device.");
    }
    text
}

pub(crate) fn format_location(value: &Value) -> String {
    if value.get("permission").and_then(Value::as_bool) == Some(false) {
        return "Location permission is not granted to Android Use. Ask the user to allow location for Android Use on the device, then retry.".into();
    }
    let (Some(lat), Some(lon)) = (value.get("lat").and_then(Value::as_f64), value.get("lon").and_then(Value::as_f64)) else {
        return "No location fix is available right now. Location may be turned off on the device.".into();
    };
    let mut text = format!("Location: {lat:.5}, {lon:.5}");
    if let Some(accuracy) = value.get("acc").and_then(Value::as_f64).filter(|accuracy| *accuracy >= 0.0) {
        text.push_str(&format!(", accurate to about {} m", accuracy.round() as u64));
    }
    if value.get("current").and_then(Value::as_bool) == Some(true) {
        text.push_str(", current fix.");
    } else {
        let minutes = value.get("age_ms").and_then(Value::as_u64).unwrap_or(0) / 60_000;
        text.push_str(&match minutes {
            0 => ", last known fix from under a minute ago.".to_owned(),
            1..=119 => format!(", last known fix from {minutes} min ago."),
            _ => format!(", last known fix from {} h ago.", minutes / 60),
        });
    }
    text
}

pub(crate) fn format_notifications(value: &Value) -> String {
    if value.get("enabled").and_then(Value::as_bool) != Some(true) {
        return NOTIFICATION_ACCESS_OFF.into();
    }
    let items = crate::resolve::notification_items(value);
    if items.is_empty() {
        return if value.get("active").and_then(Value::as_bool) == Some(false) {
            "Notification access is on, but the listener has not connected yet. Retry in a few seconds.".into()
        } else {
            "There are no notifications.".into()
        };
    }
    // Listed and targeted by the same label (title, or text when untitled), so a number shown
    // here selects the same item in `dismiss notification` and the other notification commands.
    let label = |item: &Value| display_n(&crate::resolve::notification_label(item), 120);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in &items {
        *counts.entry(normalized(&label(item))).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut lines = vec![format!("{} notifications, newest first:", items.len())];
    for item in items.iter().take(16) {
        let (title, text) = (display_n(item.get("title").and_then(Value::as_str).unwrap_or(""), 120), display_n(item.get("text").and_then(Value::as_str).unwrap_or(""), 120));
        let shown = label(item);
        let key = normalized(&shown);
        let number = if counts.get(&key).copied().unwrap_or(0) > 1 {
            let n = seen.entry(key).or_default();
            *n += 1;
            format!(" (number {n})")
        } else {
            String::new()
        };
        let shown = if shown.is_empty() { "(no title or text)".to_owned() } else { shown };
        let detail = if title.is_empty() || text.is_empty() { String::new() } else { format!(" — {text}") };
        lines.push(format!("- {shown}{number}{detail}"));
    }
    bounded_output(lines.join("\n"), SCREEN_BYTES)
}

pub(crate) fn format_image_hash(alias: &str, value: &Value) -> String {
    let hash = display_n(value.get("hash").and_then(Value::as_str).unwrap_or("unavailable"), 96);
    format!("Image {} has hash {hash}.", quoted(alias))
}

pub(crate) fn format_image_difference(left: &str, right: &str, value: &Value) -> String {
    let verdict = if value.get("changed").and_then(Value::as_u64) == Some(1) { "are different" } else { "match" };
    format!("Images {} and {} {verdict}.", quoted(left), quoted(right))
}

// ---- Action results ----------------------------------------------------

/// Imperative description of one step, used to name what failed.
pub(crate) fn describe_action(action: &Action) -> String {
    use AndroidAction as A;
    use BrowserAction as B;
    match action {
        Action::Android(action) => match action {
            A::Tap(t) => format!("tap {}", quoted(&t.label)),
            A::Toggle(t) => format!("toggle {}", quoted(&t.label)),
            A::Set { target, on } => format!("set {} {}", quoted(&target.label), if *on { "on" } else { "off" }),
            A::Hold(t) => format!("hold {}", quoted(&t.label)),
            A::Type { target, .. } => format!("type into {}", quoted(&target.label)),
            A::Scroll { direction, target } => format!("scroll {} in {}", format!("{direction:?}").to_lowercase(), quoted(&target.label)),
            A::ScrollScreen(direction) => format!("scroll {}", format!("{direction:?}").to_lowercase()),
            A::ScrollUntil { direction, text, .. } => format!("scroll {} until {}", format!("{direction:?}").to_lowercase(), quoted(text)),
            A::Key(key) => format!("press {}", format!("{key:?}").to_lowercase()),
            A::WaitTarget { target, .. } => format!("wait for {}", quoted(&target.label)),
            A::WaitText { text, .. } => format!("wait for text {}", quoted(text)),
            A::WaitScreenChange { .. } => "wait for the screen to change".into(),
            A::VerifyExists(t) => format!("verify {} exists", quoted(&t.label)),
            A::VerifyGone(t) => format!("verify {} is gone", quoted(&t.label)),
            A::VerifyText(text) => format!("verify text {} exists", quoted(text)),
            A::OpenApp(name) => format!("open app {}", quoted(name)),
            A::OpenSetting(name) => format!("open setting {}", quoted(name)),
            A::OpenLink(url) => format!("open link {}", quoted(url)),
            A::CaptureScreen => "capture the screen".into(),
            A::Camera { .. } => "take a photo".into(),
            A::Microphone(_) => "record the microphone".into(),
            A::ScreenRecord(_) => "record the screen".into(),
            A::NotificationOpen(t) => format!("open notification {}", quoted(&t.label)),
            A::NotificationDismiss(t) => format!("dismiss notification {}", quoted(&t.label)),
            A::NotificationAction(t, _) => format!("run notification action {}", quoted(&t.label)),
            A::PointTap { x, y } => format!("tap point {x} {y}"),
            A::Swipe { .. } => "swipe".into(),
        },
        Action::Browser(action) => match action {
            B::Open(url) => format!("page open {}", quoted(url)),
            B::Click(t) => format!("page click {}", quoted(&t.label)),
            B::Focus(t) => format!("page focus {}", quoted(&t.label)),
            B::Type { target, .. } => format!("page type into {}", quoted(&target.label)),
            B::Key(key) => format!("page press {}", quoted(key)),
            B::Scroll(px) => format!("page scroll {px}"),
            B::WaitText { text, .. } => format!("page wait for text {}", quoted(text)),
            B::WaitCss { selector, .. } => format!("page wait for css {}", quoted(selector)),
            B::Back => "page back".into(),
            B::Forward => "page forward".into(),
            B::Reload => "page reload".into(),
            B::Screenshot => "page screenshot".into(),
            B::SelectTab(t) => format!("select tab {}", quoted(&t.label)),
            B::CloseTab(t) => format!("close tab {}", quoted(&t.label)),
            B::NewTab(url) => format!("new tab {}", quoted(url)),
        },
        Action::Visual(_) => "crop the image".into(),
        Action::Look(_) => "read the screen".into(),
    }
}

/// Past-tense description of one completed step.
fn done_phrase(action: &Action) -> String {
    use AndroidAction as A;
    use BrowserAction as B;
    let short = |value: &str| display_n(value, 64);
    match action {
        Action::Android(A::Tap(t)) => format!("tapped {}", short(&t.label)),
        Action::Android(A::Toggle(t)) => format!("toggled {}", short(&t.label)),
        Action::Android(A::Set { target, on }) => format!("set {} {}", short(&target.label), if *on { "on" } else { "off" }),
        Action::Android(A::Hold(t)) => format!("held {}", short(&t.label)),
        Action::Android(A::Type { text, target }) | Action::Browser(B::Type { text, target }) => format!("typed {} in {}", short(text), short(&target.label)),
        Action::Android(A::VerifyExists(t)) => format!("verified {} exists", short(&t.label)),
        Action::Android(A::VerifyGone(t)) => format!("verified {} is gone", short(&t.label)),
        Action::Android(A::VerifyText(text)) => format!("verified that {} appeared", short(text)),
        Action::Android(A::Scroll { direction, .. } | A::ScrollScreen(direction)) => format!("scrolled {}", format!("{direction:?}").to_lowercase()),
        Action::Android(A::ScrollUntil { direction, text, .. }) => format!("scrolled {} until {} appeared", format!("{direction:?}").to_lowercase(), short(text)),
        Action::Android(A::Key(key)) => format!("pressed {}", format!("{key:?}").to_lowercase()),
        Action::Android(A::OpenApp(name)) => format!("opened {}", short(name)),
        Action::Android(A::OpenSetting(name)) => format!("opened {} settings", short(name)),
        Action::Android(A::OpenLink(_)) => "opened the link".into(),
        Action::Android(A::CaptureScreen) => "captured the screen".into(),
        Action::Android(A::Camera { .. }) => "took a photo".into(),
        Action::Android(A::Microphone(_)) => "recorded audio".into(),
        Action::Android(A::ScreenRecord(_)) => "recorded the screen".into(),
        Action::Android(A::NotificationOpen(t)) => format!("opened the {} notification", short(&t.label)),
        Action::Android(A::NotificationDismiss(t)) => format!("dismissed the {} notification", short(&t.label)),
        Action::Android(A::NotificationAction(t, _)) => format!("ran the {} notification action", short(&t.label)),
        Action::Android(A::PointTap { x, y }) => format!("tapped point {x} {y}"),
        Action::Android(A::Swipe { .. }) => "swiped".into(),
        Action::Android(A::WaitTarget { .. } | A::WaitText { .. } | A::WaitScreenChange { .. }) | Action::Browser(B::WaitText { .. } | B::WaitCss { .. }) => {
            "met the wait condition".into()
        }
        Action::Browser(B::Open(url)) => format!("opened {}", short(url)),
        Action::Browser(B::Click(t)) => format!("clicked {}", short(&t.label)),
        Action::Browser(B::Focus(t)) => format!("focused {}", short(&t.label)),
        Action::Browser(B::Key(key)) => format!("pressed {}", short(key)),
        Action::Browser(B::Scroll(_)) => "scrolled the page".into(),
        Action::Browser(B::Back) => "went back".into(),
        Action::Browser(B::Forward) => "went forward".into(),
        Action::Browser(B::Reload) => "reloaded the page".into(),
        Action::Browser(B::Screenshot) => "captured the page".into(),
        Action::Browser(B::SelectTab(t)) => format!("switched to the {} tab", short(&t.label)),
        Action::Browser(B::CloseTab(t)) => format!("closed the {} tab", short(&t.label)),
        Action::Browser(B::NewTab(_)) => "opened a new tab".into(),
        Action::Visual(_) => "cropped the image".into(),
        Action::Look(_) => "looked".into(),
    }
}

pub(crate) fn success_text(actions: &[Action]) -> String {
    const SHOWN: usize = 6;
    let mut parts: Vec<String> = actions.iter().take(SHOWN).map(done_phrase).collect();
    if actions.len() > SHOWN {
        parts.push(format!("{} more steps", actions.len() - SHOWN));
    }
    let text = match parts.len() {
        0 => return "Done.".into(),
        1 => format!("Done. {}.", capitalize(parts.remove(0))),
        2 => format!("Done. {} and {}.", capitalize(parts.remove(0)), parts.remove(0)),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!("Done. {}, and {last}.", capitalize(parts.join(", ")))
        }
    };
    bounded_output(text, 320)
}

fn wait_condition(action: &Action) -> Option<(u16, String)> {
    match action {
        Action::Android(AndroidAction::WaitTarget { target, seconds }) => Some((*seconds, format!("{} to appear", quoted(&target.label)))),
        Action::Android(AndroidAction::WaitText { text, seconds }) => Some((*seconds, format!("text {} to appear", quoted(text)))),
        Action::Android(AndroidAction::WaitScreenChange { seconds }) => Some((*seconds, "the screen to change".into())),
        Action::Browser(BrowserAction::WaitText { text, seconds }) => Some((*seconds, format!("page text {} to appear", quoted(text)))),
        Action::Browser(BrowserAction::WaitCss { selector, seconds }) => Some((*seconds, format!("an element matching {} to appear", quoted(selector)))),
        _ => None,
    }
}

/// Explains a failed receipt: what failed, whether anything changed, and the
/// single safe next move.
pub(crate) fn failure_text(receipt: &Receipt, actions: &[Action]) -> String {
    let step = receipt.at.map(usize::from).filter(|index| *index < actions.len()).map(|index| (index, &actions[index]));
    let what = step.map_or_else(|| "run the command".to_owned(), |(_, action)| describe_action(action));
    let which = step.map_or_else(|| "a later step".to_owned(), |(index, action)| format!("step {} of {} ({})", index + 1, actions.len(), describe_action(action)));
    let text = if let (Some((_, Action::Android(AndroidAction::ScrollUntil { text, .. }))), true) = (step, receipt.e.as_deref() == Some("partial")) {
        format!("Scrolled, but {} did not appear; the list may have ended. The screen has moved: read it before doing anything else.", quoted(text))
    } else if receipt.m > 0 && receipt.e.as_deref() == Some("unknown") {
        format!("Partly done, and uncertain: at least {} change(s) were made, and {which} may or may not have finished. Read the screen before doing anything else, and do not repeat the whole command.", receipt.m)
    } else if receipt.m > 0 || receipt.e.as_deref() == Some("partial") {
        format!("Partly done: {} change(s) were made before {which} failed. Read the screen before doing anything else, and do not repeat the whole command.", receipt.m)
    } else {
        match receipt.e.as_deref() {
            Some("stale") => "The screen changed before anything ran, so nothing was done. Retry the same command.".into(),
            Some("unknown") => "The result is uncertain: the command may or may not have run. Read the screen before doing anything else, and do not repeat it blindly.".into(),
            Some("permission") => {
                format!("Android Use is missing the Android permission needed to {what}. Ask the user to grant it in the Android-Use app on the device, then retry.")
            }
            Some("timeout") => format!("Timed out at {which}; nothing changed. Retry if it is still needed."),
            Some("unsupported") => format!("Cannot {what} on this device or Android version; nothing changed."),
            Some("bounds") => format!("Cannot {what}: it exceeds a safety limit. Nothing changed."),
            Some("ambiguous") => "The target is ambiguous; nothing changed. Read the screen and add number N after the label.".into(),
            _ => match step {
                Some((_, action)) if wait_condition(action).is_some() => {
                    let (seconds, condition) = wait_condition(action).unwrap_or_default();
                    format!("Waited {seconds} seconds for {condition}, but it did not happen. Nothing changed.")
                }
                Some((_, Action::Android(AndroidAction::VerifyExists(_) | AndroidAction::VerifyGone(_) | AndroidAction::VerifyText(_)))) => {
                    format!("Check failed: {what} is not true on the current screen. Nothing changed.")
                }
                Some(_) => format!("Could not {what} on the current screen; nothing changed. Read the screen and choose another target."),
                None => "The command failed before it changed anything. Read the screen, then retry if it is still needed.".into(),
            },
        }
    };
    bounded_output(text, 480)
}

pub(crate) fn receipt_code(receipt: &Receipt) -> Code {
    if receipt.m > 0 {
        return Code::Partial;
    }
    match receipt.e.as_deref() {
        Some("stale") => Code::Stale,
        Some("unknown") => Code::Unknown,
        Some("timeout") => Code::Timeout,
        Some("permission") => Code::Permission,
        Some("ambiguous") => Code::Ambiguous,
        Some("unsupported") => Code::Unsupported,
        Some("bounds") => Code::Bounds,
        Some("args") => Code::Args,
        _ => Code::Helper,
    }
}

/// Turns any error into one short recovery instruction for an agent.
/// Internal diagnostics are summarized rather than echoed.
pub(crate) fn plain_error_model(error: &Error) -> String {
    let message = display(&error.message);
    if error.plain {
        return bounded_output(message, 720);
    }
    match error.code {
        Code::Args | Code::Bounds | Code::Unsupported if message.contains("Use ") => bounded_output(message, 320),
        Code::Args | Code::Bounds | Code::Unsupported => bounded_output(format!("The command needs a correction: {message}."), 320),
        Code::Ambiguous | Code::Missing | Code::Partial | Code::Locked | Code::ScreenOff | Code::HelperKilled | Code::Stopped | Code::Blocked | Code::Paused | Code::Denied => bounded_output(message, 480),
        Code::Permission => "Android Use's Accessibility service is off or a required Android permission is missing. Ask the user to turn on Android-Use under Settings → Accessibility on the device (and grant any permission the task needs), then retry.".into(),
        Code::Artifact => bounded_output(format!("{message}. Capture the screen before using an image alias."), 320),
        Code::Stale => "The screen changed before anything ran, so nothing was done. Retry the same command.".into(),
        Code::Unknown => "The result is uncertain: the command may or may not have run. Read the screen before doing anything else, and do not repeat it blindly.".into(),
        Code::Identity => "The enrolled Android device is not connected. Ask the user to connect and unlock it and approve USB debugging if Android asks, then retry. A different device needs au setup first.".into(),
        Code::Device if message.contains("adb was not found") => "Android platform tools (adb) were not found on this computer. Ask the user to install Android SDK Platform-Tools or set AU_ADB, then run au doctor.".into(),
        Code::Device => bounded_output(format!("The computer cannot reach the Android device ({message}). Ask the user to connect and unlock it with a data cable and approve USB debugging; au doctor explains what is missing."), 480),
        Code::Timeout => "The device took too long to respond. Make sure it is unlocked and awake, then retry.".into(),
        Code::Helper | Code::Auth | Code::Sequence | Code::Protocol | Code::Io => {
            "The Android Use helper on the device did not respond correctly. Retry once; if it fails again, ask the user to run au doctor, and au repair if the helper is broken.".into()
        }
    }
}

pub(crate) fn mime_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") {
        "audio/wav"
    } else if bytes.get(4..8) == Some(b"ftyp") {
        "video/mp4"
    } else {
        "application/octet-stream"
    }
}

pub(crate) fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "audio/wav" => "wav",
        "video/mp4" => "mp4",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::api::Target;
    use serde_json::json;

    fn row(label: &str, kind: &str, value: &str, state: &str) -> SemanticRow {
        SemanticRow { label: label.into(), value: value.into(), kind: kind.into(), state: state.into(), enabled: true, selected: false, handle: None }
    }

    #[test]
    fn model_text_is_plain_and_bounded() {
        let actions = vec![
            Action::Android(AndroidAction::Type { text: "Sample text".into(), target: Target { label: "Name".into(), ordinal: None, handle: None } }),
            Action::Android(AndroidAction::Tap(Target { label: "Save".into(), ordinal: None, handle: None })),
            Action::Android(AndroidAction::VerifyText("Submitted".into())),
        ];
        assert_eq!(success_text(&actions), "Done. Typed Sample text in Name, tapped Save, and verified that Submitted appeared.");
    }

    #[test]
    fn newlines_and_bidi_overrides_in_labels_render_on_one_line() {
        let line = semantic_line(&row("Pay\nSend all\t\u{202E}evil\u{2066}x", "button", "", ""), None, false);
        assert_eq!(line, "btn Pay Send all  evil x".replace("btn ", kind_tag("button")));
        assert!(!line.contains(['\n', '\u{202E}', '\u{2066}']));
        assert_eq!(display("a\u{200B}b\u{FEFF}c\u{E0041}"), "abc", "invisible characters are dropped");
    }

    #[test]
    fn duplicate_rows_show_the_number_an_agent_must_use() {
        let rows = [row("Settings", "heading", "", ""), row("Save", "button", "", ""), row("Wi-Fi", "switch", "", "checked"), row("Save", "button", "", "")];
        let text = format_semantic_rows(&rows, false, "");
        assert_eq!(text, "Settings\nSave#1\nsw Wi-Fi=on\nSave#2");
        assert_eq!(semantic_line(&row("Name", "text field", "", ""), None, false), "edit Name=\"\"");
    }

    #[test]
    fn rows_are_dense_but_labels_stay_verbatim() {
        let mut off = row("Alarm", "switch", "", "unchecked");
        off.enabled = false;
        let lines: Vec<_> = [
            row("Internet", "", "AndroidWifi", ""),
            row("Network", "control", "Mobile, Wi‑Fi, hotspot", ""),
            row("Email", "text field", "a@b.co", ""),
            off,
            row("List", "scroll area", "", ""),
        ]
        .iter()
        .map(|r| semantic_line(r, None, false))
        .collect();
        assert_eq!(lines, ["Internet: AndroidWifi", "Network", "edit Email=\"a@b.co\"", "sw Alarm=off [disabled]", "list List"]);
        assert_eq!(semantic_line(&row("Network", "control", "Mobile, Wi‑Fi, hotspot", ""), None, true), "Network: Mobile, Wi‑Fi, hotspot", "full keeps subtitles");
    }

    #[test]
    fn a_change_on_the_same_screen_is_a_diff_and_a_new_screen_is_not() {
        let before = [
            row("Settings", "heading", "", ""),
            row("Wi-Fi", "switch", "", "checked"),
            row("Bluetooth", "switch", "", "unchecked"),
            row("Saved", "", "", ""),
            row("More", "button", "", ""),
        ];
        let mut after = before.to_vec();
        after[1] = row("Wi-Fi", "switch", "", "unchecked");
        after.remove(3);
        after.push(row("Done", "", "", ""));
        assert_eq!(screen_changes(&before, &after).unwrap(), "~ sw Wi-Fi=off\n+ Done\n- Saved");
        assert_eq!(screen_changes(&before, &before).unwrap(), "Unchanged.");
        let other = [row("Network", "heading", "", ""), row("Internet", "button", "", ""), row("VPN", "button", "", "")];
        assert!(screen_changes(&before, &other).is_none(), "a different screen is listed whole");
    }

    #[test]
    fn numbers_skip_plain_text_that_shares_words_with_a_control() {
        let rows = [row("Battery Saver", "", "", ""), row("Battery Saver", "switch", "", "checked"), row("Save", "button", "", ""), row("Save", "button", "", "")];
        let text = format_semantic_rows(&rows, false, "");
        assert_eq!(text, "Battery Saver\nsw Battery Saver=on\nSave#1\nSave#2");
    }

    #[test]
    fn omitted_rows_are_counted_before_the_body_so_truncation_keeps_the_count() {
        let rows: Vec<_> = (0..90).map(|i| row(&format!("{i} item"), "button", "", "")).collect();
        let text = format_semantic_rows(&rows, false, "");
        assert!(text.starts_with("26 more rows not shown; use find"), "{text}");
    }

    #[test]
    fn device_state_reads_report_values_not_field_names() {
        let location = format_location(&json!({"permission":true,"available":true,"current":true,"lat":47.606_21,"lon":-122.332_07,"acc":12.4,"age_ms":0}));
        assert_eq!(location, "Location: 47.60621, -122.33207, accurate to about 12 m, current fix.");
        assert!(format_location(&json!({"permission":false,"available":false})).contains("not granted"));
        let notifications = format_notifications(&json!({"enabled":true,"active":true,"items":[
            {"id":"a","title":"Messages","text":"Lunch?"},{"id":"b","title":"Messages","text":"Running late"},{"id":"c","title":"Calendar","text":""}
        ]}));
        assert_eq!(notifications, "3 notifications, newest first:\n- Calendar\n- Messages (number 1) — Running late\n- Messages (number 2) — Lunch?");
        assert_eq!(format_notifications(&json!({"enabled":false,"items":[]})), NOTIFICATION_ACCESS_OFF);
        let capabilities = format_capabilities(&json!({"screen_capture":true,"camera":true,"camera_permission":false,"microphone":true,"microphone_permission":true}));
        assert!(capabilities.contains("camera needs permission") && capabilities.contains("microphone ready"));
    }

    #[test]
    fn failures_name_the_step_and_whether_anything_changed() {
        let actions =
            vec![Action::Android(AndroidAction::Tap(Target { label: "Save".into(), ordinal: None, handle: None })), Action::Android(AndroidAction::VerifyText("Saved".into()))];
        let failed = |m: u8, at: u8, e: &str| Receipt { id: "x".into(), ok: 0, g: 1, m, at: Some(at), e: Some(e.into()), partial: None, next: None, artifact: None };
        assert!(failure_text(&failed(1, 1, "partial"), &actions).contains("step 2 of 2 (verify text \"Saved\" exists)"));
        assert_eq!(receipt_code(&failed(1, 1, "partial")), Code::Partial);
        assert!(failure_text(&failed(1, 1, "unknown"), &actions).contains("step 2 of 2 (verify text \"Saved\" exists) may or may not have finished"));
        let verify = [Action::Android(AndroidAction::VerifyText("Saved".into()))];
        assert!(failure_text(&failed(0, 0, "helper"), &verify).starts_with("Check failed"));
        let wait = [Action::Android(AndroidAction::WaitText { text: "Done".into(), seconds: 5 })];
        assert!(failure_text(&failed(0, 0, "helper"), &wait).starts_with("Waited 5 seconds for text \"Done\" to appear"));
    }

    #[test]
    fn tabs_list_only_pages_and_say_when_there_are_more() {
        let tabs = json!({"tabs":[
            {"id":"p1","type":"page","title":"Docs"},
            {"id":"w1","type":"service_worker","title":"Worker"},
            {"id":"p2","type":"page","title":""}
        ], "truncated": false});
        assert_eq!(format_browser_tabs(&tabs), "Chrome has 2 tabs: Docs; untitled.");
        let many = json!({"tabs": (0..14).map(|i| json!({"id": format!("p{i}"), "type": "page", "title": format!("T{i}")})).collect::<Vec<_>>(), "truncated": true});
        assert!(format_browser_tabs(&many).contains("Chrome has 14 tabs: T0; T1; T2; T3; T4; T5; T6; T7; T8; T9; T10; T11, and 2 more. More tabs may be open."));
    }

    #[test]
    fn model_errors_hide_wire_syntax_and_give_a_next_step() {
        let error = Error::new(Code::Ambiguous, "Save matches two controls. Use tap \"Save\" number 1 or number 2.");
        let text = plain_error_model(&error);
        assert!(!text.contains(['{', '}', '[', ']']));
        assert!(plain_error_model(&Error::new(Code::Identity, "no connected transport matches")).contains("not connected"));
        assert!(plain_error_model(&Error::new(Code::Device, "adb was not found; set AU_ADB")).contains("Platform-Tools"));
        assert_eq!(plain_error_model(&Error::plain(Code::Helper, "Check failed.")), "Check failed.");
        assert_eq!(mime_type(b"\0\0\0\x18ftypmp42"), "video/mp4");
    }
}
