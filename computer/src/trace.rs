//! Session traces: every command a session ran and what came back, one JSON line each under
//! `AU_HOME/traces/`, so a run can be reviewed, shared as one page, or turned into a test.
//! Passwords are redacted before anything is written, and traces expire with other artifacts.
use crate::{
    api::{Code, Error, Result},
    device::{private_dir, private_file},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_TEXT: usize = 4000;

pub struct Trace {
    path: PathBuf,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Trace {
    /// A new trace file in `root/traces`; old ones are deleted first. `None` when the folder
    /// cannot be written: tracing never stops a session.
    /// `tag` (the process id) only tells the trace files of one second apart.
    pub fn open(root: &Path, tag: &str) -> Option<Self> {
        let dir = root.join("traces");
        private_dir(&dir).ok()?;
        expire(&dir);
        let id = format!("{}-{}", now_ms() / 1000, tag.chars().filter(char::is_ascii_alphanumeric).take(6).collect::<String>());
        Some(Self { path: dir.join(format!("{id}.jsonl")) })
    }

    pub fn record(&self, read: bool, command: &str, outcome: &std::result::Result<(&str, &str, bool), (&str, &str)>) {
        let (ok, status, text, image) = match outcome {
            Ok((status, text, image)) => (true, *status, *text, *image),
            Err((code, text)) => (false, *code, *text, false),
        };
        let cmd = redact(command);
        // The screen after hidden text was typed can show it again, and a code read from a
        // notification is a secret of its own: neither reply is kept.
        let text = if cmd != command {
            "[reply hidden: the command typed hidden text]"
        } else if read && command.trim_start().to_ascii_lowercase().starts_with("code from notification") {
            "[code hidden]"
        } else {
            text
        };
        let line = json!({
            "t": now_ms(),
            "tool": if read { "read" } else { "act" },
            "cmd": cmd,
            "ok": ok,
            "status": status,
            "text": text.chars().take(MAX_TEXT).collect::<String>(),
            "image": image,
        });
        if let Ok(mut file) = private_file().append(true).create(true).open(&self.path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn expire(dir: &Path) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > TTL);
        if old && entry.path().extension().is_some_and(|e| e == "jsonl") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Field names whose typed text is never written down.
fn secret_field(field: &str) -> bool {
    let field = field.to_lowercase();
    ["password", "passcode", "passphrase", "pin", "secret", "otp", "cvv", "token", "code"].iter().any(|word| field.split(|c: char| !c.is_alphanumeric()).any(|w| w == *word))
}

/// A command's words and quoted values (straight or curly double quotes), as `(quoted, range)`;
/// a quoted value's range is its inside.
fn tokens(command: &str) -> Vec<(bool, std::ops::Range<usize>)> {
    let quote = |c: char| matches!(c, '"' | '\u{201c}' | '\u{201d}');
    let mut out = Vec::new();
    let mut chars = command.char_indices().peekable();
    while let Some(&(at, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if quote(c) {
            chars.next();
            let start = at + c.len_utf8();
            let end = chars.by_ref().find(|&(_, c)| quote(c)).map_or(command.len(), |(end, _)| end);
            out.push((true, start..end));
        } else {
            let end = chars.by_ref().take_while(|&(_, c)| !c.is_whitespace()).last().map_or(at, |(i, c)| i + c.len_utf8());
            out.push((false, at..end));
        }
    }
    out
}

/// The command with typed text replaced by `***` when it goes into a secret-looking field, or into
/// a target that has no name to judge by (`e12`, none); clipboard text and the values of `run` and
/// `save routine` always.
/// Any spelling the grammar takes counts: `type` in any case, `in` or `into`, curly quotes.
pub(crate) fn redact(command: &str) -> String {
    let tokens = tokens(command);
    let word = |i: usize, w: &str| tokens.get(i).is_some_and(|(quoted, r)| !quoted && command[r.clone()].eq_ignore_ascii_case(w));
    let quoted = |i: usize| tokens.get(i).is_some_and(|(quoted, _)| *quoted);
    let routine = (word(0, "run") || word(0, "save")) && word(1, "routine");
    let mut hide = Vec::new();
    for i in 0..tokens.len() {
        if routine && i > 2 && quoted(i) {
            hide.push(tokens[i].1.clone());
        } else if (word(i, "type") || word(i, "clipboard")) && quoted(i + 1) {
            let target = if word(i + 2, "in") || word(i + 2, "into") { i + 3 } else { i + 2 };
            let secret_target = quoted(target) && secret_field(&command[tokens[target].1.clone()]);
            let unnamed = !quoted(target);
            if word(i, "clipboard") || secret_target || unnamed {
                hide.push(tokens[i + 1].1.clone());
            }
        }
    }
    let mut out = command.to_owned();
    for range in hide.into_iter().rev() {
        out.replace_range(range, "***");
    }
    out
}

pub(crate) fn dir(root: &Path) -> PathBuf {
    root.join("traces")
}

fn ids(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.path().file_stem().and_then(|s| s.to_str()).filter(|_| e.path().extension().is_some_and(|x| x == "jsonl")).map(str::to_owned))
        .collect();
    ids.sort_unstable();
    ids
}

fn entries(root: &Path, id: &str) -> Result<Vec<Value>> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(Error::new(Code::Args, "a trace id is letters, digits and -; `au trace list` shows them"));
    }
    let text = fs::read_to_string(dir(root).join(format!("{id}.jsonl"))).map_err(|_| Error::new(Code::Missing, format!("no trace {id}; `au trace list` shows them")))?;
    Ok(text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect())
}

/// The newest `n` entries of the newest trace (what `au watch` shows).
pub fn latest(root: &Path, n: usize) -> Vec<Value> {
    let Some(id) = ids(root).pop() else { return Vec::new() };
    let all = entries(root, &id).unwrap_or_default();
    all[all.len().saturating_sub(n)..].to_vec()
}

pub fn list(root: &Path) -> Value {
    let rows: Vec<Value> = ids(root).iter().map(|id| json!({"id": id, "steps": entries(root, id).map_or(0, |e| e.len())})).collect();
    json!({"ok": 1, "traces": rows})
}

pub fn show(root: &Path, id: &str) -> Result<String> {
    Ok(entries(root, id)?
        .iter()
        .enumerate()
        .map(|(n, e)| {
            format!(
                "{}. {} {} -> {}\n{}",
                n + 1,
                e["tool"].as_str().unwrap_or("?"),
                e["cmd"].as_str().unwrap_or(""),
                e["status"].as_str().unwrap_or("?"),
                e["text"].as_str().unwrap_or("").lines().map(|l| format!("   {l}")).collect::<Vec<_>>().join("\n")
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// One self-contained page: no scripts, no external files.
pub fn export_html(root: &Path, id: &str) -> Result<String> {
    let rows: String = entries(root, id)?
        .iter()
        .map(|e| {
            let class = if e["ok"] == true { "ok" } else { "bad" };
            format!(
                "<section class=\"{class}\"><h2>{} <code>{}</code> <small>{}</small></h2><pre>{}</pre></section>",
                esc(e["tool"].as_str().unwrap_or("?")),
                esc(e["cmd"].as_str().unwrap_or("")),
                esc(e["status"].as_str().unwrap_or("?")),
                esc(e["text"].as_str().unwrap_or(""))
            )
        })
        .collect();
    Ok(format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Android Use trace {id}</title><style>body{{font:15px system-ui;max-width:52rem;margin:2rem auto;padding:0 1rem}}section{{border-left:4px solid #3a7;padding:.2rem .8rem;margin:1rem 0}}section.bad{{border-color:#c33}}h2{{font-size:1rem}}small{{color:#666}}pre{{white-space:pre-wrap;background:#f4f4f4;padding:.6rem;border-radius:4px}}</style><h1>Trace {id}</h1>{rows}",
        id = esc(id)
    ))
}

/// The commands of a trace that ran to the end, as an `au test` file.
pub fn to_test(root: &Path, id: &str) -> Result<String> {
    let mut out = format!("# From trace {id}. Add `expect screen \"TEXT\"` lines after the steps that matter.\n");
    for e in entries(root, id)?.iter().filter(|e| e["tool"] == "act" && e["ok"] == true && e["status"] == "done") {
        let cmd = e["cmd"].as_str().unwrap_or("");
        if cmd.starts_with("save routine") {
            continue;
        }
        if cmd.starts_with("run routine") {
            // Routine values are redacted in traces, so the step is kept for a person to complete.
            out.push_str(&format!("# {cmd}  (a routine step: write its values back in, or its commands)\n"));
            continue;
        }
        if cmd.contains("***") {
            return Err(Error::new(Code::Args, "the trace typed redacted text, which a test cannot replay; edit that step by hand"));
        }
        out.push_str(cmd);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_in_secret_fields_and_clipboard_text_are_redacted() {
        assert_eq!(redact(r#"type "hunter2" in "Password""#), r#"type "***" in "Password""#);
        assert_eq!(redact(r#"tap "Login" then type "1234" in "PIN code" then tap "OK""#), r#"tap "Login" then type "***" in "PIN code" then tap "OK""#);
        assert_eq!(redact(r#"page type "s3cret" in "Your password""#), r#"page type "***" in "Your password""#);
        assert_eq!(redact(r#"type "hello" in "Message""#), r#"type "hello" in "Message""#);
        assert_eq!(redact(r#"type "ok" in "Pinned notes""#), r#"type "ok" in "Pinned notes""#, "whole words only");
        assert_eq!(redact(r#"set clipboard "private""#), r#"set clipboard "***""#);
        assert_eq!(redact(r#"tap "unclosed"#), r#"tap "unclosed"#);
        // Other spellings the grammar accepts are covered too.
        for (command, hidden) in [
            (r#"type "hunter2" into "Password""#, r#"type "***" into "Password""#),
            (r#"TYPE "x1" in "PIN""#, r#"TYPE "***" in "PIN""#),
            (r#"type  "x2"   in   "Password""#, r#"type  "***"   in   "Password""#),
            ("type \u{201c}x3\u{201d} in \u{201c}Password\u{201d}", "type \u{201c}***\u{201d} in \u{201c}Password\u{201d}"),
            (r#"type "x4" in e12"#, r#"type "***" in e12"#),
            (r#"run routine "login" with "hunter2" with "x""#, r#"run routine "login" with "***" with "***""#),
            (r#"save routine "login" from last with "hunter2""#, r#"save routine "login" from last with "***""#),
        ] {
            assert_eq!(redact(command), hidden, "{command}");
        }
        assert_eq!(redact(r#"type secret "bank pin" in "PIN""#), r#"type secret "bank pin" in "PIN""#, "a secret's name is not its value");
        assert_eq!(redact(r#"type "hello" into "Message""#), r#"type "hello" into "Message""#);
    }

    #[test]
    fn a_trace_is_private_listed_shown_exported_and_expires() {
        let d = tempfile::tempdir().unwrap();
        let t = Trace::open(d.path(), "a1b2c3d4").unwrap();
        t.record(false, r#"type "pw" in "PIN""#, &Ok(("done", "Done.\nPIN: pw", false)));
        t.record(false, r#"tap "Missing""#, &Err(("missing", "No such label")));
        t.record(true, "screen", &Ok(("ok", "Settings <b>x</b>", false)));
        t.record(true, r#"code from notification "Bank""#, &Ok(("ok", "Code: 482913", false)));
        // Literal `***` in the command does not turn the hiding off.
        t.record(false, r#"type "hunter2" in "Password" then tap "Missing***""#, &Err(("partial", "Partly done: Typed hunter2 in Password.")));
        let id = ids(d.path()).remove(0);
        assert_eq!(list(d.path())["traces"][0]["steps"], 5);
        let shown = show(d.path(), &id).unwrap();
        assert!(shown.contains(r#"1. act type "***" in "PIN" -> done"#) && !shown.contains("pw") && !shown.contains("482913") && !shown.contains("hunter2"), "{shown}");
        let html = export_html(d.path(), &id).unwrap();
        assert!(html.contains("&lt;b&gt;x&lt;/b&gt;") && !html.contains("<script") && html.contains("class=\"bad\""), "{html}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!((mode(dir(d.path())), mode(dir(d.path()).join(format!("{id}.jsonl")))), (0o700, 0o600));
        }
        let old = SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        fs::File::options().write(true).open(dir(d.path()).join(format!("{id}.jsonl"))).unwrap().set_modified(old).unwrap();
        Trace::open(d.path(), "zzzz");
        assert!(ids(d.path()).is_empty(), "a trace older than 7 days is deleted when the next one opens");
    }

    #[test]
    fn a_trace_becomes_a_test_but_never_replays_redacted_text() {
        let d = tempfile::tempdir().unwrap();
        let t = Trace::open(d.path(), "abc123").unwrap();
        t.record(false, r#"open app "Settings""#, &Ok(("done", "Done.", false)));
        t.record(false, r#"tap "Missing""#, &Ok(("not_found", "miss", false)));
        t.record(true, "screen", &Ok(("ok", "x", false)));
        t.record(false, r#"tap "Wi-Fi""#, &Ok(("done", "Done.", false)));
        let id = ids(d.path()).remove(0);
        assert_eq!(to_test(d.path(), &id).unwrap().lines().skip(1).collect::<Vec<_>>(), [r#"open app "Settings""#, r#"tap "Wi-Fi""#]);
        t.record(false, r#"type "pw" in "Password""#, &Ok(("done", "Done.", false)));
        assert!(to_test(d.path(), &id).is_err());
        assert!(entries(d.path(), "../x").is_err() && entries(d.path(), "nope").is_err());
    }
}
