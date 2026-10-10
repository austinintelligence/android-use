#![forbid(unsafe_code)]

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

mod emulator_e2e;
mod live;
mod tokens;

fn main() {
    if let Err(e) = run() {
        eprintln!("tools: {e}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), String> {
    let root = root()?;
    match env::args().nth(1).as_deref().unwrap_or("help") {
        "check" => {
            cmd(&root, "cargo", &["fmt", "--all", "--", "--check"])?;
            cmd(&root, "cargo", &["clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"])?;
            gate(&root)
        }
        "test" => {
            cmd(&root, "cargo", &["test", "--workspace", "--all-targets"])?;
            android(&root, "testDebugUnitTest")
        }
        "android" => android(&root, "assembleDebug"),
        "package" => package(&root),
        "release" => {
            verify(&root)?;
            package(&root)
        }
        "version" => version(&root),
        "release-manifest" => release_manifest(&root, env::args().nth(2).as_deref()),
        "release-notes" => release_notes(&root),
        "manifest" => manifest(&root),
        "verify" => verify(&root),
        "benchmark" => benchmark(&root),
        "benchmark-live" => live::run(&root),
        "emulator-e2e" => emulator_e2e::run(&root),
        "size" => size(&root).map(|_| ()),
        "docs" => docs(&root, env::args().nth(2).as_deref() == Some("--write")),
        "golden-capture" => tokens::capture(&root, &env::args().nth(2).ok_or("golden-capture needs a NAME")?),
        _ => {
            println!(
                "cargo xtask check|test|android|package|release|version|release-manifest DIR|release-notes|manifest|verify|benchmark|benchmark-live|emulator-e2e|size|docs [--write]|golden-capture NAME"
            );
            Ok(())
        }
    }
}
fn verify(root: &Path) -> Result<(), String> {
    version(root)?;
    cmd(root, "cargo", &["fmt", "--all", "--", "--check"])?;
    cmd(root, "cargo", &["clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"])?;
    cmd(root, "cargo", &["test", "--workspace", "--all-targets"])?;
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    cmd(root, npm, &["test"])?;
    cmd(root, npm, &["run", "lint"])?;
    android(root, "testDebugUnitTest")?;
    android(root, "assembleRelease")?;
    gate(root)?;
    docs(root, false)?;
    cmd(root, "cargo", &["build", "--profile", "dist", "-p", "au"])?;
    artifact_sizes(root)?;
    size(root).map(|_| ())
}
/// The release artifacts stay small: the helper APK, and the host binary (which carries the APK in
/// releases, so its limit has room for it).
fn artifact_sizes(root: &Path) -> Result<(), String> {
    let binary = file_bytes(&root.join("target/dist").join(if cfg!(windows) { "au.exe" } else { "au" }))?;
    let apk = file_bytes(&release_apk(root).or_else(|_| root.join("device/app/build/outputs/apk/release/app-release-unsigned.apk").canonicalize().map_err(ioe))?)?;
    // Linux (ELF) binaries come out about 30% larger than macOS and Windows ones from the same
    // source (v1.0.0: 892,536 against 688,528), so the limit is per platform.
    let max_binary = if cfg!(target_os = "linux") { 1_300_000 } else { 1_000_000 };
    println!("artifact_sizes binary_bytes={binary} apk_bytes={apk} max_binary={max_binary} max_apk=60000");
    if binary > max_binary || apk > 60_000 {
        return Err("a release artifact exceeds its size limit".into());
    }
    Ok(())
}
fn package(root: &Path) -> Result<(), String> {
    android(root, "assembleRelease")?;
    // The release binary carries the helper, so `au setup` needs nothing beside it.
    env::set_var("AU_EMBED_APK", release_apk(root)?);
    cmd(root, "cargo", &["build", "--profile", "dist", "-p", "au"])?;
    let dist = root.join("dist");
    fs::create_dir_all(&dist).map_err(ioe)?;
    let exe = if cfg!(windows) { "au.exe" } else { "au" };
    copy(&root.join("target/dist").join(exe), &dist.join(exe))?;
    let apk = release_apk(root)?;
    copy(&apk, &dist.join("aubridge.apk"))?;
    copy(&root.join("skills/android-use/SKILL.md"), &dist.join("SKILL.md"))?;
    copy(&root.join("docs/agents/install.md"), &dist.join("install.md"))?;
    copy_tree(&root.join("skills/android-use"), &dist.join("skills/android-use"))?;
    let bundle = dist.join("npm/android-use");
    copy_tree(&root.join("install"), &bundle)?;
    let bin = bundle.join("bin").join(platform_name());
    fs::create_dir_all(&bin).map_err(ioe)?;
    copy(&root.join("target/dist").join(exe), &bin.join(exe))?;
    copy(&apk, &bin.join("aubridge.apk"))?;
    let files = json!({
        format!("bin/{}/{}", platform_name(), exe): hex(&Sha256::digest(fs::read(bin.join(exe)).map_err(ioe)?)),
        format!("bin/{}/aubridge.apk", platform_name()): hex(&Sha256::digest(fs::read(bin.join("aubridge.apk")).map_err(ioe)?)),
    });
    // Published, the launcher finds the binary in the matching platform package; the repository's
    // own package.json leaves these out so npm ci works before they exist.
    let manifest_path = bundle.join("package.json");
    let mut package_json: Value = serde_json::from_slice(&fs::read(&manifest_path).map_err(ioe)?).map_err(ioe)?;
    let version = env!("CARGO_PKG_VERSION");
    let platforms = platforms(root)?;
    package_json["optionalDependencies"] = Value::Object(platforms.iter().map(|p| (format!("@androiduse/{}", p["npm"].as_str().unwrap_or_default()), json!(version))).collect());
    fs::write(&manifest_path, serde_json::to_vec_pretty(&package_json).map_err(ioe)?).map_err(ioe)?;
    fs::write(bundle.join("manifest.json"), serde_json::to_vec(&json!({"schema":1,"version":env!("CARGO_PKG_VERSION"),"files":files})).map_err(|e| e.to_string())?).map_err(ioe)?;
    manifest(root)
}
fn manifest(root: &Path) -> Result<(), String> {
    let dist = root.join("dist");
    let mut assets = serde_json::Map::new();
    for e in fs::read_dir(&dist).map_err(ioe)? {
        let p = e.map_err(ioe)?.path();
        if p.file_name().and_then(|s| s.to_str()) == Some("manifest.json") || !p.is_file() {
            continue;
        }
        let b = fs::read(&p).map_err(ioe)?;
        assets.insert(p.file_name().unwrap().to_string_lossy().into_owned(), json!({"bytes":b.len(),"sha256":hex(&Sha256::digest(&b))}));
    }
    fs::write(dist.join("manifest.json"), serde_json::to_vec(&json!({"schema":3,"version":env!("CARGO_PKG_VERSION"),"assets":assets})).map_err(|e| e.to_string())?).map_err(ioe)
}
fn benchmark(root: &Path) -> Result<(), String> {
    cmd(root, "cargo", &["build", "--release", "-p", "au"])?;
    let exe = root.join("target/release").join(if cfg!(windows) { "au.exe" } else { "au" });
    let mut samples = Vec::with_capacity(40);
    for _ in 0..40 {
        let start = Instant::now();
        let s = Command::new(&exe).arg("--version").stdout(Stdio::null()).status().map_err(ioe)?;
        if !s.success() {
            return Err("benchmark command failed".into());
        }
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    println!("startup_p50_us={}", quantile(&samples, 0.50));
    println!("startup_p95_us={}", quantile(&samples, 0.95));
    println!("startup_p99_us={}", quantile(&samples, 0.99));
    println!("binary_bytes={}", fs::metadata(&exe).map_err(ioe)?.len());
    println!("apk_bytes={}", file_bytes(&release_apk(root)?)?);
    println!("installer_bytes={}", tree_bytes(&root.join("install"))?);
    let golden: Value = serde_json::from_str(&fs::read_to_string(root.join("protocol-golden.json")).map_err(ioe)?).map_err(|e| e.to_string())?;
    for key in ["frame", "success", "stale"] {
        let bytes = serde_json::to_vec(golden.get(key).ok_or_else(|| format!("golden fixture is missing {key}"))?).map_err(|e| e.to_string())?;
        println!("{key}_bytes={}", bytes.len());
    }
    Ok(())
}

pub(crate) fn approx_tokens(bytes: usize) -> usize {
    bytes.saturating_add(3) / 4
}
fn size(root: &Path) -> Result<(usize, usize, usize), String> {
    let (rust, inline_tests) = rust_split(&root.join("computer/src"))?;
    let java = lines(&root.join("device/app/src/main"), &["java"])?;
    let automation = lines(&root.join("tools"), &["rs"])? + lines(&root.join("install"), &["mjs"])?;
    let tests = inline_tests + lines(&root.join("computer/tests"), &["rs"])? + lines(&root.join("device/app/src/test"), &["java"])? + lines(&root.join("install/test"), &["mjs"])?;
    let authored = lines(root, &["rs", "java", "mjs", "js", "ps1", "sh", "cmd", "kt"])?;
    let source_files = file_count(root, &["rs", "java", "mjs", "js", "ps1", "sh", "cmd", "kt"])?;
    let production_modules = file_count(&root.join("computer/src"), &["rs"])? + file_count(&root.join("device/app/src/main"), &["java"])?;
    println!("rust_production={rust} android_production={java} tests={tests} automation={automation} authored_code={authored} source_files={source_files} production_modules={production_modules}");
    // Budgets measure conventionally formatted code (rustfmt; google-java-format --aosp for the
    // helper), so they track real complexity rather than how densely lines are packed.
    if rust > 7520 || java > 3450 || automation > 1700 || authored > 15500 {
        return Err("source budget exceeded".into());
    }
    Ok((rust, java, automation))
}
fn gate(root: &Path) -> Result<(), String> {
    version(root)?;
    for p in ["crates", "packages", "wire", "android", "xtask", "scripts", "packaging"] {
        if root.join(p).is_dir() {
            return Err(format!("old wrapper folder remains: {p}"));
        }
    }
    for p in walk(&root.join("computer/src"))? {
        if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            let s = fs::read_to_string(&p).map_err(ioe)?;
            if s.contains("unsafe {") || s.contains("Command::new(\"sh\")") || s.contains("Command::new(\"adb\")") {
                return Err(format!("security gate failed: {}", p.display()));
            }
            // A full-tree dump is slow and bypasses the semantic compiler; reading goes through the helper.
            if s.contains("uiautomator") {
                return Err(format!("product code must not use uiautomator: {}", p.display()));
            }
            if s.lines().count() > 900 {
                return Err(format!("production file exceeds 900 lines: {}", p.display()));
            }
        }
    }
    size(root).map(|_| ())
}
fn version(root: &Path) -> Result<(), String> {
    let version = fs::read_to_string(root.join("VERSION")).map_err(ioe)?;
    let version = version.trim();
    if version.split('.').count() != 3 || version.split('.').any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit())) {
        return Err("VERSION must contain a stable numeric semantic version".into());
    }
    if version != env!("CARGO_PKG_VERSION") {
        return Err(format!("tools version {} does not match VERSION {version}", env!("CARGO_PKG_VERSION")));
    }
    for manifest in ["computer/Cargo.toml", "tools/Cargo.toml"] {
        let text = fs::read_to_string(root.join(manifest)).map_err(ioe)?;
        if !text.contains("version.workspace = true") {
            return Err(format!("{manifest} must inherit the workspace version"));
        }
    }
    let installer: Value = serde_json::from_slice(&fs::read(root.join("install/package.json")).map_err(ioe)?).map_err(|e| e.to_string())?;
    if installer.get("version").and_then(Value::as_str) != Some(version) {
        return Err("install/package.json does not match VERSION".into());
    }
    let lock: Value = serde_json::from_slice(&fs::read(root.join("package-lock.json")).map_err(ioe)?).map_err(|e| e.to_string())?;
    if lock.pointer("/packages/install/version").and_then(Value::as_str) != Some(version) {
        return Err("package-lock.json does not match VERSION".into());
    }
    let installer_cli = fs::read_to_string(root.join("install/cli.mjs")).map_err(ioe)?;
    if !installer_cli.contains(&format!("const version = \"{version}\";")) {
        return Err("install/cli.mjs does not match VERSION".into());
    }
    let gradle = fs::read_to_string(root.join("device/app/build.gradle")).map_err(ioe)?;
    if !gradle.contains("def releaseVersion = file(\"../../VERSION\").text.trim()") || !gradle.contains("versionName = releaseVersion") {
        return Err("Android helper must read its version from VERSION".into());
    }
    for path in ["README.md", "docs/guides/install.md", "install/README.md", "skills/android-use/SKILL.md"] {
        let text = fs::read_to_string(root.join(path)).map_err(ioe)?;
        if text.contains("v3") || text.contains("v2") {
            return Err(format!("stale product version marker in {path}"));
        }
    }
    platforms_agree(root)?;
    println!("version={version}");
    Ok(())
}
/// `install/platforms.json` is the one list of release platforms.
fn platforms(root: &Path) -> Result<Vec<Value>, String> {
    let list: Value = serde_json::from_slice(&fs::read(root.join("install/platforms.json")).map_err(ioe)?).map_err(|e| e.to_string())?;
    list.as_array().cloned().filter(|l| !l.is_empty()).ok_or_else(|| "install/platforms.json must be a non-empty list".into())
}
/// Everything that names a platform (the release matrix, both install scripts) agrees with that list,
/// and no install script pins a version.
fn platforms_agree(root: &Path) -> Result<(), String> {
    let release = fs::read_to_string(root.join(".github/workflows/release.yml")).map_err(ioe)?;
    let sh = fs::read_to_string(root.join("install/install.sh")).map_err(ioe)?;
    let ps1 = fs::read_to_string(root.join("install/install.ps1")).map_err(ioe)?;
    for p in platforms(root)? {
        let (asset, target, npm) = (p["asset"].as_str().unwrap_or_default(), p["target"].as_str().unwrap_or_default(), p["npm"].as_str().unwrap_or_default());
        if asset.is_empty() || target.is_empty() || npm.is_empty() {
            return Err("every platform needs npm, asset and target".into());
        }
        if !release.contains(&format!("asset: {asset}")) || !release.contains(&format!("target: {target}")) {
            return Err(format!("release.yml has no matrix entry for {asset}"));
        }
        let installer = if asset.starts_with("windows") { &ps1 } else { &sh };
        if !installer.contains(&format!("-{asset}.")) {
            return Err(format!("the install script for {asset} does not name that archive"));
        }
    }
    for (name, text) in [("install.sh", &sh), ("install.ps1", &ps1)] {
        let digit_after = |marker: &str| text.match_indices(marker).any(|(at, _)| text[at + marker.len()..].starts_with(|c: char| c.is_ascii_digit()));
        if digit_after("VERSION:-") || digit_after("else { '") {
            return Err(format!("{name} must not pin a version"));
        }
    }
    Ok(())
}
fn release_manifest(root: &Path, directory: Option<&str>) -> Result<(), String> {
    version(root)?;
    let directory = directory.map(PathBuf::from).ok_or_else(|| "release-manifest requires a directory".to_owned())?;
    let directory = if directory.is_absolute() { directory } else { root.join(directory) };
    if !directory.is_dir() {
        return Err(format!("release directory does not exist: {}", directory.display()));
    }
    let mut files = fs::read_dir(&directory).map_err(ioe)?.map(|entry| entry.map(|entry| entry.path()).map_err(ioe)).collect::<std::result::Result<Vec<_>, _>>()?;
    files.retain(|path| path.is_file() && !matches!(path.file_name().and_then(|name| name.to_str()), Some("SHA256SUMS" | "release-manifest.json")));
    files.sort();
    if files.is_empty() {
        return Err("release directory has no files to checksum".into());
    }
    let mut assets = serde_json::Map::new();
    let mut sums = String::new();
    for path in files {
        let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| "release asset name was not UTF-8".to_owned())?;
        let bytes = fs::read(&path).map_err(ioe)?;
        let digest = hex(&Sha256::digest(&bytes));
        sums.push_str(&format!("{digest}  {name}\n"));
        assets.insert(name.to_owned(), json!({"bytes":bytes.len(),"sha256":digest}));
    }
    fs::write(directory.join("SHA256SUMS"), sums).map_err(ioe)?;
    let commit = env::var("GITHUB_SHA").unwrap_or_else(|_| "local".into());
    let version = fs::read_to_string(root.join("VERSION")).map_err(ioe)?.trim().to_owned();
    let manifest = json!({"schema":1,"product":"android-use","version":version,"commit":commit,"assets":assets});
    fs::write(directory.join("release-manifest.json"), serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?).map_err(ioe)
}
fn docs(root: &Path, write: bool) -> Result<(), String> {
    let protocol = root.join("skills/android-use/references/protocol.md");
    if write {
        fs::write(&protocol, au::spec::protocol_markdown()).map_err(ioe)?;
    } else if fs::read_to_string(&protocol).map_err(ioe)? != au::spec::protocol_markdown() {
        return Err("skills/android-use/references/protocol.md is out of date; run cargo xtask docs --write".into());
    }
    tokens::gate(root)?;
    let required = [
        "README.md",
        "AGENTS.md",
        "llms.txt",
        "SECURITY.md",
        "docs/README.md",
        "docs/guides/install.md",
        "docs/agents/install.md",
        "docs/agents/quickstart.md",
        "docs/reference/cli.md",
        "docs/reference/agent-protocol.md",
        "docs/guides/troubleshooting.md",
        "docs/guides/emulators.md",
        "docs/reference/environment.md",
        "docs/reference/capabilities.md",
        "docs/concepts/token-budget.md",
        "docs/concepts/security-model.md",
        "docs/contributing/release.md",
        "docs/contributing/testing.md",
        "examples/README.md",
        "skills/android-use/SKILL.md",
        "skills/android-use/references/protocol.md",
        "skills/android-use/references/safety.md",
        "skills/android-use/references/setup.md",
    ];
    for path in required {
        if !root.join(path).is_file() {
            return Err(format!("missing document {path}"));
        }
        links(root)?;
    }
    let budgets = [
        ("skills/android-use/SKILL.md", 2800usize, 700usize),
        ("skills/android-use/references/protocol.md", 2000, 520),
        ("skills/android-use/references/safety.md", 520, 130),
        ("skills/android-use/references/setup.md", 520, 130),
        ("AGENTS.md", 1800, 450),
        ("docs/reference/agent-protocol.md", 2800, 700),
        ("docs/agents/quickstart.md", 1800, 450),
        ("docs/agents/install.md", 7200, 1800),
        ("docs/guides/workflows.md", 1400, 350),
        ("docs/reference/cli.md", 2800, 700),
        ("examples/README.md", 2000, 500),
    ];
    let mut skill_bytes = 0usize;
    for (path, max_bytes, max_tokens) in budgets {
        let bytes = fs::read(root.join(path)).map_err(ioe)?;
        let tokens = approx_tokens(bytes.len());
        println!(
            "docs_budget path={path} bytes={} approx_tokens={tokens} max_bytes={max_bytes} max_tokens={max_tokens} status={}",
            bytes.len(),
            if bytes.len() <= max_bytes && tokens <= max_tokens { "pass" } else { "fail" }
        );
        if bytes.len() > max_bytes || tokens > max_tokens {
            return Err(format!("documentation budget exceeded: {path}"));
        }
        if path.starts_with("skills/android-use/") {
            skill_bytes = skill_bytes.saturating_add(bytes.len());
        }
    }
    let skill_tokens = approx_tokens(skill_bytes);
    println!(
        "docs_budget path=skills/android-use bundle_bytes={skill_bytes} approx_tokens={skill_tokens} max_bytes=4800 max_tokens=1200 status={}",
        if skill_bytes <= 4800 && skill_tokens <= 1200 { "pass" } else { "fail" }
    );
    if skill_bytes > 4800 || skill_tokens > 1200 {
        return Err("installed skill bundle exceeds its budget".into());
    }
    for line in fs::read_to_string(root.join("llms.txt")).map_err(ioe)?.lines() {
        if line.len() > 100 {
            return Err("llms.txt contains a description over 100 bytes".into());
        }
    }
    let primary_paths = ["skills/android-use/SKILL.md", "AGENTS.md", "docs/agents/quickstart.md", "docs/guides/workflows.md", "examples/README.md"];
    let mut primary = String::new();
    for path in primary_paths {
        primary.push_str(&fs::read_to_string(root.join(path)).map_err(ioe)?);
    }
    for forbidden in ["Austin", "q=", "\"g\":", "target:\"", "\"target\":", "\"p\":", "artifact range", "integer ref"] {
        if primary.contains(forbidden) {
            return Err(format!("legacy primary-path phrase remains: {forbidden}"));
        }
    }
    for path in ["device/app/src/main/java/dev/codex/aubridge/SemanticCompiler.java", "device/app/src/main/java/dev/codex/aubridge/Ui.java"] {
        let source = fs::read_to_string(root.join(path)).map_err(ioe)?;
        for forbidden in ["Austin", "Network & internet", "Airplane mode", "Discord", "com.android.settings", "switch_widget", "VPN"] {
            if source.contains(forbidden) {
                return Err(format!("semantic compiler contains fixture-specific literal {forbidden}: {path}"));
            }
        }
    }
    println!("semantic_source_scan=pass unknown_app_literals=0 fixture_labels_in_production=0");
    let skill = fs::read_to_string(root.join("skills/android-use/SKILL.md")).map_err(ioe)?;
    for name in [au::spec::READ_TOOL, au::spec::ACT_TOOL] {
        if !skill.contains(name) {
            return Err(format!("SKILL.md must name the {name} tool"));
        }
    }
    println!("docs_examples=pass canonical_forms=pass primary_legacy_syntax=pass llms_descriptions=pass");
    if let Some(packaged) = root.join("dist/skills/android-use/SKILL.md").to_str() {
        let packaged = Path::new(packaged);
        if packaged.is_file() {
            let source_bytes = fs::read(root.join("skills/android-use/SKILL.md")).map_err(ioe)?;
            let packaged_bytes = fs::read(packaged).map_err(ioe)?;
            if source_bytes != packaged_bytes {
                return Err("packaged skill is not byte-for-byte equal to source skill".into());
            }
            println!("packaged_skill=pass");
        } else {
            println!("packaged_skill=not_built");
        }
    }
    Ok(())
}
fn android(root: &Path, task: &str) -> Result<(), String> {
    let gradle = env::var_os("AU_GRADLE")
        .map(PathBuf::from)
        .or_else(|| if cfg!(windows) { env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join("Codex/android-use/tools/gradle-9.1.0/bin/gradle.bat")) } else { None })
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "gradle.bat" } else { "gradle" }));
    let task_arg = format!(":app:{task}");
    let native_dir = env::temp_dir().join("android-use-gradle-native");
    let args = if cfg!(windows) {
        vec!["--no-daemon".to_owned(), format!("-Dorg.gradle.native.dir={}", native_dir.display()), task_arg]
    } else {
        vec!["--no-daemon".to_owned(), task_arg]
    };
    println!("+ {} {}", gradle.display(), args.join(" "));
    let mut command = Command::new(&gradle);
    command.args(&args).current_dir(root.join("device"));
    if env::var_os("ANDROID_HOME").is_none() {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            let sdk = PathBuf::from(local).join("Android/Sdk");
            command.env("ANDROID_HOME", &sdk).env("ANDROID_SDK_ROOT", sdk);
        }
    }
    if env::var_os("GRADLE_USER_HOME").is_none() {
        if let Some(user) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
            command.env("GRADLE_USER_HOME", PathBuf::from(user).join(".gradle"));
        }
    }
    let status = command.status().map_err(ioe)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", gradle.display()))
    }
}
/// Production and inline-test line counts for a Rust source tree. Unit tests
/// conventionally sit in a trailing `#[cfg(test)]` module; everything from that
/// attribute to the end of the file counts as tests, not production code.
fn rust_split(root: &Path) -> Result<(usize, usize), String> {
    let (mut production, mut tests) = (0, 0);
    for p in walk(root)? {
        if p.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&p).map_err(ioe)?;
        let all: Vec<&str> = text.lines().collect();
        // `foo_tests.rs` files are only compiled through `#[cfg(test)] #[path] mod tests`.
        let test_file = p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.ends_with("_tests"));
        let split = if test_file { 0 } else { all.iter().position(|line| line.trim() == "#[cfg(test)]" && !line.starts_with(' ')).unwrap_or(all.len()) };
        production += split;
        tests += all.len() - split;
    }
    Ok((production, tests))
}
fn lines(root: &Path, exts: &[&str]) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }
    let mut n = 0;
    for p in walk(root)? {
        if p.components().any(|c| matches!(c.as_os_str().to_str(), Some("target" | "build" | ".git" | "node_modules" | ".gradle" | "artifacts" | "dist"))) {
            continue;
        }
        if p.is_file() && exts.contains(&p.extension().and_then(|s| s.to_str()).unwrap_or("")) {
            n += fs::read_to_string(p).map_err(ioe)?.lines().count();
        }
    }
    Ok(n)
}
pub(crate) fn walk(root: &Path) -> Result<Vec<PathBuf>, String> {
    let (mut out, mut todo) = (Vec::new(), vec![root.to_path_buf()]);
    while let Some(p) = todo.pop() {
        for e in fs::read_dir(p).map_err(ioe)? {
            let p = e.map_err(ioe)?.path();
            if p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| matches!(name, "target" | "build" | ".git" | "node_modules" | ".gradle" | "artifacts" | "dist" | ".codex-run"))
            {
                continue;
            }
            if p.is_dir() {
                todo.push(p.clone())
            }
            out.push(p)
        }
    }
    Ok(out)
}
fn root() -> Result<PathBuf, String> {
    let mut p = env::current_dir().map_err(ioe)?;
    loop {
        if p.join("tools/Cargo.toml").is_file() {
            return Ok(p);
        }
        if !p.pop() {
            return Err("run inside the android-use checkout".into());
        }
    }
}
fn cmd(cwd: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    println!("+ {program} {}", args.join(" "));
    let status = Command::new(program).args(args).current_dir(cwd).status().map_err(ioe)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(ioe)?;
    }
    fs::copy(from, to).map(|_| ()).map_err(ioe)
}
fn release_apk(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("device/app/build/outputs/apk/release");
    for name in ["bridge-release.apk", "app-release.apk"] {
        let path = dir.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!("release APK was not found under {}", dir.display()))
}
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(ioe)?;
    for entry in fs::read_dir(from).map_err(ioe)? {
        let entry = entry.map_err(ioe)?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            copy(&source, &target)?;
        }
    }
    Ok(())
}
fn platform_name() -> String {
    let os = if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "ia32"
    } else {
        env::consts::ARCH
    };
    format!("{os}-{arch}")
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}
pub(crate) fn quantile(values: &[u128], percentile: f64) -> u128 {
    let index = ((values.len().saturating_sub(1)) as f64 * percentile).round() as usize;
    values[index.min(values.len().saturating_sub(1))]
}
fn file_bytes(path: &Path) -> Result<u64, String> {
    Ok(fs::metadata(path).map(|m| m.len()).unwrap_or(0))
}
fn tree_bytes(root: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    for path in walk(root)? {
        if path.is_file() {
            total = total.saturating_add(fs::metadata(path).map_err(ioe)?.len());
        }
    }
    Ok(total)
}
fn file_count(root: &Path, exts: &[&str]) -> Result<usize, String> {
    Ok(walk(root)?.into_iter().filter(|p| p.is_file() && exts.contains(&p.extension().and_then(|s| s.to_str()).unwrap_or(""))).count())
}
pub(crate) fn ioe(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Every relative link in a Markdown file points at a file or folder that exists.
fn links(root: &Path) -> Result<(), String> {
    let out = Command::new("git").args(["ls-files", "*.md"]).current_dir(root).output().map_err(ioe)?;
    for file in String::from_utf8_lossy(&out.stdout).lines().filter(|f| !f.starts_with("docs/design/baseline/") && *f != "CHANGELOG.md") {
        let text = fs::read_to_string(root.join(file)).map_err(ioe)?;
        for (at, _) in text.match_indices("](") {
            let rest = &text[at + 2..];
            let Some(end) = rest.find(')') else { continue };
            let target = rest[..end].split(['#', ' ']).next().unwrap_or_default();
            if target.is_empty() || target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            let base = root.join(file).parent().map(Path::to_path_buf).unwrap_or_default();
            if !base.join(target).exists() {
                return Err(format!("{file}: broken link {target}"));
            }
        }
    }
    println!("doc_links=pass");
    Ok(())
}

/// The body of a changelog section: from its `## VERSION` heading to the next `## ` heading.
fn changelog_section(changelog: &str, version: &str) -> Option<String> {
    let start = changelog.lines().position(|l| l.starts_with("## ") && l[3..].split([' ', '\u{2014}']).next() == Some(version))?;
    let body: Vec<&str> = changelog.lines().skip(start + 1).take_while(|l| !l.starts_with("## ")).collect();
    Some(body.join("\n").trim().to_owned())
}
/// `cargo xtask release-notes`: the GitHub release body for `VERSION`, from its changelog section.
fn release_notes(root: &Path) -> Result<(), String> {
    let version = fs::read_to_string(root.join("VERSION")).map_err(ioe)?.trim().to_owned();
    let changelog = fs::read_to_string(root.join("CHANGELOG.md")).map_err(ioe)?;
    let section = changelog_section(&changelog, &version).ok_or(format!("CHANGELOG.md has no section for {version}"))?;
    // Relative links do not work in a release body.
    let section = section.replace("](docs/", &format!("](https://github.com/austinintelligence/android-use/blob/v{version}/docs/"));
    let rows: String = platforms(root)?
        .iter()
        .map(|p| {
            format!(
                "| {} | `android-use-v{version}-{}.{}` |\n",
                p["npm"].as_str().unwrap_or_default().replace('-', " "),
                p["asset"].as_str().unwrap_or_default(),
                p["archive"].as_str().unwrap_or_default()
            )
        })
        .collect();
    print!(
        "# android-use {version}\n\n{section}\n\n## Install\n\n```sh\nnpx -y android-use setup                                  # npm\nbrew install austinintelligence/tap/android-use           # Homebrew\ncurl -fsSL https://raw.githubusercontent.com/austinintelligence/android-use/v{version}/install/install.sh | sh   # macOS, Linux\n```\n\nOn Windows run `install/install.ps1`, or unpack an archive below and run `au setup`. Every archive contains `au` with the Android helper built in; the helper is also attached on its own for audited deployment.\n\n| Platform | Archive |\n| --- | --- |\n{rows}\n## Verify\n\nCheck an archive against `SHA256SUMS`, and its provenance with `gh attestation verify FILE --repo austinintelligence/android-use`. `android-use.spdx.json` is the software bill of materials, and `release-manifest.json` lists every asset with its size and digest.\n\n## Security\n\nUSB debugging and access to `au serve` are device-control authority; keep them local and private. Android Use ships no remote broker or multi-user authorization. Read [SECURITY.md](https://github.com/austinintelligence/android-use/blob/v{version}/SECURITY.md) before exposing any transport beyond your computer. Full history: [CHANGELOG.md](https://github.com/austinintelligence/android-use/blob/v{version}/CHANGELOG.md).\n"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::changelog_section;

    #[test]
    fn a_changelog_section_stops_at_the_next_version() {
        let log = "# Changelog\n\n## 1.2.0 \u{2014} 2026-10-10\n\nNew.\n\n### Added\n\n- a\n\n## 1.1.2 \u{2014} 2026-10-09\n\nOld.\n";
        assert_eq!(changelog_section(log, "1.2.0").unwrap(), "New.\n\n### Added\n\n- a");
        assert_eq!(changelog_section(log, "1.1.2").unwrap(), "Old.");
        assert!(changelog_section(log, "1.3.0").is_none());
    }
}
