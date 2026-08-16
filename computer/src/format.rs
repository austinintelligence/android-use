use super::*;
use crate::api::normalized;

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

pub(super) fn semantic_labels(value: &str) -> Vec<String> {
    let mut labels = vec![normalized(value)];
    if let Some((_, tail)) = value.split_once(":id/") {
        let friendly = tail.replace(['_', '-'], " ");
        labels.push(normalized(&friendly));
    }
    labels
}

pub(super) fn display_node_label(value: &str) -> String {
    if let Some((_, tail)) = value.split_once(":id/") {
        return display_n(&tail.replace(['_', '-'], " "), 96);
    }
    display_n(value, 96)
}

pub(super) fn internal_resource_label(value: &str) -> bool {
    value.contains(":id/")
}

pub(super) fn actionable_node(node: &crate::api::Node) -> bool {
    !internal_resource_label(&node.label) || node.role as char != 't' || node.flags & 1 != 0
}

pub(super) fn display(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if matches!(c, '{' | '}' | '[' | ']' | '\0') || c.is_control() && !matches!(c, '\n' | '\r' | '\t') {
            continue;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

pub(super) fn ambiguity(label: &str, count: usize) -> String {
    let count = count.min(16);
    let noun = if count == 1 { "control" } else { "controls" };
    format!("{} matches {} {}. Use tap \"{}\" number 1 for the first or number {} for the last.", display(label), count, noun, display(label), count)
}

pub(super) fn android_deadline(actions: &[Action]) -> u32 {
    let mut deadline = 8000u32;
    for action in actions {
        if let Action::Android(value) = action {
            let seconds = match value {
                AndroidAction::WaitTarget { seconds, .. }
                | AndroidAction::WaitText { seconds, .. }
                | AndroidAction::WaitScreenChange { seconds }
                | AndroidAction::Microphone(seconds)
                | AndroidAction::ScreenRecord(seconds) => u32::from(*seconds),
                _ => 0,
            };
            deadline = deadline.max(seconds.saturating_mul(1000).saturating_add(1000));
        }
    }
    deadline.min(30_000)
}

pub(super) fn browser_deadline(actions: &[Action]) -> u32 {
    let mut deadline = 8000u32;
    for action in actions {
        if let Action::Browser(value) = action {
            let seconds = match value {
                BrowserAction::WaitText { seconds, .. } | BrowserAction::WaitCss { seconds, .. } => u32::from(*seconds),
                _ => 0,
            };
            deadline = deadline.max(seconds.saturating_mul(1000).saturating_add(1000));
        }
    }
    deadline.min(30_000)
}

pub(super) fn resolve_json_target(observed: &Value, target: &Target) -> Result<u16> {
    let needle = normalized(&target.label);
    let rows = observed.get("n").and_then(Value::as_array).ok_or_else(|| Error::new(Code::Protocol, "Chrome observation omitted its controls"))?;
    let mut candidates = rows
        .iter()
        .filter_map(|row| {
            let row = row.as_array()?;
            let index = u16::try_from(row.first()?.as_u64()?).ok()?;
            let label = row.get(1)?.as_str()?;
            (normalized(label) == needle).then_some((index, label.to_owned()))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        candidates = rows
            .iter()
            .filter_map(|row| {
                let row = row.as_array()?;
                let index = u16::try_from(row.first()?.as_u64()?).ok()?;
                let label = row.get(1)?.as_str()?;
                normalized(label).contains(&needle).then_some((index, label.to_owned()))
            })
            .collect();
    }
    if candidates.is_empty() {
        return Err(Error::new(Code::Unsupported, format!("{} is not available semantically", display(&target.label))));
    }
    if let Some(ordinal) = target.ordinal {
        return candidates.get(ordinal.saturating_sub(1) as usize).map(|(index, _)| *index).ok_or_else(|| Error::new(Code::Ambiguous, ambiguity(&target.label, candidates.len())));
    }
    if candidates.len() > 1 {
        return Err(Error::new(Code::Ambiguous, ambiguity(&target.label, candidates.len())));
    }
    Ok(candidates[0].0)
}

pub(super) fn compile_browser_action(action: &BrowserAction, observed: &Value, browser: &Browser) -> Result<crate::api::BrowserOp> {
    use crate::api::BrowserOp;
    Ok(match action {
        BrowserAction::Open(url) => BrowserOp::Navigate(url.clone()),
        BrowserAction::Click(target) => BrowserOp::Click(resolve_json_target(observed, target)?),
        BrowserAction::Focus(target) => BrowserOp::Focus(resolve_json_target(observed, target)?),
        BrowserAction::Type { text, target } => BrowserOp::Text(resolve_json_target(observed, target)?, text.clone()),
        BrowserAction::Key(key) => BrowserOp::Key(key.clone()),
        BrowserAction::Scroll(px) => BrowserOp::Scroll(*px),
        BrowserAction::WaitText { text, seconds } => BrowserOp::Wait(BrowserPredicate::Text(text.clone()), seconds.saturating_mul(1000)),
        BrowserAction::WaitCss { selector, seconds } => BrowserOp::Wait(BrowserPredicate::Css(selector.clone()), seconds.saturating_mul(1000)),
        BrowserAction::Back => BrowserOp::Back,
        BrowserAction::Forward => BrowserOp::Forward,
        BrowserAction::Reload => BrowserOp::Reload,
        BrowserAction::Screenshot => BrowserOp::Screenshot,
        BrowserAction::SelectTab(target) => BrowserOp::Select(browser.resolve_tab_target(target)?),
        BrowserAction::CloseTab(target) => BrowserOp::Close(browser.resolve_tab_target(target)?),
        BrowserAction::NewTab(url) => BrowserOp::New(url.clone()),
    })
}

pub(super) fn format_status(value: &Value) -> String {
    if value.get("ok").and_then(Value::as_u64) == Some(1) {
        "Ready. Android Use is connected.".into()
    } else {
        "Android Use is not ready. Run the setup or doctor command, then retry.".into()
    }
}

pub(super) fn role_name(role: u8) -> &'static str {
    match role as char {
        'b' => "button",
        'i' => "text field",
        'c' => "checkbox",
        's' => "scroll area",
        'm' => "control",
        't' => "text",
        _ => "item",
    }
}

pub(super) fn format_scene_focus(scene: &Scene, full: bool, matching: Option<&str>) -> String {
    let Some(matching) = matching.filter(|value| !value.trim().is_empty()) else { return format_scene(scene, full) };
    let needle = normalized(matching);
    let nodes = scene.nodes.iter().filter(|node| semantic_labels(&node.label).iter().any(|label| label.contains(&needle))).cloned().collect::<Vec<_>>();
    if nodes.is_empty() {
        return format!("No matching screen item was found for {}. Read screen for the current view.", display_n(matching, 64));
    }
    let mut focused = scene.clone();
    focused.nodes = nodes.into_boxed_slice();
    format_scene(&focused, full)
}

pub(super) fn format_semantic_rows(rows: &[SemanticRow], full: bool, focus: &str) -> String {
    if rows.is_empty() {
        return if focus.is_empty() {
            "The screen has no readable controls. Read screen full after the view changes.".into()
        } else {
            format!("No matching screen item was found for {}. Read screen for the current view.", display_n(focus, 64))
        };
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows.iter().filter(|row| row.kind != "heading") {
        *counts.entry(normalized(&row.label)).or_default() += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut lines = Vec::new();
    for row in rows.iter().take(if full { 256 } else { 64 }) {
        let line = semantic_line(row);
        if line.is_empty() {
            continue;
        }
        if row.kind == "heading" {
            lines.push(line);
        } else {
            let key = normalized(&row.label);
            let number = if counts.get(&key).copied().unwrap_or(0) > 1 {
                let next = seen.entry(key).or_default();
                *next += 1;
                Some(*next)
            } else {
                None
            };
            lines.push(number.map_or(line.clone(), |number| format!("{number} {line}")));
        }
    }
    let omitted = rows.len().saturating_sub(lines.len());
    let mut text = lines.join("\n");
    if omitted > 0 {
        text.push_str(&format!("\n{} more screen items were omitted.", omitted));
    }
    bounded_output_with_tail(
        text,
        if full { 2400 } else { 480 },
        if focus.is_empty() { "\nRead screen full for the complete control list." } else { "\nRead screen for a broader match." },
    )
}

pub(super) fn semantic_delta(previous: &[SemanticRow], current: &[SemanticRow]) -> Vec<SemanticRow> {
    current.iter().filter(|row| !previous.contains(row)).cloned().collect()
}

pub(super) fn format_semantic_delta(rows: &[SemanticRow]) -> String {
    if rows.is_empty() {
        return "No semantic screen changes.".into();
    }
    let lines = rows.iter().map(semantic_line).filter(|line| !line.is_empty()).collect::<Vec<_>>();
    bounded_output_with_tail(lines.join("\n"), 480, "\nRead screen for the complete current state.")
}

pub(super) fn semantic_line(row: &SemanticRow) -> String {
    let label = display_n(&row.label, 96);
    if label.is_empty() {
        return String::new();
    }
    if row.kind == "heading" {
        return label;
    }
    let value = display_n(&row.value, 128);
    let state = display_n(&row.state, 32);
    let disabled = (!row.enabled).then_some("disabled");
    let selected = row.selected.then_some("selected");
    let suffix = match row.kind.as_str() {
        "switch" => {
            let state = match state.as_str() {
                "checked" | "on" => "on",
                _ => "off",
            };
            let mut parts = vec![state];
            if let Some(word) = disabled {
                parts.push(word);
            }
            parts.push("switch");
            parts.join(" ")
        }
        "checkbox" => {
            let mut parts = vec![if state == "checked" { "checked" } else { "unchecked" }];
            if let Some(word) = disabled {
                parts.push(word);
            }
            parts.push("checkbox");
            parts.join(" ")
        }
        "radio" => {
            let mut parts = Vec::new();
            if row.selected || state == "checked" {
                parts.push("selected");
            }
            if let Some(word) = disabled {
                parts.push(word);
            }
            parts.push("option");
            parts.join(" ")
        }
        "text field" => {
            let mut parts = vec![if value.is_empty() { "empty" } else { value.as_str() }];
            if let Some(word) = disabled {
                parts.push(word);
            }
            parts.push("text field");
            parts.join(" ")
        }
        "slider" => {
            let mut parts = Vec::new();
            if !value.is_empty() {
                parts.push(value.as_str());
            }
            if let Some(word) = disabled {
                parts.push(word);
            }
            parts.push("slider");
            parts.join(" ")
        }
        "scroll area" => {
            let mut parts = Vec::new();
            if !value.is_empty() {
                parts.push(value.as_str());
            }
            if let Some(word) = disabled {
                parts.push(word);
            }
            parts.push("scroll area");
            parts.join(" ")
        }
        "tab" if row.selected => "selected tab".into(),
        "tab" => "tab".into(),
        "button" => {
            let mut parts = Vec::new();
            if !value.is_empty() {
                parts.push(value.as_str());
            }
            if let Some(word) = disabled {
                parts.push(word);
            }
            parts.push("button");
            parts.join(" ")
        }
        "link" => {
            let mut parts = Vec::new();
            if !value.is_empty() {
                parts.push(value.as_str());
            }
            if let Some(word) = disabled {
                parts.push(word);
            }
            parts.push("link");
            parts.join(" ")
        }
        _ => {
            let mut parts = Vec::new();
            if !value.is_empty() {
                parts.push(value.as_str());
            }
            if !state.is_empty() && value.is_empty() {
                parts.push(state.as_str());
            }
            if let Some(word) = disabled {
                parts.push(word);
            }
            if let Some(word) = selected {
                parts.push(word);
            }
            parts.join(" ")
        }
    };
    if suffix.is_empty() {
        label
    } else {
        format!("{label} — {suffix}")
    }
}

pub(super) fn format_scene(scene: &Scene, full: bool) -> String {
    let useful: Vec<_> = scene.nodes.iter().filter(|node| !node.label.is_empty() && actionable_node(node)).collect();
    if useful.is_empty() {
        return "The screen has no readable controls. Use screen full after the view changes.".into();
    }
    let count = useful.len();
    let limit = if full { 256 } else { 20 };
    let mut entries = Vec::new();
    for node in useful.into_iter().take(limit) {
        let mut state = match node.role as char {
            'c' => {
                if node.flags & 4 != 0 {
                    "checked"
                } else {
                    "unchecked"
                }
            }
            's' => {
                if node.flags & 8 != 0 {
                    "scrollable"
                } else {
                    "not scrollable"
                }
            }
            _ => {
                if node.flags & 2 != 0 {
                    "enabled"
                } else {
                    "disabled"
                }
            }
        };
        if node.role as char == 'i' && node.label.is_empty() {
            state = "empty";
        }
        entries.push(format!("{} is an {} {}", display_node_label(&node.label), state, role_name(node.role)));
    }
    let suffix = if count > limit { format!("; {} more controls were omitted", count - limit) } else { String::new() };
    bounded_output_with_tail(
        format!("The screen has {} useful controls. {}{}.", count, entries.join("; "), suffix),
        if full { 2400 } else { 480 },
        " Read screen full for the complete control list.",
    )
}

pub(super) fn format_browser_tabs(value: &Value) -> String {
    let tabs = value.get("tabs").and_then(Value::as_array).cloned().unwrap_or_default();
    if tabs.is_empty() {
        return "Chrome has no open tabs. Open a page to continue.".into();
    }
    let mut names = Vec::new();
    for tab in tabs.iter().take(12) {
        let title = tab.get("title").and_then(Value::as_str).unwrap_or("untitled");
        names.push(display_n(title, 80));
    }
    bounded_output(format!("Chrome has {} tabs. {}.", tabs.len(), names.join("; ")), 320)
}

pub(super) fn format_browser_page(value: &Value) -> String {
    let title = display_n(value.get("title").and_then(Value::as_str).unwrap_or("current page"), 96);
    let rows = value.get("n").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut controls = Vec::new();
    for row in rows.iter().take(20) {
        let Some(row) = row.as_array() else { continue };
        let label = row.get(1).and_then(Value::as_str).unwrap_or("");
        let role = row.get(2).and_then(Value::as_str).and_then(|s| s.as_bytes().first().copied()).unwrap_or(b'm');
        if !label.is_empty() {
            controls.push(format!("{} {}", display_n(label, 72), role_name(role)));
        }
    }
    if controls.is_empty() {
        format!("The current page is {}. It has no readable controls.", title)
    } else {
        bounded_output_with_tail(format!("The current page is {}. {}.", title, controls.join("; ")), 480, " Read page for the complete control list.")
    }
}

pub(super) fn format_page_text(value: &Value, matching: bool) -> String {
    let text = display(value.get("text").and_then(Value::as_str).unwrap_or(""));
    if text.is_empty() && matching {
        return "No matching page text was found. Read page text for a broader view.".into();
    }
    if text.is_empty() {
        return "The page has no readable text. Read the page again after it loads.".into();
    }
    let excerpt: String = text.chars().take(if matching { 260 } else { 720 }).collect();
    let omitted = text.chars().count() > excerpt.chars().count();
    bounded_output_with_tail(
        format!("Page text follows. {}{}.", excerpt, if omitted { " More text was omitted" } else { "" }),
        if matching { 480 } else { 1200 },
        " Read page text for a broader view.",
    )
}

pub(super) fn format_summary(kind: &str, value: &Value) -> String {
    if value.is_null() {
        return format!("No {} information is available.", kind.to_lowercase());
    }
    if value.is_object() {
        let keys = value.as_object().map(|object| object.keys().filter(|key| *key != "ok").take(5).map(|key| display(key)).collect::<Vec<_>>()).unwrap_or_default();
        if keys.is_empty() {
            return format!("{} information is available.", kind);
        }
        return format!("{} information is available for {}.", kind, keys.join(", "));
    }
    format!("{} information is available.", kind)
}

pub(super) fn format_notifications(value: &Value) -> String {
    let count = value.as_array().map_or(0, Vec::len);
    if count == 0 {
        "There are no actionable notifications.".into()
    } else {
        format!("There are {} actionable notifications.", count)
    }
}

pub(super) fn format_image_hash(alias: &str, value: &Value) -> String {
    let hash = display_n(value.get("hash").and_then(Value::as_str).unwrap_or("unavailable"), 96);
    bounded_output(format!("Image {} has hash {}.", display_n(alias, 64), hash), 320)
}

pub(super) fn format_image_difference(left: &str, right: &str, value: &Value) -> String {
    if value.get("changed").and_then(Value::as_u64) == Some(1) {
        format!("Images {} and {} are different.", display_n(left, 64), display_n(right, 64))
    } else {
        format!("Images {} and {} match.", display_n(left, 64), display_n(right, 64))
    }
}

pub(super) fn format_receipt(receipt: &Receipt, actions: &[Action]) -> String {
    if receipt.ok == 1 {
        let mut parts = actions
            .iter()
            .take(4)
            .map(|action| match action {
                Action::Android(AndroidAction::Tap(target)) => format!("tapped {}", display_n(&target.label, 64)),
                Action::Android(AndroidAction::Toggle(target)) => format!("toggled {}", display_n(&target.label, 64)),
                Action::Android(AndroidAction::Hold(target)) => format!("held {}", display_n(&target.label, 64)),
                Action::Android(AndroidAction::Type { text, target }) => format!("typed {} in {}", display_n(text, 64), display_n(&target.label, 64)),
                Action::Android(AndroidAction::VerifyExists(target)) => format!("verified {} exists", display_n(&target.label, 64)),
                Action::Android(AndroidAction::VerifyGone(target)) => format!("verified {} is gone", display_n(&target.label, 64)),
                Action::Android(AndroidAction::VerifyText(text)) => format!("verified that {} appeared", display_n(text, 64)),
                Action::Android(AndroidAction::Scroll { direction, .. }) => format!("scrolled {}", format!("{direction:?}").to_lowercase()),
                Action::Browser(BrowserAction::Click(target)) => format!("clicked {}", display_n(&target.label, 64)),
                Action::Browser(BrowserAction::Focus(target)) => format!("focused {}", display_n(&target.label, 64)),
                Action::Browser(BrowserAction::Type { text, target }) => format!("typed {} in {}", display_n(text, 64), display_n(&target.label, 64)),
                Action::Browser(BrowserAction::Screenshot) => "captured the page".into(),
                Action::Browser(BrowserAction::WaitText { .. } | BrowserAction::WaitCss { .. })
                | Action::Android(AndroidAction::WaitTarget { .. } | AndroidAction::WaitText { .. } | AndroidAction::WaitScreenChange { .. }) => "met the wait condition".into(),
                _ => "completed the action".into(),
            })
            .collect::<Vec<_>>();
        if parts.is_empty() {
            return "Done.".into();
        }
        let text = if parts.len() == 1 {
            format!("Done. {}.", capitalize(parts.pop().unwrap()))
        } else if parts.len() == 2 {
            format!("Done. {} and {}.", capitalize(parts.remove(0)), parts.remove(0))
        } else {
            let last = parts.pop().unwrap();
            let first = parts.remove(0);
            let middle = if parts.is_empty() { capitalize(first) } else { format!("{}, {}", capitalize(first), parts.join(", ")) };
            format!("Done. {}, and {}.", middle, last)
        };
        return bounded_output(text, 96);
    }
    match receipt.e.as_deref() {
        Some("stale") => "The screen changed before anything ran. Android Use refreshed it; retry the same command.".into(),
        Some("partial") => "Some actions completed before one failed. Read the screen before continuing.".into(),
        Some("unknown") => "The result is uncertain after dispatch. Read the screen before doing anything else.".into(),
        Some("permission") => "The required Android permission is not granted. Grant it in Android Use, then retry.".into(),
        Some("timeout") => "The action timed out before it ran. Retry the same command.".into(),
        Some("ambiguous") => "The target is ambiguous. Use the numbered command shown for the candidates.".into(),
        Some(other) => format!("The action failed because {}. Read the screen, then retry if it is safe.", display(other)),
        None => "The action failed before completion. Read the screen before continuing.".into(),
    }
}

pub(super) fn plain_error_model(error: &Error) -> String {
    match error.code {
        Code::Args | Code::Bounds | Code::Unsupported => {
            let message = display(&error.message);
            if message.contains("Use ") {
                bounded_output(message, 144)
            } else {
                bounded_output(format!("The command needs a correction. {}.", message), 144)
            }
        }
        Code::Permission => "Accessibility control is off. Enable Android Use in system settings, then retry.".into(),
        Code::Ambiguous => bounded_output(display(&error.message), 144),
        Code::Artifact => bounded_output(format!("{}. Capture a screen before using an image alias.", display(&error.message)), 144),
        Code::Stale => "The screen changed before anything ran. Android Use refreshed it; retry the same command.".into(),
        Code::Partial => "Some actions completed before one failed. Read the screen before continuing.".into(),
        Code::Unknown => "The result is uncertain after dispatch. Read the screen before doing anything else.".into(),
        _ => "Android Use could not complete that safely. Read the screen and retry when the connection is ready.".into(),
    }
}

pub(super) fn capitalize(value: String) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else { return value };
    first.to_uppercase().collect::<String>() + chars.as_str()
}

pub(super) fn display_n(value: &str, max_chars: usize) -> String {
    let mut out = display(value);
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
    }
    out
}

pub(super) fn bounded_output(value: String, max_bytes: usize) -> String {
    bounded_output_with_tail(value, max_bytes, " More text was omitted.")
}

pub(super) fn bounded_output_with_tail(value: String, max_bytes: usize, tail: &str) -> String {
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

pub(super) fn image_alias(actions: &[Action]) -> &'static str {
    for action in actions {
        if let Action::Android(value) = action {
            return match value {
                AndroidAction::Camera { .. } => "photo",
                AndroidAction::Microphone(_) => "audio",
                AndroidAction::ScreenRecord(_) => "recording",
                _ => "screen",
            };
        }
    }
    "page screenshot"
}

pub(super) fn mime_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") {
        "audio/wav"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_text_is_plain_and_bounded() {
        let scene = Scene {
            observation: "1".into(),
            generation: 1,
            package: "fixture".into(),
            nodes: vec![
                crate::api::Node { id: 0, label: "Save".into(), role: b'b', flags: 3 },
                crate::api::Node { id: 1, label: "Name".into(), role: b'i', flags: 2 },
                crate::api::Node { id: 2, label: "com.android:id/internal_text".into(), role: b't', flags: 2 },
                crate::api::Node { id: 3, label: "com.android:id/submit_button".into(), role: b'b', flags: 3 },
            ]
            .into_boxed_slice(),
        };
        let text = format_scene(&scene, false);
        assert!(!text.contains(['{', '}', '[', ']']));
        assert!(!text.contains("com.android"));
        assert!(text.contains("submit button"));
        assert!(text.len() <= 480);

        let actions = vec![
            Action::Android(AndroidAction::Type { text: "Sample text".into(), target: Target { label: "Name".into(), ordinal: None } }),
            Action::Android(AndroidAction::Tap(Target { label: "Save".into(), ordinal: None })),
            Action::Android(AndroidAction::VerifyText("Submitted".into())),
        ];
        let receipt = Receipt { id: "internal".into(), ok: 1, g: 1, m: 2, at: None, e: None, partial: None, next: None, artifact: None };
        let receipt_text = format_receipt(&receipt, &actions);
        assert_eq!(receipt_text, "Done. Typed Sample text in Name, tapped Save, and verified that Submitted appeared.");
        assert!(receipt_text.len() <= 96);
    }

    #[test]
    fn model_errors_hide_wire_syntax() {
        let error = Error::new(Code::Ambiguous, "Save matches two controls. Use tap \"Save\" number 1 or number 2.");
        let text = plain_error_model(&error);
        assert!(!text.contains(['{', '}', '[', ']']));
        assert!(text.len() <= 144);
    }
}
