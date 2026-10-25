//! Secrets the agent can use but never read: values live in the operating system's store, only
//! names and the apps allowed to receive them are kept in `AU_HOME/secrets.json`. The agent says
//! `type secret "NAME" in "FIELD"`; the host checks the field is a password field of an allowed
//! app, types the value, and removes it from every reply, error and trace.
use crate::{
    api::{normalized, Code, Error, Result},
    device::private_file,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(not(windows))]
const SERVICE: &str = "android-use";

pub trait SecretStore {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn remove(&self, name: &str) -> Result<()>;
}

/// macOS Keychain (`security`), Secret Service (`secret-tool`), or a per-user DPAPI file on Windows.
pub struct OsStore {
    root: PathBuf,
}

impl OsStore {
    pub fn at(root: &Path) -> Self {
        Self { root: root.to_path_buf() }
    }

    /// Runs the store's own tool. `file` reaches PowerShell as `AU_SECRET_FILE`: arguments after
    /// `-Command` are part of the script, not values it can read.
    fn run(program: &str, args: &[&str], stdin: Option<&str>, file: Option<&Path>) -> Result<(bool, String)> {
        Self::run_code(program, args, stdin, file).map(|(code, out)| (code == Some(0), out))
    }

    /// `run` with the tool's exit code, for telling "not there" from a failure.
    fn run_code(program: &str, args: &[&str], stdin: Option<&str>, file: Option<&Path>) -> Result<(Option<i32>, String)> {
        let mut command = Command::new(program);
        if let Some(file) = file {
            command.env("AU_SECRET_FILE", file);
        }
        let mut child = command
            .args(args)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Error::new(Code::Unsupported, format!("the secret store needs `{program}`: {e}")))?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(text.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        Ok((out.status.code(), String::from_utf8_lossy(&out.stdout).trim_end_matches(['\r', '\n']).to_owned()))
    }

    #[cfg(windows)]
    fn file(&self, name: &str) -> PathBuf {
        // Hex keeps names that differ only in punctuation ("bank pin", "bank-pin") apart.
        let hex: String = name.bytes().map(|b| format!("{b:02x}")).collect();
        self.root.join("secrets").join(format!("{hex}.dpapi"))
    }
}

impl SecretStore for OsStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        let _ = &self.root;
        // One expression per platform, so no branch needs an early return.
        #[cfg(target_os = "macos")]
        let found = Self::run("security", &["find-generic-password", "-s", SERVICE, "-a", name, "-w"], None, None)?;
        #[cfg(windows)]
        let found = {
            let file = self.file(name);
            let script = "$s = Get-Content -LiteralPath $env:AU_SECRET_FILE | ConvertTo-SecureString; [Runtime.InteropServices.Marshal]::PtrToStringBSTR([Runtime.InteropServices.Marshal]::SecureStringToBSTR($s))";
            if file.is_file() {
                Self::run("powershell", &["-NoProfile", "-Command", script], None, Some(&file))?
            } else {
                (false, String::new())
            }
        };
        #[cfg(not(any(target_os = "macos", windows)))]
        let found = Self::run("secret-tool", &["lookup", "service", SERVICE, "account", name], None, None)?;
        let (ok, out) = found;
        Ok((ok && !out.is_empty()).then_some(out))
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        let result = Self::run("security", &["add-generic-password", "-U", "-s", SERVICE, "-a", name, "-w", value], None, None);
        #[cfg(windows)]
        let result = {
            fs::create_dir_all(self.root.join("secrets"))?;
            let script = "[Console]::In.ReadLine() | ConvertTo-SecureString -AsPlainText -Force | ConvertFrom-SecureString | Set-Content -LiteralPath $env:AU_SECRET_FILE";
            Self::run("powershell", &["-NoProfile", "-Command", script], Some(&format!("{value}\n")), Some(&self.file(name)))
        };
        #[cfg(not(any(target_os = "macos", windows)))]
        let result = Self::run("secret-tool", &["store", "--label=Android Use", "service", SERVICE, "account", name], Some(value), None);
        match result? {
            (true, _) => Ok(()),
            _ => {
                Err(Error::plain(Code::Unsupported, "The operating system's secret store refused the value. On Linux, install `secret-tool` (libsecret) and unlock your keyring."))
            }
        }
    }

    fn remove(&self, name: &str) -> Result<()> {
        // Done, or the value was not there: `security` exits 44 for an item it cannot find, and
        // `secret-tool clear` succeeds when nothing matched. Anything else leaves the value.
        #[cfg(target_os = "macos")]
        let done = matches!(Self::run_code("security", &["delete-generic-password", "-s", SERVICE, "-a", name], None, None)?.0, Some(0 | 44));
        #[cfg(windows)]
        let done = match fs::remove_file(self.file(name)) {
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
            Ok(()) => true,
        };
        #[cfg(not(any(target_os = "macos", windows)))]
        let done = Self::run("secret-tool", &["clear", "service", SERVICE, "account", name], None, None)?.0;
        if !done {
            return Err(Error::plain(Code::Io, "The system's secret store kept the value; unlock it and try again. Nothing was changed."));
        }
        Ok(())
    }
}

/// Secret names and the packages each may be typed into: the only secret state kept in AU_HOME.
fn meta_path(root: &Path) -> PathBuf {
    root.join("secrets.json")
}

/// A JSON object of name -> package list.
type Meta = serde_json::Map<String, Value>;

/// A missing file is empty; one that cannot be read is an error, so nothing rewrites it from scratch.
fn load_meta(root: &Path) -> Result<Meta> {
    let text = match fs::read_to_string(meta_path(root)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Meta::new()),
        read => read?,
    };
    match serde_json::from_str(&text) {
        Ok(Value::Object(meta)) => Ok(meta),
        _ => Err(Error::plain(Code::Io, "secrets.json in the state folder is damaged. Fix or delete it; nothing was changed.")),
    }
}

fn apps_of(value: &Value) -> Vec<String> {
    value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

fn save_meta(root: &Path, meta: &Meta) -> Result<()> {
    let mut file = private_file().write(true).create(true).truncate(true).open(meta_path(root))?;
    writeln!(file, "{}", Value::Object(meta.clone()))?;
    Ok(())
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 48 && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')) && name.chars().any(char::is_alphanumeric)
}

/// A value the grammar can carry inside quotes.
fn typeable(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= 256 && !value.chars().any(|c| c == '"' || c == '\\' || c.is_control())
}

pub fn add(root: &Path, store: &dyn SecretStore, name: &str, apps: &[String], value: &str) -> Result<Value> {
    if !valid_name(name) {
        return Err(Error::new(Code::Args, "a secret name is 1 to 48 letters, digits, spaces, - or _"));
    }
    if apps.is_empty() || apps.iter().any(|app| app.is_empty() || !app.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')) {
        return Err(Error::new(Code::Args, "give the app packages that may receive it: --apps com.example.bank,com.example.other"));
    }
    if !typeable(value) {
        return Err(Error::new(Code::Args, "a secret is 1 to 256 characters without quotes, backslashes or control characters"));
    }
    let key = normalized(name);
    let mut meta = load_meta(root)?;
    // Narrowed first: before the new value is stored, only apps on both the old and the new list
    // keep the name, so a failure part-way never leaves an app the new list drops with the value.
    if let Some(old) = meta.get(&key).map(apps_of) {
        let both: Vec<&String> = old.iter().filter(|app| apps.contains(app)).collect();
        if both.len() < old.len() {
            meta.insert(key.clone(), json!(both));
            save_meta(root, &meta)?;
        }
    }
    store.set(&key, value)?;
    meta.insert(key, json!(apps));
    save_meta(root, &meta)?;
    Ok(json!({"ok":1,"text":format!("Saved secret \"{name}\" for {}.", apps.join(", "))}))
}

pub fn remove(root: &Path, store: &dyn SecretStore, name: &str) -> Result<Value> {
    let mut meta = load_meta(root)?;
    store.remove(&normalized(name))?;
    let existed = meta.remove(&normalized(name)).is_some();
    save_meta(root, &meta)?;
    Ok(json!({"ok":1,"text":if existed { format!("Removed secret \"{name}\".") } else { format!("There was no secret \"{name}\".") }}))
}

/// Names and allowed apps; never values.
pub fn list(root: &Path) -> String {
    let meta = match load_meta(root) {
        Ok(meta) => meta,
        Err(e) => return e.message.to_string(),
    };
    if meta.is_empty() {
        return "No secrets saved. Add one with `au secret add NAME --apps PACKAGE`.".into();
    }
    meta.iter().map(|(name, apps)| format!("{name}: {}", apps_of(apps).join(", "))).collect::<Vec<_>>().join("\n")
}

/// A `type secret "NAME" in "FIELD"` step found in a command.
#[derive(Debug)]
pub(crate) struct Step {
    pub name: String,
    pub field: String,
    /// Byte range of the whole step in the command.
    pub range: std::ops::Range<usize>,
}

/// The one secret step in `command`, if any. It must open the command.
pub(crate) fn find(command: &str) -> Result<Option<Step>> {
    let mut found = None;
    let mut search = 0;
    while let Some(rel) = command[search..].find("type secret ") {
        let at = search + rel;
        search = at + 1;
        let before = command[..at].trim_end();
        let step_start = before.is_empty() || before.to_ascii_lowercase().ends_with(" then") || before.eq_ignore_ascii_case("then");
        if !step_start {
            continue;
        }
        if !before.is_empty() {
            // The field must be on the screen when the command starts, or it cannot be checked.
            return Err(Error::new(Code::Args, "type secret must be the first step of its command; tap the field in an earlier command"));
        }
        let rest = &command[at + "type secret ".len()..];
        let bad = || Error::new(Code::Args, r#"use: type secret "NAME" in "FIELD""#);
        let name_body = rest.strip_prefix('"').ok_or_else(bad)?;
        let name_end = name_body.find('"').ok_or_else(bad)?;
        let after = name_body[name_end + 1..].strip_prefix(" in \"").ok_or_else(bad)?;
        let field_end = after.find('"').ok_or_else(bad)?;
        if found.is_some() {
            return Err(Error::new(Code::Bounds, "one secret per command; nothing was run"));
        }
        let consumed = "type secret ".len() + 1 + name_end + 1 + " in \"".len() + field_end + 1;
        found = Some(Step { name: name_body[..name_end].to_owned(), field: after[..field_end].to_owned(), range: at..at + consumed });
    }
    Ok(found)
}

/// Replaces the step with a plain `type "VALUE" in "FIELD"`.
pub(crate) fn substitute(command: &str, step: &Step, value: &str) -> String {
    format!("{}type \"{value}\" in \"{}\"{}", &command[..step.range.start], step.field, &command[step.range.end..])
}

/// Removes the value (and its quoted form) from any text that is about to leave the host.
pub(crate) fn scrub(text: &str, name: &str, value: &str) -> String {
    let text = text.replace(value, &format!("[secret \"{name}\"]"));
    // A reply can carry a shortened or cleaned-up copy (long values are cut, some punctuation is
    // dropped), which is not the exact value. Any line still holding a piece of it is hidden whole.
    let pieces = fragments(value);
    let hidden = format!("[a line holding part of secret \"{name}\" was hidden]");
    text.lines().map(|line| if pieces.iter().any(|piece| line.contains(piece.as_str())) { hidden.as_str() } else { line }).collect::<Vec<_>>().join("\n")
}

/// Every run of four characters of the value, also with the punctuation a reply drops removed;
/// the whole value when it is shorter.
fn fragments(value: &str) -> Vec<String> {
    let cleaned: String = value.chars().filter(|c| !matches!(c, '[' | ']' | '{' | '}') && !c.is_control()).collect();
    let mut out = Vec::new();
    for form in [value, cleaned.as_str()] {
        let chars: Vec<char> = form.chars().collect();
        if chars.is_empty() {
            continue;
        }
        let width = chars.len().min(4);
        out.extend(chars.windows(width).map(|w| w.iter().collect::<String>()));
    }
    out
}

pub(crate) fn allowed_apps(root: &Path, name: &str) -> Option<Vec<String>> {
    load_meta(root).ok()?.get(&normalized(name)).map(apps_of)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};

    #[derive(Default)]
    pub(crate) struct Memory(pub RefCell<HashMap<String, String>>);

    impl SecretStore for Memory {
        fn get(&self, name: &str) -> Result<Option<String>> {
            Ok(self.0.borrow().get(name).cloned())
        }
        fn set(&self, name: &str, value: &str) -> Result<()> {
            self.0.borrow_mut().insert(name.to_owned(), value.to_owned());
            Ok(())
        }
        fn remove(&self, name: &str) -> Result<()> {
            self.0.borrow_mut().remove(name);
            Ok(())
        }
    }

    #[test]
    fn the_value_goes_to_the_store_and_only_names_reach_the_disk() {
        let d = tempfile::tempdir().unwrap();
        let store = Memory::default();
        add(d.path(), &store, "Bank PIN", &["com.example.bank".into()], "4921").unwrap();
        assert_eq!(store.get("bank pin").unwrap().as_deref(), Some("4921"));
        let on_disk = fs::read_to_string(meta_path(d.path())).unwrap();
        assert!(!on_disk.contains("4921") && on_disk.contains("com.example.bank"), "{on_disk}");
        assert_eq!(list(d.path()), "bank pin: com.example.bank");
        assert_eq!(allowed_apps(d.path(), "BANK  pin"), Some(vec!["com.example.bank".into()]));
        remove(d.path(), &store, "bank pin").unwrap();
        assert!(store.get("bank pin").unwrap().is_none() && list(d.path()).starts_with("No secrets"));
    }

    /// A store that refuses every new value.
    struct Refusing;
    impl SecretStore for Refusing {
        fn get(&self, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &str) -> Result<()> {
            Err(Error::plain(Code::Unsupported, "refused"))
        }
        fn remove(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_re_add_that_fails_part_way_leaves_only_apps_on_both_lists() {
        let d = tempfile::tempdir().unwrap();
        add(d.path(), &Memory::default(), "pin", &["com.a".into(), "com.b".into()], "1111").unwrap();
        assert!(add(d.path(), &Refusing, "pin", &["com.b".into(), "com.c".into()], "2222").is_err());
        assert_eq!(allowed_apps(d.path(), "pin"), Some(vec!["com.b".into()]), "com.a was dropped before the new value went in");
    }

    #[test]
    fn a_damaged_names_file_is_kept_and_nothing_is_stored() {
        let d = tempfile::tempdir().unwrap();
        let store = Memory::default();
        fs::write(meta_path(d.path()), "{\"bank pin\": [\"com.example.bank\"").unwrap();
        assert_eq!(add(d.path(), &store, "other", &["com.x".into()], "v").unwrap_err().code, Code::Io);
        assert!(remove(d.path(), &store, "bank pin").is_err() && store.0.borrow().is_empty());
        assert!(list(d.path()).contains("damaged") && allowed_apps(d.path(), "bank pin").is_none());
        assert_eq!(fs::read_to_string(meta_path(d.path())).unwrap(), "{\"bank pin\": [\"com.example.bank\"", "the file is left for the person to fix");
    }

    #[test]
    fn bad_names_apps_and_values_are_refused_before_anything_is_stored() {
        let d = tempfile::tempdir().unwrap();
        let store = Memory::default();
        for (name, apps, value) in [
            ("a/b", vec!["com.x"], "v"),
            ("ok", vec![], "v"),
            ("ok", vec!["bad app"], "v"),
            ("ok", vec!["com.x"], ""),
            ("ok", vec!["com.x"], "a\"b"),
            ("ok", vec!["com.x"], "a\\b"),
        ] {
            let apps: Vec<String> = apps.into_iter().map(str::to_owned).collect();
            assert!(add(d.path(), &store, name, &apps, value).is_err(), "{name} {apps:?} {value:?}");
        }
        assert!(store.0.borrow().is_empty() && !meta_path(d.path()).exists());
    }

    #[test]
    fn the_step_must_open_the_command_and_is_swapped_for_a_typed_value() {
        let command = r#"type secret "bank pin" in "PIN" then tap "OK""#;
        let step = find(command).unwrap().unwrap();
        assert_eq!((step.name.as_str(), step.field.as_str()), ("bank pin", "PIN"));
        assert_eq!(substitute(command, &step, "4921"), r#"type "4921" in "PIN" then tap "OK""#);
        assert!(find(r#"tap "PIN" then type secret "bank pin" in "PIN""#).unwrap_err().message.contains("first step"));
        assert!(find(r#"type secret "x" in "F""#).unwrap().is_some());
        assert!(find(r#"tap "type secret" then tap "x""#).unwrap().is_none(), "words inside a label are not a step");
        assert!(find(r#"type secret x in "F""#).is_err() && find(r#"type secret "x""#).is_err());
        assert!(find(r#"type secret "a" in "F" then type secret "b" in "G""#).is_err(), "one per command");
    }

    #[test]
    fn a_cut_or_cleaned_copy_of_the_value_hides_its_line() {
        let long = "x".repeat(10) + "Zq7#mK9!vL2@pR4" + &"y".repeat(60);
        let cut = format!("Partly done: Typed {}… in Password\nsw Wi-Fi=on", &long[..63]);
        assert_eq!(scrub(&cut, "k", &long), "[a line holding part of secret \"k\" was hidden]\nsw Wi-Fi=on");
        let cleaned = scrub("Typed password1 in Password", "k", "pa[ss]word1");
        assert!(!cleaned.contains("password1") && cleaned.contains("was hidden"), "{cleaned}");
        assert_eq!(scrub("Settings\nWi-Fi", "k", "4921"), "Settings\nWi-Fi", "lines without a piece stay");
    }

    #[test]
    fn scrub_replaces_every_copy_of_the_value() {
        assert_eq!(scrub("typed 4921 then 4921", "bank pin", "4921"), r#"typed [secret "bank pin"] then [secret "bank pin"]"#);
    }
}
