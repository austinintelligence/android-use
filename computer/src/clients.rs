//! `au install AGENT` and `au uninstall AGENT`: register the MCP server (and, for agents that
//! have a skills folder, the skill) with an agent client without touching anything else in its
//! configuration.
//!
//! Rules: one entry named `android-use` is added, updated, or removed and every other entry is
//! left as it was (key order and indentation included). An entry we did not write is never
//! replaced without `--force`. Claude Code and Codex are driven through their own CLI, because
//! their files are rewritten by the running app; the rest are edited in place, atomically, with
//! the previous file kept as `<file>.au-backup`. A file that is not strict JSON (comments, trailing
//! commas) is not edited: the snippet to paste is returned instead.
use crate::{
    api::{Code, Error, Result},
    device::{atomic, run_bounded},
};
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

/// How long an agent's own CLI may run before it is killed.
const CLI_TIMEOUT: Duration = Duration::from_secs(30);

const NAME: &str = "android-use";
const SKILL_FILES: [(&str, &str); 4] = [
    ("SKILL.md", include_str!("../../skills/android-use/SKILL.md")),
    ("references/protocol.md", include_str!("../../skills/android-use/references/protocol.md")),
    ("references/safety.md", include_str!("../../skills/android-use/references/safety.md")),
    ("references/setup.md", include_str!("../../skills/android-use/references/setup.md")),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Client {
    Claude,
    Codex,
    Cursor,
    Gemini,
    VsCode,
    ClaudeDesktop,
    OpenCode,
}

pub const ALL: [Client; 7] = [Client::Claude, Client::Codex, Client::Cursor, Client::Gemini, Client::VsCode, Client::ClaudeDesktop, Client::OpenCode];

/// Where a user's configuration lives. Built from the environment; tests point it at a temp dir.
#[derive(Clone)]
pub struct Dirs {
    pub home: PathBuf,
    pub config: PathBuf,
    pub app_data: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Result<Self> {
        let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).ok_or_else(|| Error::new(Code::Args, "HOME is not set"))?;
        let config = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        let app_data = if cfg!(windows) {
            env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Roaming"))
        } else if cfg!(target_os = "macos") {
            home.join("Library/Application Support")
        } else {
            config.clone()
        };
        Ok(Self { home, config, app_data })
    }
}

#[derive(Clone, Copy, Default)]
pub struct Options {
    pub dry_run: bool,
    pub force: bool,
}

impl Client {
    pub fn parse(name: &str) -> Option<Self> {
        ALL.into_iter().find(|client| client.name() == name.to_ascii_lowercase())
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::Gemini => "gemini",
            Self::VsCode => "vscode",
            Self::ClaudeDesktop => "claude-desktop",
            Self::OpenCode => "opencode",
        }
    }

    /// The agent's own command, when it has one that manages MCP servers.
    fn cli(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
            _ => None,
        }
    }

    /// Desktop apps do not inherit the shell's PATH, so they get an absolute command.
    fn gui(self) -> bool {
        matches!(self, Self::Cursor | Self::VsCode | Self::ClaudeDesktop)
    }

    fn config_file(self, d: &Dirs) -> PathBuf {
        match self {
            Self::Claude => d.home.join(".claude.json"),
            Self::Codex => d.home.join(".codex/config.toml"),
            Self::Cursor => d.home.join(".cursor/mcp.json"),
            Self::Gemini => d.home.join(".gemini/settings.json"),
            Self::VsCode => d.app_data.join("Code/User/mcp.json"),
            Self::ClaudeDesktop => d.app_data.join("Claude/claude_desktop_config.json"),
            Self::OpenCode => d.config.join("opencode/opencode.json"),
        }
    }

    /// The JSON object that holds the servers, as a key path.
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::VsCode => &["servers"],
            Self::OpenCode => &["mcp"],
            _ => &["mcpServers"],
        }
    }

    fn skill_dir(self, d: &Dirs) -> Option<PathBuf> {
        match self {
            Self::Claude => Some(d.home.join(".claude/skills").join(NAME)),
            Self::Codex => Some(d.home.join(".codex/skills").join(NAME)),
            _ => None,
        }
    }

    fn entry(self, launch: &[String]) -> Value {
        let argv: Vec<&str> = launch.iter().map(String::as_str).chain(["serve", "--mcp"]).collect();
        match self {
            Self::VsCode => json!({"type":"stdio","command":argv[0],"args":argv[1..]}),
            Self::OpenCode => json!({"type":"local","command":argv,"enabled":true}),
            _ => json!({"command":argv[0],"args":argv[1..]}),
        }
    }

    /// Whether this agent looks installed: its command is on PATH or its settings folder exists.
    pub fn present(self, d: &Dirs) -> bool {
        let (program, folder) = match self {
            Self::Claude => ("claude", d.home.join(".claude")),
            Self::Codex => ("codex", d.home.join(".codex")),
            Self::Cursor => ("", d.home.join(".cursor")),
            Self::Gemini => ("gemini", d.home.join(".gemini")),
            Self::VsCode => ("code", d.app_data.join("Code")),
            Self::ClaudeDesktop => ("", d.app_data.join("Claude")),
            Self::OpenCode => ("opencode", d.config.join("opencode")),
        };
        which(program).is_some() || folder.is_dir()
    }
}

/// One line of an install report.
#[derive(Debug)]
pub struct Report {
    pub client: Client,
    /// installed, updated, unchanged, removed, absent, manual, or `would_…` for a dry run.
    pub action: String,
    pub detail: String,
}

impl Report {
    pub fn json(&self) -> Value {
        json!({"client": self.client.name(), "action": self.action, "detail": self.detail})
    }
}

fn which(program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let names: Vec<String> = if cfg!(windows) { ["exe", "cmd", "bat"].iter().map(|ext| format!("{program}.{ext}")).collect() } else { vec![program.to_owned()] };
    env::split_paths(&env::var_os("PATH")?).flat_map(|dir| names.iter().map(move |name| dir.join(name))).find(|path| path.is_file())
}

/// What a config entry should run: `au` for terminal agents when `au` on PATH is this binary, the
/// PATH location for desktop apps (a stable shim, not a versioned path), else this binary.
fn command_for(client: Client) -> Vec<String> {
    let this = env::current_exe().ok().and_then(|path| fs::canonicalize(path).ok());
    let on_path = which("au").filter(|found| fs::canonicalize(found).ok() == this);
    launch_for(this.as_deref(), on_path, client.gui(), which("npx"))
}

/// A binary inside npm's `_npx` cache is deleted when the cache is pruned, so register a pinned
/// `npx -y android-use@<this version>` instead of that path. Desktop apps often lack the shell's
/// PATH, so they get the absolute `npx` found now.
fn launch_for(this: Option<&Path>, on_path: Option<PathBuf>, gui: bool, npx: Option<PathBuf>) -> Vec<String> {
    if this.is_some_and(|path| path.components().any(|part| part.as_os_str() == "_npx")) {
        let program = npx.filter(|_| gui).map_or_else(|| if cfg!(windows) { "npx.cmd" } else { "npx" }.to_owned(), |found| found.display().to_string());
        return vec![program, "-y".into(), format!("android-use@{}", env!("CARGO_PKG_VERSION"))];
    }
    vec![match (on_path, gui) {
        (Some(found), true) => found.display().to_string(),
        (Some(_), false) => "au".into(),
        (None, _) => this.map_or_else(|| "au".into(), |path| path.display().to_string()),
    }]
}

/// Whether an existing entry was written by au (it runs `serve --mcp`), so replacing it is safe.
fn ours(entry: &Value) -> bool {
    let text = entry.to_string();
    text.contains("serve") && text.contains("--mcp")
}

fn indent_of(text: &str) -> Vec<u8> {
    text.lines().find(|line| line.starts_with([' ', '\t'])).map(|line| line.bytes().take_while(|b| *b == b' ' || *b == b'\t').collect()).unwrap_or_else(|| b"  ".to_vec())
}

fn render(value: &Value, indent: &[u8]) -> Result<String> {
    use serde::Serialize;
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(indent));
    value.serialize(&mut serializer).map_err(|e| Error::new(Code::Io, e.to_string()))?;
    Ok(String::from_utf8_lossy(&out).into_owned() + "\n")
}

enum Edit {
    Done(&'static str),
    Manual(String),
}

/// Adds, updates, or removes `android-use` in a JSON config. `entry` of `None` removes.
fn edit_json(path: &Path, keys: &[&str], entry: Option<&Value>, o: Options) -> Result<Edit> {
    let text = read_config(path)?;
    let manual =
        |entry: &Value| Edit::Manual(format!("{} has comments or is not strict JSON, so it was not edited. Add under {}: \"{NAME}\": {entry}", path.display(), keys.join(".")));
    let mut root: Value = if text.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(_) => return Ok(manual(entry.unwrap_or(&Value::Null))),
        }
    };
    let mut node = root.as_object_mut().ok_or_else(|| Error::new(Code::Args, format!("{} is not a JSON object", path.display())))?;
    for key in keys {
        if node.get(*key).is_none() {
            if entry.is_none() {
                return Ok(Edit::Done("absent"));
            }
            node.insert((*key).into(), json!({}));
        }
        node = node.get_mut(*key).and_then(Value::as_object_mut).ok_or_else(|| Error::new(Code::Args, format!("{} has a {key} that is not an object", path.display())))?;
    }
    let existing = node.get(NAME).cloned();
    let outcome = match (&existing, entry) {
        (None, None) => return Ok(Edit::Done("absent")),
        (Some(found), Some(want)) if found == want => return Ok(Edit::Done("unchanged")),
        (Some(found), _) if !ours(found) && !o.force => {
            return Err(Error::new(Code::Args, format!("{} already has an entry named {NAME} that au did not write; pass --force to replace it", path.display())))
        }
        (_, Some(want)) => {
            node.insert(NAME.into(), want.clone());
            if existing.is_some() {
                "updated"
            } else {
                "installed"
            }
        }
        (_, None) => {
            node.shift_remove(NAME);
            "removed"
        }
    };
    if !o.dry_run {
        let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()); // keep a symlinked dotfile a symlink
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        backup(&target, &text)?;
        write_config(&target, render(&root, &indent_of(&text))?.as_bytes())?;
    }
    Ok(Edit::Done(outcome))
}

/// Codex keeps servers in `[mcp_servers.android-use]` tables; replace or append exactly that table.
fn edit_toml(path: &Path, block: Option<&str>, o: Options) -> Result<Edit> {
    let text = read_config(path)?;
    let header = format!("[mcp_servers.{NAME}]");
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) = toml_table(&lines);
    let outcome = match (start, block) {
        (None, None) => return Ok(Edit::Done("absent")),
        (Some(at), Some(block)) if lines[at..end].join("\n").trim_end() == block.trim_end() => return Ok(Edit::Done("unchanged")),
        (Some(at), _) if !ours(&json!(lines[at..end].join(" "))) && !o.force => {
            return Err(Error::new(Code::Args, format!("{} already has {header} that au did not write; pass --force to replace it", path.display())))
        }
        (Some(_), Some(_)) => "updated",
        (Some(_), None) => "removed",
        (None, Some(_)) => "installed",
    };
    let mut kept: Vec<String> = lines.iter().enumerate().filter(|(i, _)| start.is_none_or(|at| *i < at || *i >= end)).map(|(_, l)| (*l).to_owned()).collect();
    if let Some(block) = block {
        if kept.last().is_some_and(|l| !l.trim().is_empty()) {
            kept.push(String::new());
        }
        kept.extend(block.lines().map(str::to_owned));
    }
    if !o.dry_run {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        backup(&target, &text)?;
        write_config(&target, (kept.join("\n").trim_end().to_owned() + "\n").as_bytes())?;
    }
    Ok(Edit::Done(outcome))
}

/// The table au writes for Codex.
fn toml_block(command: &[String]) -> String {
    let args = command[1..].iter().map(String::as_str).chain(["serve", "--mcp"]).map(|arg| json!(arg).to_string()).collect::<Vec<_>>().join(", ");
    format!("[mcp_servers.{NAME}]\ncommand = {}\nargs = [{args}]\n", json!(command[0]))
}

/// Where the `[mcp_servers.android-use]` table starts and ends in `lines`.
fn toml_table(lines: &[&str]) -> (Option<usize>, usize) {
    let start = lines.iter().position(|line| line.trim() == format!("[mcp_servers.{NAME}]"));
    (start, start.map_or(0, |at| lines[at + 1..].iter().position(|line| line.trim_start().starts_with('[')).map_or(lines.len(), |n| at + 1 + n)))
}

/// The `android-use` entry as it stands in a client's config (a JSON value or a TOML table).
fn read_entry(client: Client, file: &Path) -> Option<String> {
    let text = fs::read_to_string(file).ok()?;
    if client == Client::Codex {
        let lines: Vec<&str> = text.lines().collect();
        let (start, end) = toml_table(&lines);
        return start.map(|at| lines[at..end].join("\n") + "\n");
    }
    let value: Value = serde_json::from_str(&text).ok()?;
    let node = client.keys().iter().try_fold(&value, |node, key| node.get(*key))?;
    node.get(NAME).map(Value::to_string)
}

/// Reads a config file. Only a missing file is an empty config; any other error aborts the edit.
fn read_config(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        other => other.map_err(|e| Error::new(Code::Io, format!("could not read {}: {e}", path.display()))),
    }
}

/// Saves the previous file as `<file>.au-backup`; without the backup the write does not happen.
fn backup(target: &Path, text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    fs::copy(target, format!("{}.au-backup", target.display()))
        .map(drop)
        .map_err(|e| Error::new(Code::Io, format!("could not back up {}, so it was not changed: {e}", target.display())))
}

/// Replaces a config atomically, keeping the file's existing mode (new files stay private).
fn write_config(target: &Path, bytes: &[u8]) -> Result<()> {
    let permissions = fs::metadata(target).ok().map(|meta| meta.permissions());
    atomic(target, bytes)?;
    Ok(permissions.map_or(Ok(()), |permissions| fs::set_permissions(target, permissions))?)
}

/// Runs the agent's own CLI, killed after `timeout`; true when it succeeded.
fn run_cli(program: &Path, args: &[String], timeout: Duration) -> bool {
    run_bounded(program, &args.iter().map(String::as_str).collect::<Vec<_>>(), timeout).is_ok()
}

/// Replaces the entry through the agent's CLI (remove the old one, add the new). When the add
/// fails the old entry is put back, so a failed update never leaves the agent without au.
fn replace_entry(client: Client, d: &Dirs, cli: &Path, command: &[String], timeout: Duration) -> bool {
    let file = fs::canonicalize(client.config_file(d)).unwrap_or_else(|_| client.config_file(d));
    let before = read_entry(client, &file);
    let _ = run_cli(cli, &cli_args(client, None), timeout);
    let added = run_cli(cli, &cli_args(client, Some(command)), timeout);
    // Put back only the one entry, and only if the remove really took it: the running agent
    // writes this file too, so the rest of it is never rewritten from a snapshot.
    if let (false, Some(old), None) = (added, &before, read_entry(client, &file)) {
        let force = Options { force: true, dry_run: false };
        let _ = if client == Client::Codex {
            edit_toml(&file, Some(old), force)
        } else {
            serde_json::from_str(old).map_err(|_| Error::new(Code::Io, "bad entry")).and_then(|entry| edit_json(&file, client.keys(), Some(&entry), force))
        };
    }
    added
}

fn already_registered(client: Client, d: &Dirs, command: &[String]) -> Option<bool> {
    let text = fs::read_to_string(client.config_file(d)).ok()?;
    match client {
        Client::Codex => Some(text.contains(&format!("[mcp_servers.{NAME}]")) && command.iter().all(|part| text.contains(part.as_str()))),
        _ => {
            let value: Value = serde_json::from_str(&text).ok()?;
            let servers = value.get("mcpServers")?;
            Some(servers.get(NAME).is_some_and(|entry| entry == &client.entry(command)))
        }
    }
}

fn write_skill(dir: &Path, o: Options) -> Result<&'static str> {
    let marker = dir.join(".au-installed");
    if dir.exists() && !marker.exists() {
        return Ok("skill_kept");
    }
    if !o.dry_run {
        for (name, body) in SKILL_FILES {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap_or(dir))?;
            fs::write(path, body)?;
        }
        fs::write(&marker, env!("CARGO_PKG_VERSION"))?;
    }
    Ok("skill_installed")
}

fn file_edit(client: Client, d: &Dirs, command: Option<&[String]>, o: Options) -> Result<Edit> {
    let path = client.config_file(d);
    match (client, command) {
        (Client::Codex, _) => edit_toml(&path, command.map(toml_block).as_deref(), o),
        (_, Some(command)) => edit_json(&path, client.keys(), Some(&client.entry(command)), o),
        (_, None) => edit_json(&path, client.keys(), None, o),
    }
}

/// `claude mcp …` / `codex mcp …` arguments for removing or adding the server.
fn cli_args(client: Client, command: Option<&[String]>) -> Vec<String> {
    let scope: &[&str] = if client == Client::Claude { &["--scope", "user"] } else { &[] };
    let mut args: Vec<&str> = vec!["mcp", if command.is_some() { "add" } else { "remove" }, NAME];
    args.extend(scope);
    if let Some(command) = command {
        args.push("--");
        args.extend(command.iter().map(String::as_str).chain(["serve", "--mcp"]));
    }
    args.into_iter().map(str::to_owned).collect()
}

fn outcome(edit: Edit, o: Options) -> (String, String) {
    match edit {
        Edit::Done(action @ ("unchanged" | "absent")) => (action.to_owned(), String::new()),
        Edit::Done(action) if o.dry_run => (
            format!(
                "would_{}",
                match action {
                    "installed" => "install",
                    "updated" => "update",
                    _ => "remove",
                }
            ),
            String::new(),
        ),
        Edit::Done(action) => (action.to_owned(), String::new()),
        Edit::Manual(snippet) => ("manual".into(), snippet),
    }
}

/// Installs into every agent that looks present; the result is the list of reports as JSON.
pub fn detect_and_install(d: &Dirs, o: Options) -> Value {
    Value::Array(ALL.into_iter().filter(|client| client.present(d)).map(|client| report(install(client, d, o), client)).collect())
}

/// A report as JSON; a failure for one agent becomes an `error` row instead of ending the run.
pub fn report(result: Result<Report>, client: Client) -> Value {
    result.map(|report| report.json()).unwrap_or_else(|error| json!({"client": client.name(), "action": "error", "detail": error.message}))
}

pub fn install(client: Client, d: &Dirs, o: Options) -> Result<Report> {
    let command = command_for(client);
    let cli = client.cli().filter(|cli| which(cli).is_some());
    let (action, mut detail) = match cli {
        Some(_) if already_registered(client, d, &command) == Some(true) => ("unchanged".to_owned(), "via the agent's own CLI".to_owned()),
        Some(_) if o.dry_run => ("would_install".to_owned(), "via the agent's own CLI".to_owned()),
        Some(cli) => {
            if !replace_entry(client, d, &which(cli).unwrap_or_default(), &command, CLI_TIMEOUT) {
                return Err(Error::new(Code::Device, format!("{cli} mcp add failed; run it yourself: {cli} {}", cli_args(client, Some(&command)).join(" "))));
            }
            ("installed".to_owned(), "via the agent's own CLI".to_owned())
        }
        None => outcome(file_edit(client, d, Some(&command), o)?, o),
    };
    if action != "manual" {
        if let Some(dir) = client.skill_dir(d) {
            detail = format!("{detail} {}", write_skill(&dir, o)?).trim().to_owned();
        }
    }
    Ok(Report { client, action, detail: format!("{}; {detail}", command.join(" ")).trim_end_matches("; ").to_owned() })
}

pub fn uninstall(client: Client, d: &Dirs, o: Options) -> Result<Report> {
    let cli = client.cli().filter(|cli| which(cli).is_some());
    let (action, mut detail) = match cli {
        Some(cli) if !o.dry_run && run_cli(&which(cli).unwrap_or_default(), &cli_args(client, None), CLI_TIMEOUT) => ("removed".to_owned(), String::new()),
        _ => outcome(file_edit(client, d, None, o)?, o),
    };
    if let Some(dir) = client.skill_dir(d).filter(|dir| dir.join(".au-installed").exists()) {
        if !o.dry_run {
            fs::remove_dir_all(&dir)?;
        }
        detail = "skill removed".into();
    }
    Ok(Report { client, action, detail })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> (Dirs, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let d = Dirs { home: dir.path().to_path_buf(), config: dir.path().join(".config"), app_data: dir.path().join("Library/Application Support") };
        (d, dir)
    }

    #[test]
    fn setup_under_an_npx_cache_registers_the_pinned_npx_command() {
        let cached = Path::new("/home/u/.npm/_npx/abc123/node_modules/@androiduse/linux-x64/bin/au");
        let launch = launch_for(Some(cached), Some(PathBuf::from("/usr/bin/au")), false, None);
        assert_eq!(launch[1..], ["-y".to_owned(), format!("android-use@{}", env!("CARGO_PKG_VERSION"))]);
        assert_eq!(Client::Cursor.entry(&launch)["args"], json!(["-y", format!("android-use@{}", env!("CARGO_PKG_VERSION")), "serve", "--mcp"]));
        assert_eq!(launch_for(Some(Path::new("/opt/au/bin/au")), None, false, None), ["/opt/au/bin/au"]);
        // Desktop apps get the absolute npx, not a bare name they may fail to find.
        assert_eq!(launch_for(Some(cached), None, true, Some(PathBuf::from("/opt/node/bin/npx")))[0], "/opt/node/bin/npx");
    }

    #[test]
    fn json_clients_get_one_entry_and_keep_everything_else_byte_for_byte() {
        let (d, _dir) = dirs();
        let path = Client::Cursor.config_file(&d);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\n    \"theme\": \"dark\",\n    \"mcpServers\": {\n        \"zeta\": {\"command\": \"z\"},\n        \"alpha\": {\"command\": \"a\"}\n    }\n}\n")
            .unwrap();
        let report = install(Client::Cursor, &d, Options::default()).unwrap();
        assert_eq!(report.action, "installed");
        let text = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["mcpServers"]["zeta"], json!({"command":"z"}));
        assert_eq!(value["mcpServers"]["android-use"]["args"], json!(["serve", "--mcp"]));
        assert!(text.starts_with("{\n    \"theme\""), "indentation and key order kept:\n{text}");
        assert!(text.find("zeta").unwrap() < text.find("alpha").unwrap() && text.find("alpha").unwrap() < text.find("android-use").unwrap());
        assert_eq!(install(Client::Cursor, &d, Options::default()).unwrap().action, "unchanged");
        assert!(Path::new(&format!("{}.au-backup", path.display())).exists());
        assert_eq!(uninstall(Client::Cursor, &d, Options::default()).unwrap().action, "removed");
        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after, json!({"theme":"dark","mcpServers":{"zeta":{"command":"z"},"alpha":{"command":"a"}}}));
        assert_eq!(uninstall(Client::Cursor, &d, Options::default()).unwrap().action, "absent");
    }

    #[test]
    fn a_foreign_entry_is_never_replaced_without_force_and_dry_run_writes_nothing() {
        let (d, _dir) = dirs();
        let path = Client::VsCode.config_file(&d);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let theirs = "{\"servers\":{\"android-use\":{\"command\":\"other-tool\"}}}";
        fs::write(&path, theirs).unwrap();
        assert!(install(Client::VsCode, &d, Options::default()).unwrap_err().message.contains("--force"));
        assert_eq!(fs::read_to_string(&path).unwrap(), theirs);
        let dry = install(Client::VsCode, &d, Options { dry_run: true, force: true }).unwrap();
        assert_eq!(dry.action, "would_update");
        assert_eq!(fs::read_to_string(&path).unwrap(), theirs, "dry run changes nothing");
        assert_eq!(install(Client::VsCode, &d, Options { dry_run: false, force: true }).unwrap().action, "updated");
        assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&path).unwrap()).unwrap()["servers"]["android-use"]["type"], "stdio");
    }

    #[test]
    fn a_file_with_comments_is_left_alone_and_the_snippet_is_returned() {
        let (d, _dir) = dirs();
        let path = Client::OpenCode.config_file(&d);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\n  // my settings\n  \"theme\": \"x\",\n}\n").unwrap();
        let error = install(Client::OpenCode, &d, Options::default()).unwrap();
        assert_eq!(error.action, "manual");
        assert!(error.detail.contains("not strict JSON") && error.detail.contains("android-use"), "{}", error.detail);
        assert!(fs::read_to_string(&path).unwrap().contains("// my settings"));
    }

    #[test]
    fn toml_edits_touch_only_the_android_use_table() {
        let (d, _dir) = dirs();
        let path = Client::Codex.config_file(&d);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let theirs = "model = \"x\"\n\n[mcp_servers.other]\ncommand = \"o\"\n";
        fs::write(&path, theirs).unwrap();
        let o = Options::default();
        assert_eq!(edit_toml(&path, Some(&toml_block(&["/bin/au".to_owned()])), o).unwrap_done(), "installed");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(theirs) && text.contains("[mcp_servers.android-use]\ncommand = \"/bin/au\""), "{text}");
        assert_eq!(edit_toml(&path, Some(&toml_block(&["/bin/au".to_owned()])), o).unwrap_done(), "unchanged");
        assert_eq!(edit_toml(&path, Some(&toml_block(&["/new/au".to_owned()])), o).unwrap_done(), "updated");
        assert_eq!(edit_toml(&path, None, o).unwrap_done(), "removed");
        assert_eq!(fs::read_to_string(&path).unwrap(), theirs);
    }

    #[test]
    fn the_skill_is_installed_once_and_a_foreign_skill_folder_is_kept() {
        let (d, _dir) = dirs();
        let dir = Client::Claude.skill_dir(&d).unwrap();
        assert_eq!(write_skill(&dir, Options::default()).unwrap(), "skill_installed");
        assert!(dir.join("SKILL.md").is_file() && dir.join("references/protocol.md").is_file());
        let (d2, _dir2) = dirs();
        let theirs = Client::Codex.skill_dir(&d2).unwrap();
        fs::create_dir_all(&theirs).unwrap();
        fs::write(theirs.join("SKILL.md"), "mine").unwrap();
        assert_eq!(write_skill(&theirs, Options::default()).unwrap(), "skill_kept");
        assert_eq!(fs::read_to_string(theirs.join("SKILL.md")).unwrap(), "mine");
    }

    trait Done {
        fn unwrap_done(self) -> &'static str;
    }

    impl Done for Result<Edit> {
        fn unwrap_done(self) -> &'static str {
            match self.unwrap() {
                Edit::Done(action) => action,
                Edit::Manual(snippet) => panic!("{snippet}"),
            }
        }
    }
}

#[cfg(all(test, unix))]
mod safe_write_tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, time::Instant};

    fn fake_cli(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-cli");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn cursor(dir: &tempfile::TempDir) -> (Dirs, PathBuf) {
        let d = Dirs { home: dir.path().to_path_buf(), config: dir.path().join(".config"), app_data: dir.path().join("Library") };
        let path = Client::Cursor.config_file(&d);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        (d, path)
    }

    #[test]
    fn unreadable_config_and_failed_backup_abort_without_writing_and_mode_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (d, path) = cursor(&dir);
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(install(Client::Cursor, &d, Options::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), [0xff, 0xfe, 0x00]);

        fs::write(&path, "{\"theme\":\"x\"}").unwrap();
        fs::create_dir(format!("{}.au-backup", path.display())).unwrap();
        assert!(install(Client::Cursor, &d, Options::default()).unwrap_err().message.contains("back up"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"theme\":\"x\"}");
        fs::remove_dir(format!("{}.au-backup", path.display())).unwrap();

        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        install(Client::Cursor, &d, Options::default()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
    }

    #[test]
    fn a_hung_agent_cli_is_killed_even_when_a_grandchild_holds_the_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = cursor(&dir);
        for body in ["exec sleep 30", "sleep 6 &\nexit 0"] {
            let started = Instant::now();
            replace_entry(Client::Claude, &d, &fake_cli(dir.path(), body), &["au".to_owned()], Duration::from_millis(300));
            assert!(started.elapsed() < Duration::from_secs(3), "{body}: {:?}", started.elapsed());
        }
    }

    #[test]
    fn a_failed_re_add_restores_only_the_old_entry_and_only_when_it_was_removed() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = cursor(&dir);
        let file = Client::Claude.config_file(&d);
        let old = "{\"startups\":1,\"mcpServers\":{\"android-use\":{\"command\":\"au\"}}}";
        let state = |text: &str| fs::write(&file, text).unwrap();
        // The remove takes the entry and the running agent bumps a counter; the add fails.
        state(old);
        let removes = fake_cli(dir.path(), &format!("[ \"$2\" = remove ] && echo '{{\"startups\":2,\"mcpServers\":{{}}}}' > '{}'; exit 1", file.display()));
        assert!(!replace_entry(Client::Claude, &d, &removes, &["au".to_owned()], CLI_TIMEOUT));
        let restored: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(restored, json!({"startups":2,"mcpServers":{"android-use":{"command":"au"}}}));
        // The remove did nothing, but the agent changed the file meanwhile: leave it alone.
        state(old);
        let bumps = fake_cli(dir.path(), &format!("echo '{{\"startups\":9,\"mcpServers\":{{\"android-use\":{{\"command\":\"au\"}}}}}}' > '{}'; exit 1", file.display()));
        assert!(!replace_entry(Client::Claude, &d, &bumps, &["au".to_owned()], CLI_TIMEOUT));
        assert!(fs::read_to_string(&file).unwrap().contains("\"startups\":9"));
    }
}
