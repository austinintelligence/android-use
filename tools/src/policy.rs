//! `cargo xtask workflow-policy`: the rules every `.github/workflows/*.yml` must keep. It reads the
//! files as text (the workflows are plain YAML), so a violation is reported with its file and line.
use crate::ioe;
use std::{fs, path::Path};

const UNTRUSTED: &[&str] = &["head_ref", "title", "body", "head.ref", "head.label", "head_commit.message", "head_branch", "workflow_run.", "tojson(github.event"];
const LOCKED: &[&str] = &["build", "b", "test", "t", "run", "r", "install", "clippy", "check", "c", "bench", "doc", "deny"];
const PR_WRITE: &[&str] = &["contents: write", "id-token: write", "packages: write", "actions: write", "pull-requests: write"];

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
    let mut scan: Vec<String> = lines.iter().map(|l| uncommented(l)).collect();
    for i in 0..scan.len() {
        while scan[i].ends_with('\\') && i + 1 < scan.len() {
            let next = std::mem::take(&mut scan[i + 1]);
            scan[i].pop();
            scan[i].push_str(&format!(" {next}"));
        }
        while scan[i].ends_with('|') && i + 1 < scan.len() {
            let next = std::mem::take(&mut scan[i + 1]);
            scan[i].push_str(&format!(" {next}"));
        }
    }
    let on = top_level(&lines, "on:");
    let pr = on.contains("pull_request");
    for (i, line) in scan.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let line = &line.to_lowercase();
        let mut bad = |why: &str| out.push(format!("line {}: {why}", i + 1));
        let step = line.trim_start();
        if let Some(target) = step.strip_prefix("- ").unwrap_or(step).trim_start().strip_prefix("uses:").map(str::trim).filter(|t| !t.starts_with("./")) {
            let sha = target.split('@').nth(1).unwrap_or("");
            if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                bad(&format!("`uses: {target}` is not pinned to a full commit SHA"));
            }
        }
        for banned in ["pull_request_target", "workflow_run"] {
            if line.contains(banned) {
                bad(&format!("{banned} is not allowed"));
            }
        }
        if line.contains("${{") && UNTRUSTED.iter().any(|e| line.contains(e)) {
            bad("attacker-controlled text must not be expanded in a workflow; pass it through env");
        }
        let squeezed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if pr && PR_WRITE.iter().any(|w| squeezed.contains(w)) {
            bad("a pull_request workflow must not hold contents, id-token, packages, actions or pull-requests write");
        }
        if squeezed.contains("npm install") || squeezed.contains("npm i ") || squeezed.ends_with("npm i") {
            bad("use npm ci, which installs exactly what the lockfile says");
        }
        if line.contains("actions/checkout") && !scan[i + 1..].iter().take_while(|l| !l.trim_start().starts_with("- ")).any(|l| l.contains("persist-credentials: false")) {
            bad("actions/checkout needs persist-credentials: false");
        }
        if line.contains("-latest") {
            bad("runner labels must name a version, not -latest");
        }
        if line.contains("write-all") {
            bad("permissions: write-all is not allowed");
        }
        let into_shell = line.split('|').skip(1).any(|seg| seg.split_whitespace().find(|w| *w != "sudo").is_some_and(|w| ["sh", "bash", "zsh"].contains(&w)));
        if (line.contains("curl") || line.contains("wget")) && (into_shell || line.contains("<(")) {
            bad("piping a download into a shell is not allowed");
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        for (n, w) in words.iter().enumerate().filter(|(_, w)| **w == "cargo") {
            let sub = words[n + 1..].iter().find(|a| !a.starts_with(['+', '-']));
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
    if !on.contains("push:") {
        if on.contains("push") {
            out.push("`on:` lists push without limiting it to main or v* tags".into());
        }
    } else if let Some(why) = push_problem(&on) {
        out.push(why);
    }
    out
}

/// The line without a trailing ` # comment`; a `#` inside quotes is not a comment.
fn uncommented(line: &str) -> String {
    let quotes = |s: &str| s.matches(['"', '\'']).count();
    let cut = line.match_indices('#').find(|(at, _)| (*at == 0 || line[..*at].ends_with(' ')) && quotes(&line[..*at]) % 2 == 0);
    cut.map_or(line, |(at, _)| &line[..at]).trim_end().to_owned()
}

/// A top-level key's value and indented body, one line each, comments dropped.
fn top_level(lines: &[&str], key: &str) -> String {
    let mut body = String::new();
    let mut inside = false;
    for l in lines {
        if l.replace(['"', '\''], "").replace(" :", ":").starts_with(key) {
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
    const OLD_CI: &str = "name: CI\non: [push, pull_request]\npermissions: { contents: read }\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - run: cargo test --workspace\n      - run: cargo -q build --release\n      - run: |\n          echo \"x #\"; cargo install cargo-audit\n          curl -fsSL https://x.example/i.sh \\\n            | sh\n      - run: npm install\n      - run: echo \"${{ github.event.pull_request.title }}\"\n      - run: cargo install cargo-audit --locked\n";
    const OLD_SECURITY: &str = "on:\n  pull_request_target:\n  push:\n    branches: [main, 'claude/*']\npermissions:\n  contents: read\n  security-events: write\n";
    const FIXED: &str = "name: CI\non:\n  push: { branches: [main], tags: ['v*'] }\n  pull_request:\npermissions: { contents: read }\njobs:\n  a:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4\n        with: { persist-credentials: false }\n      - run: cargo +1.94.0 test --locked --workspace\n      - run: cargo install --locked --version 0.22.2 cargo-audit\n      - run: cargo fmt --all -- --check\n";

    fn mentions(problems: &[String], what: &str) -> bool {
        problems.iter().any(|p| p.contains(what))
    }

    #[test]
    fn the_old_workflows_fail_each_rule() {
        let ci = check(OLD_CI);
        for what in ["full commit SHA", "-latest", "--locked", "--version", "download into a shell", "without limiting", "npm ci", "persist-credentials", "attacker-controlled"] {
            assert!(mentions(&ci, what), "{what}: {ci:?}");
        }
        let security = check(OLD_SECURITY);
        for what in ["pull_request_target", "claude/*", "read-only"] {
            assert!(mentions(&security, what), "{what}: {security:?}");
        }
        assert!(mentions(&check("on: push\njobs: {}\n"), "no top-level permissions"));
        assert!(mentions(&check("\"on\":\n  workflow_run:\n  push: { branches: [dev] }\n"), "workflow_run is not allowed"));
        assert!(mentions(&check("\"on\":\n  push: { branches: [dev] }\n"), "not `dev`"));
        assert!(mentions(&check("\"on\" :\n  push: { branches: [dev] }\n"), "not `dev`"));
        let pr = "permissions: { contents: read }\non: pull_request\njobs:\n  a:\n    permissions: { contents: write }\n    runs-on: ubuntu-24.04\n";
        assert!(mentions(&check(pr), "pull_request workflow must not hold"));
        for sneaky in [
            "echo ${{ GITHUB.HEAD_REF }}",
            "echo ${{ github.event.pull_request['title'] }}",
            "echo ${{ toJson(github.event) }}",
            "cargo b --release",
            "curl -fsSL x.example | sudo sh",
            "curl -fsSL x.example |\n            bash",
            "bash <(curl -fsSL x.example)",
            "npm i",
        ] {
            let text = format!("permissions: {{ contents: read }}\njobs:\n  a:\n    steps:\n      - run: {sneaky}\n");
            assert!(!check(&text).is_empty(), "{sneaky}");
        }
    }

    #[test]
    fn a_compliant_workflow_passes() {
        assert_eq!(check(FIXED), Vec::<String>::new());
        let statuses = FIXED.replace("steps:", "permissions:\n      statuses: write\n    steps:");
        assert_eq!(check(&statuses), Vec::<String>::new(), "`statuses: write` is not a `uses:` key");
    }

    #[test]
    fn the_repository_workflows_pass() {
        run(&crate::root().unwrap()).unwrap();
    }
}
