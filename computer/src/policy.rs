//! `AU_HOME/policy.toml`: what a session may do, checked on the host before anything reaches the
//! helper. The file is a small TOML subset (`key = "text" | true | 12 | ["a", "b"]`, `#` comments).
//! A file that does not parse refuses the session; it never means "no policy".
use crate::api::{normalized, Code, Error, Result};
use std::{fs, path::Path};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Policy {
    /// When set, only these apps (names or packages) may be opened.
    pub apps_allow: Vec<String>,
    pub apps_deny: Vec<String>,
    /// Words that make a label risky, on top of the built-in list.
    pub risky_extra: Vec<String>,
    pub max_actions: Option<u32>,
    /// `false` blocks camera, microphone, screen capture and recording.
    pub capture: bool,
    /// Apps whose notifications `read code from notification` may read.
    pub codes_apps: Vec<String>,
    pub present: bool,
}

fn text(value: &str, line: usize) -> Result<String> {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .filter(|v| !v.contains('"') && !v.contains('\\'))
        .map(str::to_owned)
        .ok_or_else(|| Error::plain(Code::Args, format!("policy.toml line {line}: expected a \"quoted\" value without backslashes")))
}

fn list(value: &str, line: usize) -> Result<Vec<String>> {
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or_else(|| Error::plain(Code::Args, format!("policy.toml line {line}: expected a list like [\"a\", \"b\"]")))?
        .trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    inner.split(',').map(|item| text(item.trim(), line)).collect()
}

impl Policy {
    pub(crate) fn parse(source: &str) -> Result<Self> {
        let mut policy = Self { capture: true, present: true, ..Self::default() };
        for (index, raw) in source.lines().enumerate() {
            let line = index + 1;
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            let (key, value) = raw.split_once('=').ok_or_else(|| Error::plain(Code::Args, format!("policy.toml line {line}: expected key = value")))?;
            let (key, value) = (key.trim(), value.trim());
            match key {
                "apps.allow" => policy.apps_allow = list(value, line)?,
                "apps.deny" => policy.apps_deny = list(value, line)?,
                "risky.extra" => policy.risky_extra = list(value, line)?,
                "codes.apps" => policy.codes_apps = list(value, line)?,
                "capture.allow" => {
                    policy.capture = match value {
                        "true" => true,
                        "false" => false,
                        _ => return Err(Error::plain(Code::Args, format!("policy.toml line {line}: capture.allow is true or false"))),
                    }
                }
                "actions.max_per_session" => {
                    policy.max_actions = Some(
                        value
                            .parse::<u32>()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or_else(|| Error::plain(Code::Args, format!("policy.toml line {line}: actions.max_per_session is a positive whole number")))?,
                    )
                }
                _ => return Err(Error::plain(Code::Args, format!("policy.toml line {line}: unknown key {key}"))),
            }
        }
        Ok(policy)
    }

    /// The policy in `root`, none when the file is absent.
    pub(crate) fn load(root: &Path) -> Result<Self> {
        match fs::read_to_string(root.join("policy.toml")) {
            Ok(source) => Self::parse(&source),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self { capture: true, ..Self::default() }),
            Err(e) => Err(Error::plain(Code::Args, format!("policy.toml cannot be read: {e}"))),
        }
    }

    /// Whether any spelling of the app (the name asked, its package, its launcher label) is in `list`.
    fn listed(list: &[String], names: &[&str]) -> bool {
        let names: Vec<String> = names.iter().map(|name| normalized(name)).collect();
        list.iter().any(|entry| names.contains(&normalized(entry)))
    }

    pub(crate) fn restricts_apps(&self) -> bool {
        !self.apps_allow.is_empty() || !self.apps_deny.is_empty()
    }

    /// Whether an app may be opened. `names` holds every spelling known for it; the first is shown
    /// in the refusal.
    pub(crate) fn app_ok(&self, names: &[&str]) -> Result<()> {
        let shown = names.first().copied().unwrap_or_default();
        let blocked = |why: String| Err(Error::plain(Code::Blocked, format!("Blocked by policy.toml: {why}. Nothing was run.")));
        if Self::listed(&self.apps_deny, names) {
            return blocked(format!("{shown} is in apps.deny"));
        }
        if !self.apps_allow.is_empty() && !Self::listed(&self.apps_allow, names) {
            return blocked(format!("{shown} is not in apps.allow"));
        }
        Ok(())
    }

    pub(crate) fn capture_ok(&self) -> Result<()> {
        if self.capture {
            Ok(())
        } else {
            Err(Error::plain(Code::Blocked, "Blocked by policy.toml: capture.allow is false. Nothing was run."))
        }
    }

    pub(crate) fn code_ok(&self, package: &str, title: &str) -> bool {
        Self::listed(&self.codes_apps, &[title, package])
    }

    pub(crate) fn describe(&self) -> String {
        if !self.present {
            return "No policy.toml; nothing is restricted beyond the built-in rules.".into();
        }
        let join = |list: &[String]| if list.is_empty() { "none".to_owned() } else { list.join(", ") };
        format!(
            "apps.allow: {}\napps.deny: {}\nrisky.extra: {}\nactions.max_per_session: {}\ncapture.allow: {}\ncodes.apps: {}",
            join(&self.apps_allow),
            join(&self.apps_deny),
            join(&self.risky_extra),
            self.max_actions.map_or("none".to_owned(), |n| n.to_string()),
            self.capture,
            join(&self.codes_apps)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# team policy\napps.allow = [\"com.android.settings\", \"Clock\"]\napps.deny = [\"com.example.bank\"]\nrisky.extra = [\"unsubscribe\", \"cancel\"]\nactions.max_per_session = 200\ncapture.allow = false\ncodes.apps = [\"Bank\"]\n";

    #[test]
    fn the_sample_parses_and_describes_itself() {
        let p = Policy::parse(SAMPLE).unwrap();
        assert_eq!((p.apps_allow.len(), p.max_actions, p.capture), (2, Some(200), false));
        assert!(p.describe().contains("apps.deny: com.example.bank") && p.describe().contains("capture.allow: false"));
        assert!(Policy::default().describe().starts_with("No policy.toml"));
    }

    #[test]
    fn a_bad_file_names_its_line_and_never_means_no_policy() {
        for (source, line) in [
            ("apps.allow = Settings", 1),
            ("\n\nactions.max_per_session = 0", 3),
            ("capture.allow = maybe", 1),
            ("nope = 1", 1),
            ("just words", 1),
            ("apps.deny = [\"a\\\\b\"]", 1),
        ] {
            let error = Policy::parse(source).unwrap_err();
            assert_eq!(error.code, Code::Args, "{source}");
            assert!(error.message.contains(&format!("line {line}")), "{source}: {}", error.message);
        }
    }

    #[test]
    fn apps_match_by_name_or_package_and_deny_wins() {
        let p = Policy::parse(SAMPLE).unwrap();
        assert!(p.app_ok(&["Settings", "com.android.settings"]).is_ok());
        assert!(p.app_ok(&["com.android.settings", "Settings"]).is_ok(), "the launcher label counts when the agent said the package");
        assert!(p.app_ok(&["Clock", "com.google.android.deskclock"]).is_ok());
        assert_eq!(p.app_ok(&["Chrome", "com.android.chrome"]).unwrap_err().code, Code::Blocked);
        assert!(p.app_ok(&["Bank", "com.example.bank"]).unwrap_err().message.contains("apps.deny"));
        let deny_only = Policy::parse("apps.deny = [\"Bank\"]").unwrap();
        assert!(deny_only.app_ok(&["Chrome", "com.android.chrome"]).is_ok());
        assert!(deny_only.app_ok(&["com.x", "Bank"]).is_err() && deny_only.app_ok(&["anything", "com.BANK"]).is_ok(), "names compare whole, not as substrings");
    }

    #[test]
    fn capture_and_codes_follow_the_file() {
        let p = Policy::parse(SAMPLE).unwrap();
        assert_eq!(p.capture_ok().unwrap_err().code, Code::Blocked);
        assert!(p.code_ok("com.bank", "Bank") && !p.code_ok("com.chat", "Chat"));
    }

    #[test]
    fn a_missing_file_is_the_default_and_a_broken_one_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        assert!(Policy::load(d.path()).unwrap().capture_ok().is_ok());
        fs::write(d.path().join("policy.toml"), "bogus").unwrap();
        assert!(Policy::load(d.path()).is_err());
    }
}
