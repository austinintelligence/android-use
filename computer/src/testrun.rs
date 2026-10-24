//! `au test FILE...`: end-to-end checks written in the command grammar. A line is an act command,
//! `reset app "PKG"`, `clear app "PKG"`, `expect screen "TEXT" [within N seconds]`,
//! `expect no screen "TEXT"`, or `expect result "TEXT"` (the previous act's reply). No model is involved.
use crate::api::{Code, Error, Result};
use std::{
    fmt::Write as _,
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// What a test needs from a device; `Engine` is the real one.
pub trait Driver {
    fn act(&mut self, command: &str) -> Result<String>;
    fn screen(&mut self) -> Result<String>;
    /// `adb shell` with these arguments (`am force-stop PKG`, `pm clear PKG`).
    fn shell(&mut self, args: &[&str]) -> Result<()>;
}

/// The enrolled device, driven through the same session layer an agent uses.
pub struct EngineDriver(pub crate::engine::Engine);

impl Driver for EngineDriver {
    fn act(&mut self, command: &str) -> Result<String> {
        crate::session::call(&mut self.0, false, command, None).map(|r| r.text).map_err(|e| Error::plain(e.code, crate::engine::plain_error(&e)))
    }
    fn screen(&mut self) -> Result<String> {
        crate::session::call(&mut self.0, true, "screen", None).map(|r| r.text).map_err(|e| Error::plain(e.code, crate::engine::plain_error(&e)))
    }
    fn shell(&mut self, args: &[&str]) -> Result<()> {
        let mut all = vec!["shell"];
        all.extend_from_slice(args);
        self.0.adb(&all, Duration::from_secs(20)).map(|_| ())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub name: String,
    /// `None` when the file passed; otherwise the failing line, its number and why.
    pub failure: Option<Failure>,
    pub steps: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub line: usize,
    pub text: String,
    pub why: String,
    pub screen: String,
}

fn quoted(rest: &str) -> Option<(&str, &str)> {
    let body = rest.trim().strip_prefix('"')?;
    let end = body.find('"')?;
    Some((&body[..end], body[end + 1..].trim()))
}

/// `expect screen "X" within 8 seconds`: the seconds, 5 when absent.
fn within(tail: &str) -> Option<u64> {
    if tail.is_empty() {
        return Some(5);
    }
    let words: Vec<&str> = tail.split_whitespace().collect();
    match words.as_slice() {
        ["within", n, "seconds" | "second"] => n.parse::<u64>().ok().filter(|n| (1..=60).contains(n)),
        _ => None,
    }
}

fn expect_screen(driver: &mut dyn Driver, text: &str, present: bool, seconds: u64) -> std::result::Result<(), String> {
    let end = Instant::now() + Duration::from_secs(seconds);
    loop {
        let screen = driver.screen().map_err(|e| e.message)?;
        if screen.contains(text) == present {
            return Ok(());
        }
        if Instant::now() >= end {
            return Err(format!("the screen {} \"{text}\"", if present { "never showed" } else { "still shows" }));
        }
        thread::sleep(Duration::from_millis(400));
    }
}

fn step(driver: &mut dyn Driver, line: &str, last: &mut String) -> std::result::Result<(), String> {
    let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
    let package = |rest: &str| {
        quoted(rest)
            .filter(|(_, tail)| tail.is_empty())
            .map(|(p, _)| p.to_owned())
            .filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_'))
            .ok_or_else(|| "use: reset app \"com.example.app\"".to_owned())
    };
    match (verb, rest.split_once(' ').unwrap_or((rest, ""))) {
        ("reset", ("app", r)) => driver.shell(&["am", "force-stop", &package(r)?]).map_err(|e| e.message),
        ("clear", ("app", r)) => driver.shell(&["pm", "clear", &package(r)?]).map_err(|e| e.message),
        ("expect", ("screen", r)) => {
            let (text, tail) = quoted(r).ok_or("use: expect screen \"TEXT\"")?;
            expect_screen(driver, text, true, within(tail).ok_or("use: expect screen \"TEXT\" within N seconds (1 to 60)")?)
        }
        ("expect", ("no", r)) => {
            let (text, tail) = quoted(r.strip_prefix("screen").unwrap_or(r)).ok_or("use: expect no screen \"TEXT\"")?;
            expect_screen(driver, text, false, within(tail).ok_or("use: expect no screen \"TEXT\" within N seconds")?)
        }
        ("expect", ("result", r)) => {
            let (text, tail) = quoted(r).filter(|(_, t)| t.is_empty()).ok_or("use: expect result \"TEXT\"")?;
            let _ = tail;
            if last.contains(text) {
                Ok(())
            } else {
                Err(format!("the last reply did not contain \"{text}\": {}", last.lines().next().unwrap_or("")))
            }
        }
        ("expect", _) => Err("expect is followed by screen, no screen, or result".into()),
        _ => {
            *last = driver.act(line).map_err(|e| e.message)?;
            Ok(())
        }
    }
}

/// Runs one file's text. Stops at the first failing line, with the screen as it was.
pub fn run_source(driver: &mut dyn Driver, name: &str, source: &str) -> Outcome {
    let (mut steps, mut last) = (0, String::new());
    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        steps += 1;
        if let Err(why) = step(driver, line, &mut last) {
            let screen = driver.screen().unwrap_or_default().chars().take(1500).collect();
            return Outcome { name: name.to_owned(), failure: Some(Failure { line: index + 1, text: line.to_owned(), why, screen }), steps };
        }
    }
    Outcome { name: name.to_owned(), failure: None, steps }
}

fn xml(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect::<String>().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// JUnit XML, one test case per file.
pub fn junit(outcomes: &[Outcome]) -> String {
    let failures = outcomes.iter().filter(|o| o.failure.is_some()).count();
    let mut out = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"android-use\" tests=\"{}\" failures=\"{failures}\">\n", outcomes.len());
    for o in outcomes {
        match &o.failure {
            None => {
                let _ = writeln!(out, "  <testcase name=\"{}\" classname=\"au\"/>", xml(&o.name));
            }
            Some(f) => {
                let _ = writeln!(
                    out,
                    "  <testcase name=\"{}\" classname=\"au\"><failure message=\"line {}: {}\">{}\nscreen:\n{}</failure></testcase>",
                    xml(&o.name),
                    f.line,
                    xml(&f.why),
                    xml(&f.text),
                    xml(&f.screen)
                );
            }
        }
    }
    out.push_str("</testsuite>\n");
    out
}

/// Runs each file in turn; files after a failure still run.
pub fn run_files(driver: &mut dyn Driver, files: &[String]) -> Result<Vec<Outcome>> {
    if files.is_empty() {
        return Err(Error::new(Code::Args, "use: au test FILE... [--junit PATH]"));
    }
    files
        .iter()
        .map(|file| {
            let source = std::fs::read_to_string(file).map_err(|e| Error::new(Code::Args, format!("cannot read {file}: {e}")))?;
            let name = Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or(file).to_owned();
            Ok(run_source(driver, &name, &source))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        screens: Vec<&'static str>,
        shown: usize,
        log: Vec<String>,
    }

    impl Driver for Fake {
        fn act(&mut self, command: &str) -> Result<String> {
            self.log.push(command.to_owned());
            if command.contains("Missing") {
                return Err(Error::plain(Code::Missing, "No label Missing"));
            }
            self.shown = (self.shown + 1).min(self.screens.len() - 1);
            Ok(format!("Done. {command}"))
        }
        fn screen(&mut self) -> Result<String> {
            Ok(self.screens[self.shown].to_owned())
        }
        fn shell(&mut self, args: &[&str]) -> Result<()> {
            self.log.push(format!("adb {}", args.join(" ")));
            Ok(())
        }
    }

    fn fake() -> Fake {
        Fake { screens: vec!["Home", "Settings\nWi-Fi"], shown: 0, log: vec![] }
    }

    #[test]
    fn a_passing_file_runs_every_line_and_skips_comments() {
        let mut d = fake();
        let out = run_source(
            &mut d,
            "t",
            "# a test\nreset app \"com.example.app\"\n\nopen app \"Settings\"\nexpect screen \"Wi-Fi\"\nexpect no screen \"Home\"\nexpect result \"Done.\"\n",
        );
        assert_eq!(out, Outcome { name: "t".into(), failure: None, steps: 5 });
        assert_eq!(d.log, ["adb am force-stop com.example.app", r#"open app "Settings""#]);
    }

    #[test]
    fn a_failing_line_reports_its_number_and_the_screen() {
        let mut d = fake();
        let out = run_source(&mut d, "t", "open app \"Settings\"\ntap \"Missing\"\ntap \"Never\"");
        let f = out.failure.unwrap();
        assert_eq!((f.line, f.text.as_str(), f.why.as_str(), f.screen.as_str()), (2, r#"tap "Missing""#, "No label Missing", "Settings\nWi-Fi"));
        assert_eq!(d.log.len(), 2, "nothing after the failing line runs");
    }

    #[test]
    fn expects_wait_up_to_their_window_and_then_fail_naming_the_text() {
        let mut d = fake();
        let started = Instant::now();
        let out = run_source(&mut d, "t", "expect screen \"Wi-Fi\" within 1 seconds");
        assert!(started.elapsed() >= Duration::from_secs(1));
        assert_eq!(out.failure.unwrap().why, "the screen never showed \"Wi-Fi\"");
    }

    #[test]
    fn malformed_lines_say_what_to_type() {
        for (line, hint) in [
            ("reset app \"bad name!\"", "reset app"),
            ("expect screen Wi-Fi", "expect screen"),
            ("expect screen \"x\" within 99 seconds", "within N seconds"),
            ("expect something", "expect is followed"),
            ("expect result \"x\" extra", "expect result"),
        ] {
            let out = run_source(&mut fake(), "t", line);
            assert!(out.failure.as_ref().is_some_and(|f| f.why.contains(hint)), "{line}: {out:?}");
        }
    }

    #[test]
    fn junit_has_one_case_per_file_and_escapes_the_failure() {
        let outcomes = vec![
            Outcome { name: "a".into(), failure: None, steps: 1 },
            Outcome { name: "b<".into(), failure: Some(Failure { line: 3, text: "tap \"X\"".into(), why: "a & b".into(), screen: "<s>".into() }), steps: 3 },
        ];
        let xml = junit(&outcomes);
        assert!(xml.contains("tests=\"2\" failures=\"1\"") && xml.contains("name=\"b&lt;\"") && xml.contains("line 3: a &amp; b") && xml.contains("&lt;s&gt;"), "{xml}");
    }

    #[test]
    fn files_after_a_failure_still_run_and_no_files_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (bad, good) = (dir.path().join("bad.au"), dir.path().join("good.au"));
        std::fs::write(&bad, "tap \"Missing\"").unwrap();
        std::fs::write(&good, "open app \"Settings\"").unwrap();
        let out = run_files(&mut fake(), &[bad.display().to_string(), good.display().to_string()]).unwrap();
        assert_eq!((out[0].name.as_str(), out[0].failure.is_some(), out[1].failure.is_none()), ("bad", true, true));
        assert!(run_files(&mut fake(), &[]).is_err());
    }
}
