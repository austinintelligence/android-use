//! `cargo xtask workflow-policy`: the rules every `.github/workflows/*.yml` must keep. It reads the
//! files as text (the workflows are plain YAML), so a violation is reported with its file and line.
use crate::ioe;
use std::{fs, path::Path};

const LOCKED: &[&str] = &["build", "test", "run", "install", "clippy", "check", "bench", "doc", "deny"];

pub(crate) fn run(root: &Path) -> Result<(), String> {
    let dir = root.join(".github/workflows");
    let mut problems = Vec::new();
    for entry in fs::read_dir(&dir).map_err(ioe)? {
        let path = entry.map_err(ioe)?.path();
        if path.extension().is_some_and(|e| e == "yml" || e == "yaml") {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            problems.extend(check(&fs::read_to_string(&path).map_err(ioe)?).into_iter().map(|p| format!("{name}: {p}")));
        }
    }
    if problems.is_empty() {
        println!("workflow policy ok");
        Ok(())
    } else {
        Err(format!("workflow policy:\n  {}", problems.join("\n  ")))
    }
}

/// Every violation in one workflow file, each prefixed with its line number.
pub(crate) fn check(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.split(" #").next().unwrap_or("").trim_end();
        if line.trim_start().starts_with('#') {
            continue;
        }
        let mut bad = |why: &str| out.push(format!("line {}: {why}", i + 1));
        if let Some(target) = line.split("uses:").nth(1).map(str::trim).filter(|t| !t.starts_with("./")) {
            let sha = target.split('@').nth(1).unwrap_or("");
            if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                bad(&format!("`uses: {target}` is not pinned to a full commit SHA"));
            }
        }
        if line.contains("pull_request_target") {
            bad("pull_request_target is not allowed");
        }
        if line.contains("-latest") {
            bad("runner labels must name a version, not -latest");
        }
        if line.contains("write-all") {
            bad("permissions: write-all is not allowed");
        }
        if (line.contains("curl") || line.contains("wget")) && ["| sh", "|sh", "| bash", "|bash"].iter().any(|p| line.contains(p)) {
            bad("piping a download into a shell is not allowed");
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        for (n, w) in words.iter().enumerate().filter(|(_, w)| **w == "cargo") {
            let sub = words[n + 1..].iter().find(|a| !a.starts_with('+'));
            if sub.is_some_and(|s| LOCKED.contains(s)) && !words.contains(&"--locked") {
                bad("cargo commands need --locked");
            }
            if sub == Some(&"install") && !words.contains(&"--version") {
                bad(&format!("`{w} install` needs --version"));
            }
        }
    }
    if !lines.iter().any(|l| l.starts_with("permissions:")) {
        out.push("no top-level permissions: block".into());
    } else if top_level(&lines, "permissions:").contains("write") {
        out.push("top-level permissions must be read-only; grant write per job".into());
    }
    let on = top_level(&lines, "on:");
    if !on.contains("push:") {
        if on.contains("push") {
            out.push("`on:` lists push without limiting it to main or v* tags".into());
        }
    } else if let Some(why) = push_problem(&on) {
        out.push(why);
    }
    out
}

/// A top-level key's value and indented body, one line each, comments dropped.
fn top_level(lines: &[&str], key: &str) -> String {
    let mut body = String::new();
    let mut inside = false;
    for l in lines {
        if l.starts_with(key) {
            inside = true;
        } else if inside && !l.starts_with([' ', '\t', '#']) && !l.is_empty() {
            break;
        }
        if inside && !l.trim_start().starts_with('#') {
            body.push_str(l.split(" #").next().unwrap_or(""));
            body.push('\n');
        }
    }
    body
}

/// Push triggers may name only `branches`/`tags`, and only `main` and `v*`.
fn push_problem(on: &str) -> Option<String> {
    let lines: Vec<&str> = on.lines().collect();
    let at = lines.iter().position(|l| l.contains("push:"))?;
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut segment = lines[at].split("push:").nth(1).unwrap_or("").to_owned();
    segment.extend(lines[at + 1..].iter().take_while(|l| indent(l) > indent(lines[at])).map(|l| format!(" {l}")));
    let cleaned = segment.replace(['{', '}', '[', ']', ',', '\'', '"'], " ").replace(" - ", " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    let mut keyed = false;
    for t in &tokens {
        match *t {
            "branches:" | "tags:" => keyed = true,
            "main" | "v*" => {}
            other => return Some(format!("push trigger may only name main and v* tags, not `{other}`")),
        }
    }
    (!keyed).then(|| "push trigger must list branches: [main] or tags: ['v*']".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The shapes in the workflows before this check existed.
    const OLD_CI: &str = "name: CI\non: [push, pull_request]\npermissions: { contents: read }\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - run: cargo test --workspace\n      - run: cargo install cargo-audit --locked\n      - run: curl -fsSL https://x.example/i.sh | sh\n";
    const OLD_SECURITY: &str = "on:\n  pull_request_target:\n  push:\n    branches: [main, 'claude/*']\npermissions:\n  contents: read\n  security-events: write\n";
    const FIXED: &str = "name: CI\non:\n  push: { branches: [main], tags: ['v*'] }\n  pull_request:\npermissions: { contents: read }\njobs:\n  a:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4\n      - run: cargo +1.94.0 test --locked --workspace\n      - run: cargo install --locked --version 0.22.2 cargo-audit\n      - run: cargo fmt --all -- --check\n";

    fn mentions(problems: &[String], what: &str) -> bool {
        problems.iter().any(|p| p.contains(what))
    }

    #[test]
    fn the_old_workflows_fail_each_rule() {
        let ci = check(OLD_CI);
        for what in ["full commit SHA", "-latest", "--locked", "--version", "download into a shell", "without limiting"] {
            assert!(mentions(&ci, what), "{what}: {ci:?}");
        }
        let security = check(OLD_SECURITY);
        for what in ["pull_request_target", "claude/*", "read-only"] {
            assert!(mentions(&security, what), "{what}: {security:?}");
        }
        assert!(mentions(&check("on: push\njobs: {}\n"), "no top-level permissions"));
    }

    #[test]
    fn a_compliant_workflow_passes() {
        assert_eq!(check(FIXED), Vec::<String>::new());
    }

    #[test]
    fn the_repository_workflows_pass() {
        run(&crate::root().unwrap()).unwrap();
    }
}
