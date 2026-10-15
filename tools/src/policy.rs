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
        // The one exception is npm itself at an exact version (trusted publishing needs npm 11.5.1 or newer).
        let pinned_npm = squeezed
            .strip_prefix("- run: npm install --global npm@")
            .is_some_and(|v| v.split('.').count() == 3 && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())));
        if !pinned_npm && (squeezed.contains("npm install") || squeezed.contains("npm i ") || squeezed.ends_with("npm i")) {
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
    out.extend(jobs(&lines));
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
/// publishing only in `npm`. Signing secrets live only in a job in the `release-signing` environment.
/// That job runs no repository code (no checkout, toolchain, or script from anywhere), uses only the
/// artifact actions, and refuses to run until the owner marks the environment ready.
fn jobs(lines: &[&str]) -> Vec<String> {
    let mut in_jobs = false;
    let mut blocks: Vec<(&str, String)> = Vec::new();
    for l in lines {
        let code = uncommented(l).to_lowercase();
        if !l.starts_with([' ', '\t', '#']) && !l.trim().is_empty() {
            in_jobs = l.starts_with("jobs:");
        } else if !in_jobs && ["secrets.", "secrets[", "tojson(secrets"].iter().any(|s| code.contains(s)) {
            return vec!["workflow-level secrets: pass them to the one job that needs them".into()];
        }
        if !in_jobs || l.starts_with("jobs:") {
            continue;
        }
        match l.strip_prefix("  ").and_then(|r| r.strip_suffix(':')).filter(|r| !r.starts_with([' ', '#'])) {
            Some(name) => blocks.push((name, String::new())),
            None => blocks.last_mut().map_or((), |(_, b)| b.push_str(&format!("{code}\n"))),
        }
    }
    let mut out = if blocks.is_empty() { vec!["no jobs found at the usual 2-space indent".to_owned()] } else { Vec::new() };
    for (name, body) in blocks {
        let signing = body.contains("environment: release-signing");
        if !signing && ["secrets.", "secrets[", "tojson(secrets", "secrets: inherit"].iter().any(|s| body.contains(s)) {
            out.push(format!("job {name}: secrets belong only in a release-signing job, and by name"));
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
        if !signing {
            continue;
        }
        for banned in [
            "actions/checkout",
            "actions/setup-",
            "cargo ",
            "npm ",
            "gradle",
            "xtask",
            "git ",
            "curl",
            "wget",
            " ./",
            "docker",
            "| sh",
            "| bash",
            "python",
            "node ",
            "perl",
            "eval ",
        ] {
            if body.contains(banned) {
                out.push(format!("job {name}: the signing job must not use `{banned}`"));
            }
        }
        for used in body.lines().filter_map(|l| l.split("uses:").nth(1)).map(str::trim) {
            if !used.starts_with("actions/download-artifact@") && !used.starts_with("actions/upload-artifact@") {
                out.push(format!("job {name}: the signing job may only use the artifact actions, not `{used}`"));
            }
        }
        if !body.contains("release_signing_ready") {
            out.push(format!("job {name}: the signing job must refuse to run until vars.RELEASE_SIGNING_READY is set"));
        }
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
    fn the_signing_job_runs_no_repository_code() {
        let job = |steps: &str| {
            format!("permissions: {{ contents: read }}\njobs:\n  sign:\n    environment: release-signing\n    steps:\n      - run: test \"$READY\" = ${{ vars.RELEASE_SIGNING_READY }}\n{steps}")
        };
        let secret = "      - run: echo ${{ secrets.AUBRIDGE_KEY_PASSWORD }}\n";
        assert_eq!(check(&job(secret)), Vec::<String>::new());
        let checkout = format!("      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683\n        with: {{ persist-credentials: false }}\n{secret}");
        assert!(mentions(&check(&job(&checkout)), "must not use `actions/checkout`"));
        assert!(mentions(&check(&job("      - run: npm ci\n")), "must not use `npm `"));
        assert!(mentions(&check(&job("      - run: git clone https://x.example/y && ./y/build.sh\n")), "must not use `git `"));
        let other = "      - uses: acme/sign@11bd71901bbe5b1630ceea73d27597364c9af683\n";
        assert!(mentions(&check(&job(other)), "only use the artifact actions"));
        let unready = "permissions: { contents: read }\njobs:\n  sign:\n    environment: release-signing\n    steps:\n      - run: true\n";
        assert!(mentions(&check(unready), "RELEASE_SIGNING_READY"));
        assert!(mentions(&check("permissions: { contents: read }\n    jobs:\n      a: {}\n"), "no jobs found"));
        assert!(mentions(&check("permissions: { contents: read }\nenv:\n  K: ${{ SECRETS.K }}\njobs:\n  a:\n    steps: []\n"), "workflow-level secrets"));
        assert!(mentions(&check(&job("      - run: gh api x | bash\n")), "must not use `| bash`"));
        let late = "permissions: { contents: read }\njobs:\n  sign:\n    environment: release-signing\n    steps:\n      - run: echo ${{ vars.RELEASE_SIGNING_READY }}\nenv:\n  K: ${{ secrets.AUBRIDGE_KEY_PASSWORD }}\n";
        assert!(mentions(&check(late), "workflow-level secrets"));
        let statuses = "permissions: { contents: read }\njobs:\n  a:\n    permissions:\n      statuses: write\n    steps:\n      - uses: acme/x@v1\n";
        let found = check(statuses);
        assert!(found.len() == 1 && mentions(&found, "acme/x@v1"), "{found:?}");
        for leak in ["${{ secrets.AUBRIDGE_KEY_PASSWORD }}", "${{ secrets['K'] }}", "${{ toJson(secrets) }}"] {
            let leaked = format!("permissions: {{ contents: read }}\njobs:\n  build:\n    steps:\n      - run: echo {leak}\n");
            assert!(mentions(&check(&leaked), "belong only in a release-signing job"), "{leak}");
        }
    }

    #[test]
    fn only_an_exactly_pinned_npm_may_be_installed() {
        let run = |cmd: &str| check(&format!("permissions: {{ contents: read }}\njobs:\n  a:\n    steps:\n      - run: {cmd}\n"));
        assert_eq!(run("npm install --global npm@11.5.1"), Vec::<String>::new());
        for loose in ["npm install --global npm@11", "npm install --global npm@latest", "npm install --global npm@11.5.1 left-pad", "npm install"] {
            assert!(mentions(&run(loose), "use npm ci"), "{loose}");
        }
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
        let statuses = FIXED.replace("steps:", "permissions:\n      statuses: write\n    steps:");
        assert_eq!(check(&statuses), Vec::<String>::new(), "`statuses: write` is not a `uses:` key");
    }

    #[test]
    fn the_repository_workflows_pass() {
        run(&crate::root().unwrap()).unwrap();
    }
}
