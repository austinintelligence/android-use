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
    out.extend(jobs(&lines));
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

/// `id-token: write` only in the `release` (attest) and `npm` (trusted publishing) environments, and npm
/// publishing only in `npm`. Signing secrets live only in a job in the `release-signing` environment, and that job runs no
/// repository code: no checkout, no toolchain setup, no cargo, npm, gradle, or xtask.
fn jobs(lines: &[&str]) -> Vec<String> {
    let start = lines.iter().position(|l| l.starts_with("jobs:")).map_or(lines.len(), |n| n + 1);
    let mut blocks: Vec<(&str, String)> = Vec::new();
    for l in &lines[start..] {
        match l.strip_prefix("  ").and_then(|r| r.strip_suffix(':')).filter(|r| !r.starts_with([' ', '#'])) {
            Some(name) => blocks.push((name, String::new())),
            None => blocks.last_mut().map_or((), |(_, b)| b.push_str(&format!("{l}\n"))),
        }
    }
    let mut out = Vec::new();
    for (name, body) in blocks {
        let signing = body.contains("environment: release-signing");
        if body.contains("secrets.AUBRIDGE") && !signing {
            out.push(format!("job {name}: signing secrets belong only in a release-signing job"));
        }
        // A tripwire on the usual spellings, not a YAML parser: job-level `environment:` only, quotes and spacing ignored.
        let env = body.lines().find_map(|l| l.strip_prefix("    environment:")).map_or("", |e| e.trim().trim_matches(['\'', '"']));
        let flat = body.replace(['\'', '"'], "").split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.contains("id-token: write") && !["release", "npm"].contains(&env) {
            out.push(format!("job {name}: id-token: write belongs only in the release and npm environments"));
        }
        let publishes = |l: &str| l.contains("publish-npm.mjs") || l.split_whitespace().any(|w| w == "npm") && l.split_whitespace().any(|w| ["publish", "pub"].contains(&w));
        if body.lines().any(|l| !l.trim_start().starts_with('#') && publishes(l)) && env != "npm" {
            out.push(format!("job {name}: npm publishing belongs only in an npm-environment job"));
        }
        for banned in ["actions/checkout", "actions/setup-", "cargo ", "npm ", "gradle", "xtask"] {
            if signing && body.lines().any(|l| !l.trim_start().starts_with('#') && l.contains(banned)) {
                out.push(format!("job {name}: the signing job must not use `{banned}`"));
            }
        }
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
    fn the_signing_job_runs_no_repository_code() {
        let job = |steps: &str| format!("permissions: {{ contents: read }}\njobs:\n  sign:\n    environment: release-signing\n    steps:\n{steps}");
        let secret = "      - run: echo ${{ secrets.AUBRIDGE_KEY_PASSWORD }}\n";
        assert_eq!(check(&job(secret)), Vec::<String>::new());
        let checkout = format!("      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683\n{secret}");
        assert!(mentions(&check(&job(&checkout)), "must not use `actions/checkout`"));
        assert!(mentions(&check(&job("      - run: npm ci\n")), "must not use `npm `"));
        let leaked = "permissions: { contents: read }\njobs:\n  build:\n    steps:\n      - run: echo ${{ secrets.AUBRIDGE_KEY_PASSWORD }}\n";
        assert!(mentions(&check(leaked), "belong only in a release-signing job"));
    }

    #[test]
    fn oidc_and_npm_publishing_stay_in_their_environments() {
        let job = |env: &str, perm: &str, run: &str| {
            format!("permissions: {{ contents: read }}\njobs:\n  j:\n    environment: {env}\n    permissions:\n      {perm}\n    steps:\n      - run: {run}\n")
        };
        assert_eq!(check(&job("npm", "id-token: write", "node install/publish-npm.mjs v1.3.0")), Vec::<String>::new());
        assert!(mentions(&check(&job("none", "id-token: write", "true")), "id-token: write belongs only"));
        assert!(mentions(&check(&job("release", "contents: read", "node install/publish-npm.mjs v1")), "npm publishing belongs only"));
        assert!(mentions(&check(&job("none", "id-token:  'write'", "true")), "id-token: write belongs only"));
        assert!(mentions(&check(&job("none", "id-token: write", "npm  pub")), "npm publishing belongs only"));
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
