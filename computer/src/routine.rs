//! Routines: a command that worked, saved with its values as parameters and replayed in one call.
//! A routine is only text. Replaying it parses and runs the expanded command like any other, so
//! generation checks, the journal and the risky-word rules all still apply.
use crate::{
    api::{Code, Error, Result},
    device::{private_dir, private_file},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const MAX_NAME: usize = 48;
const MAX_PARAMS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Meta {
    /// `save routine "NAME" from last [with "VALUE" ...]`
    Save { name: String, params: Vec<String> },
    /// `run routine "NAME" [with "VALUE" ...]`
    Run { name: String, args: Vec<String> },
}

/// The quoted values in `rest`, or an error when anything else is there.
fn quoted(rest: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut chars = rest.trim().chars().peekable();
    while chars.peek().is_some() {
        if chars.next() != Some('"') {
            return Err(Error::new(Code::Args, "values must be in straight double quotes"));
        }
        let mut value = String::new();
        loop {
            match chars.next() {
                Some('"') => break,
                Some('\\') | None => return Err(Error::new(Code::Args, "a value is not closed, or uses a backslash")),
                Some(c) => value.push(c),
            }
        }
        out.push(value);
        match chars.peek() {
            None => {}
            Some(c) if c.is_whitespace() => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some(_) => return Err(Error::new(Code::Args, "quoted values must be separated by spaces")),
        }
    }
    Ok(out)
}

/// `None` when the command is not a routine command, so the normal parser handles it.
pub(crate) fn parse(command: &str) -> Option<Result<Meta>> {
    let (verb, rest) = command.trim().split_once(' ')?;
    let save = match verb.to_ascii_lowercase().as_str() {
        "save" => true,
        "run" => false,
        _ => return None,
    };
    let rest = rest.trim_start().strip_prefix("routine ")?.trim_start();
    Some(meta(rest, save))
}

fn meta(rest: &str, save: bool) -> Result<Meta> {
    let (name, tail) = quoted_prefix(rest)?;
    let tail = tail.trim();
    let tail = if save {
        let after = tail.strip_prefix("from last").ok_or_else(|| Error::new(Code::Args, r#"use: save routine "NAME" from last"#))?.trim();
        if after.is_empty() {
            ""
        } else {
            after.strip_prefix("with ").ok_or_else(|| Error::new(Code::Args, r#"use: save routine "NAME" from last with "VALUE""#))?
        }
    } else if tail.is_empty() {
        ""
    } else {
        tail.strip_prefix("with ").ok_or_else(|| Error::new(Code::Args, r#"use: run routine "NAME" with "VALUE""#))?
    };
    let list = quoted(tail)?;
    if list.len() > MAX_PARAMS {
        return Err(Error::new(Code::Bounds, "a routine takes at most 8 values"));
    }
    valid_name(&name)?;
    Ok(if save { Meta::Save { name, params: list } } else { Meta::Run { name, args: list } })
}

/// The first quoted value in `rest` and what follows it.
fn quoted_prefix(rest: &str) -> Result<(String, String)> {
    let body = rest.strip_prefix('"').ok_or_else(|| Error::new(Code::Args, "the routine name must be in straight double quotes"))?;
    let end = body.find('"').ok_or_else(|| Error::new(Code::Args, "the routine name is not closed"))?;
    Ok((body[..end].to_owned(), body[end + 1..].to_owned()))
}

fn valid_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        && name.chars().any(char::is_alphanumeric);
    if ok {
        Ok(())
    } else {
        Err(Error::new(Code::Args, "a routine name is 1 to 48 letters, digits, spaces, - or _"))
    }
}

fn slug(name: &str) -> String {
    name.to_lowercase().split_whitespace().collect::<Vec<_>>().join("-")
}

fn dir(root: &Path) -> PathBuf {
    root.join("routines")
}

/// The template for `command` once each parameter value, as a quoted literal, becomes `{N}`.
pub(crate) fn template(command: &str, params: &[String]) -> Result<String> {
    if command.contains('{') || command.contains('}') {
        return Err(Error::new(Code::Args, "the last command contains braces, which routines reserve for values"));
    }
    let mut out = command.to_owned();
    for (index, value) in params.iter().enumerate() {
        let literal = format!("\"{value}\"");
        if !out.contains(&literal) {
            return Err(Error::new(Code::Args, format!("the last command has no quoted value {literal}")));
        }
        out = out.replace(&literal, &format!("{{{}}}", index + 1));
    }
    Ok(out)
}

pub(crate) fn save(root: &Path, name: &str, command: &str, params: &[String]) -> Result<String> {
    let template = template(command, params)?;
    let dir = dir(root);
    private_dir(&dir)?;
    let path = dir.join(format!("{}.txt", slug(name)));
    let tmp = dir.join(format!(".{}.tmp", slug(name)));
    let mut file = private_file().write(true).create(true).truncate(true).open(&tmp)?;
    writeln!(file, "{template}")?;
    file.sync_all()?;
    fs::rename(&tmp, &path)?;
    let count = params.len();
    Ok(format!("Saved routine \"{name}\" with {count} value{}: {template}", if count == 1 { "" } else { "s" }))
}

/// Number of `{N}` placeholders, assuming they run 1..=n.
fn arity(template: &str) -> usize {
    (1..=MAX_PARAMS).take_while(|n| template.contains(&format!("{{{n}}}"))).count()
}

pub(crate) fn load(root: &Path, name: &str) -> Result<String> {
    let path = dir(root).join(format!("{}.txt", slug(name)));
    match fs::read_to_string(&path) {
        Ok(text) => Ok(text.trim().to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::new(Code::Missing, format!("no routine named \"{name}\"; `read routines` lists them"))),
        Err(e) => Err(e.into()),
    }
}

/// The command to run: `template` with each `{N}` replaced by the quoted argument.
pub(crate) fn expand(name: &str, template: &str, args: &[String]) -> Result<String> {
    let need = arity(template);
    if args.len() != need {
        return Err(Error::new(Code::Args, format!("routine \"{name}\" takes {need} value{}, not {}", if need == 1 { "" } else { "s" }, args.len())));
    }
    let mut out = template.to_owned();
    for (index, value) in args.iter().enumerate() {
        if value.is_empty() || value.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
            return Err(Error::new(Code::Args, "a value cannot be empty or contain quotes, backslashes or control characters"));
        }
        out = out.replace(&format!("{{{}}}", index + 1), &format!("\"{value}\""));
    }
    Ok(out)
}

pub(crate) fn list(root: &Path) -> String {
    let mut rows: Vec<String> = fs::read_dir(dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?.to_owned();
            (path.extension()? == "txt" && !stem.starts_with('.')).then(|| format!("{}: {}", stem.replace('-', " "), fs::read_to_string(&path).unwrap_or_default().trim()))
        })
        .collect();
    rows.sort_unstable();
    if rows.is_empty() {
        "No routines saved. After a command works, `save routine \"NAME\" from last`.".into()
    } else {
        rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_meta(command: &str) -> Meta {
        parse(command).expect("a routine command").expect("parses")
    }

    #[test]
    fn only_routine_commands_are_taken() {
        assert!(parse(r#"tap "Wi-Fi""#).is_none());
        assert!(parse(r#"open app "Settings""#).is_none());
        assert!(parse("save").is_none());
        assert_eq!(is_meta(r#"save routine "set timeout" from last with "30 seconds""#), Meta::Save { name: "set timeout".into(), params: vec!["30 seconds".into()] });
        assert_eq!(is_meta(r#"run routine "set timeout" with "1 minute""#), Meta::Run { name: "set timeout".into(), args: vec!["1 minute".into()] });
        assert_eq!(is_meta(r#"run routine "x""#), Meta::Run { name: "x".into(), args: vec![] });
    }

    #[test]
    fn malformed_routine_commands_say_what_to_type() {
        for bad in [
            r#"save routine "x""#,
            r#"save routine "x" from last with 30"#,
            r#"run routine x"#,
            r#"run routine "a/b""#,
            r#"run routine "x" with "a"b""#,
            r#"run routine "x" with "a\b""#,
        ] {
            assert!(parse(bad).unwrap().is_err(), "{bad}");
        }
    }

    #[test]
    fn values_become_placeholders_and_come_back() {
        let command = r#"open app "Settings" then tap "Display" then tap "30 seconds""#;
        let t = template(command, &["30 seconds".into()]).unwrap();
        assert_eq!(t, r#"open app "Settings" then tap "Display" then tap {1}"#);
        assert_eq!(expand("n", &t, &["1 minute".into()]).unwrap(), r#"open app "Settings" then tap "Display" then tap "1 minute""#);
    }

    #[test]
    fn one_value_repeated_shares_a_placeholder_and_a_missing_value_is_refused() {
        let t = template(r#"type "bob" in "To" then type "bob" in "Cc""#, &["bob".into()]).unwrap();
        assert_eq!(t, r#"type {1} in "To" then type {1} in "Cc""#);
        assert!(template(r#"tap "Save""#, &["Cancel".into()]).is_err());
        assert!(template(r#"tap "{x}""#, &[]).is_err(), "braces are reserved");
    }

    #[test]
    fn arguments_are_counted_and_cannot_inject_commands() {
        let t = r#"tap {1} then tap {2}"#;
        assert!(expand("n", t, &["a".into()]).unwrap_err().message.contains("takes 2 values, not 1"));
        for bad in ["", "x\" then tap \"Delete", "a\\b", "line\nbreak"] {
            assert!(expand("n", "tap {1}", &[bad.into()]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn save_load_and_list_use_private_files() {
        let d = tempfile::tempdir().unwrap();
        assert!(list(d.path()).starts_with("No routines saved"));
        let said = save(d.path(), "Set Timeout", r#"tap "30 seconds""#, &["30 seconds".into()]).unwrap();
        assert_eq!(said, r#"Saved routine "Set Timeout" with 1 value: tap {1}"#);
        assert_eq!(load(d.path(), "set  timeout").unwrap(), "tap {1}", "names ignore case and spacing");
        assert_eq!(list(d.path()), "set timeout: tap {1}");
        assert_eq!(load(d.path(), "other").unwrap_err().code, Code::Missing);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!((mode(d.path().join("routines")), mode(d.path().join("routines/set-timeout.txt"))), (0o700, 0o600));
        }
    }
}
