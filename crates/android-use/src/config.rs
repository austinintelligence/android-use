use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AuError, Result};

pub const CONFIG_SCHEMA: u32 = 1;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub root: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub artifacts: PathBuf,
    pub daemon: PathBuf,
    pub forwards: PathBuf,
    pub location_journal: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let local = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| AuError::code("E_ENV", "LOCALAPPDATA is not set"))?;
        let root = local.join("Codex").join("android-use");
        let state = root.join("state");
        let artifacts = root.join("artifacts");
        fs::create_dir_all(&state)?;
        fs::create_dir_all(&artifacts)?;
        Ok(Self {
            config: root.join("config.json"),
            daemon: state.join("daemon.json"),
            forwards: state.join("forwards.json"),
            location_journal: state.join("location-journal.json"),
            root,
            state,
            artifacts,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub hardware_serial: String,
    pub selected_endpoint: Option<String>,
    pub known_wifi_endpoints: Vec<String>,
    pub adb_path: Option<PathBuf>,
    pub migration: MigrationState,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: CONFIG_SCHEMA,
            // A fresh installation is deliberately unenrolled. A physical
            // device becomes trusted only after an explicit `au u ENDPOINT`
            // enrollment proves its ro.serialno.
            hardware_serial: String::new(),
            selected_endpoint: None,
            known_wifi_endpoints: Vec::new(),
            adb_path: None,
            migration: MigrationState::default(),
        }
    }
}

impl Config {
    pub fn enrolled_serial(&self) -> Option<&str> {
        (!self.hardware_serial.is_empty()).then_some(self.hardware_serial.as_str())
    }

    pub fn require_enrolled_serial(&self) -> Result<&str> {
        self.enrolled_serial().ok_or_else(|| {
            AuError::code(
                "E_ENROLL",
                "no Android device is enrolled; run au d, then au u ENDPOINT",
            )
        })
    }

    pub fn identity_matches(&self, serial: Option<&str>) -> bool {
        self.enrolled_serial() == serial
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MigrationState {
    pub legacy_imported: bool,
    pub legacy_backup: Option<String>,
}

pub fn load(paths: &AppPaths) -> Result<Config> {
    if paths.config.exists() {
        let text = fs::read_to_string(&paths.config)?;
        let mut config: Config = match serde_json::from_str(&text) {
            Ok(config) => config,
            Err(_) => {
                // Never overwrite unreadable user state. Preserve an exact copy,
                // then rebuild a canonical configuration once so subsequent calls
                // are not trapped behind the same corrupt file.
                backup_corrupt_config(paths, &text)?;
                let mut recovered = Config::default();
                migrate_legacy(paths, &mut recovered)?;
                migrate_legacy_adb_path(&mut recovered);
                save(paths, &recovered)?;
                return Ok(recovered);
            }
        };
        let mut changed = false;
        if config.schema != CONFIG_SCHEMA {
            backup_existing_config(paths, "schema")?;
            config.schema = CONFIG_SCHEMA;
            changed = true;
        }
        if !config.migration.legacy_imported {
            migrate_legacy(paths, &mut config)?;
            changed = true;
        }
        if migrate_legacy_adb_path(&mut config) {
            changed = true;
        }
        if changed {
            save(paths, &config)?;
        }
        return Ok(config);
    }

    let mut config = Config::default();
    migrate_legacy(paths, &mut config)?;
    migrate_legacy_adb_path(&mut config);
    save(paths, &config)?;
    Ok(config)
}

pub fn save(paths: &AppPaths, config: &Config) -> Result<()> {
    let bytes = serde_json::to_vec(config)?;
    atomic_write(&paths.config, &bytes)
}

fn migrate_legacy_adb_path(config: &mut Config) -> bool {
    let Some(path) = config.adb_path.as_ref() else {
        return false;
    };
    if !path.to_string_lossy().contains("android-agent-display") {
        return false;
    }
    let Some(local) = env::var_os("LOCALAPPDATA") else {
        return false;
    };
    let canonical = PathBuf::from(local)
        .join("Android")
        .join("Sdk")
        .join("platform-tools")
        .join("adb.exe");
    if !canonical.is_file() {
        return false;
    }
    config.adb_path = Some(canonical);
    true
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AuError::code("E_PATH", "path has no parent"))?;
    fs::create_dir_all(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuError::code("E_TIME", "system clock before epoch"))?
        .as_nanos();
    let temporary = parent.join(format!(".{}.{}.tmp", file_name(path), nonce));
    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(AuError::code(
            "E_CONFIG",
            format!("atomic replace {}: {error}", path.display()),
        ));
    }
    sync_parent(parent)?;
    Ok(())
}

fn sync_parent(parent: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = File::open(parent);
    }
    #[cfg(not(windows))]
    {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config")
        .to_owned()
}

fn backup_existing_config(paths: &AppPaths, reason: &str) -> Result<()> {
    let backup = paths.state.join("config.previous.json");
    if !backup.exists() && paths.config.exists() {
        fs::copy(&paths.config, backup)?;
    }
    let marker = paths.state.join(format!("config-upgrade-{reason}.marker"));
    if !marker.exists() {
        atomic_write(&marker, b"preserved")?;
    }
    Ok(())
}

fn backup_corrupt_config(paths: &AppPaths, contents: &str) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuError::code("E_TIME", "system clock before epoch"))?
        .as_nanos();
    let backup = paths.state.join(format!("config.corrupt-{nonce}.json"));
    atomic_write(&backup, contents.as_bytes())?;
    Ok(backup)
}

fn migrate_legacy(paths: &AppPaths, config: &mut Config) -> Result<()> {
    let local = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| AuError::code("E_ENV", "LOCALAPPDATA is not set"))?;
    let legacy_root = local.join("Codex").join("android-agent-display");
    migrate_legacy_from(paths, config, &legacy_root)
}

fn migrate_legacy_from(paths: &AppPaths, config: &mut Config, legacy_root: &Path) -> Result<()> {
    let candidates = [
        legacy_root.join("config.toml"),
        legacy_root.join("config.json"),
    ];
    let backup = paths.state.join("legacy-config-backup.json");
    let mut snapshot = serde_json::Map::new();
    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let text = fs::read_to_string(&candidate)?;
        snapshot.insert(candidate.display().to_string(), Value::String(text.clone()));
        import_legacy_text(config, &text);
    }
    if !snapshot.is_empty() {
        if !backup.exists() {
            atomic_write(&backup, &serde_json::to_vec(&snapshot)?)?;
        }
        config.migration.legacy_imported = true;
        config.migration.legacy_backup = Some(backup.display().to_string());
    } else {
        // Record an empty scan as complete too. Otherwise every command keeps
        // probing the retired tree and a valid canonical config never reaches
        // a stable migration state.
        config.migration.legacy_imported = true;
    }
    Ok(())
}

fn import_legacy_text(config: &mut Config, text: &str) {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        if let Some(serial) = value.get("serial").and_then(Value::as_str) {
            import_endpoint(config, serial);
        }
        if let Some(path) = value.get("adb_path").and_then(Value::as_str) {
            config.adb_path = Some(PathBuf::from(path));
        }
    }
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("serial=") {
            import_endpoint(config, value.trim());
        }
        if let Some(value) = line.strip_prefix("adb_path=") {
            config.adb_path = Some(PathBuf::from(value.trim()));
        }
    }
}

fn import_endpoint(config: &mut Config, endpoint: &str) {
    if endpoint.contains(':')
        && !config
            .known_wifi_endpoints
            .iter()
            .any(|item| item == endpoint)
    {
        config.known_wifi_endpoints.push(endpoint.to_owned());
    }
    config.selected_endpoint = Some(endpoint.to_owned());
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{atomic_write, load, migrate_legacy_from, AppPaths, Config};

    fn paths(root: &std::path::Path) -> AppPaths {
        AppPaths {
            root: root.to_path_buf(),
            config: root.join("config.json"),
            state: root.join("state"),
            artifacts: root.join("artifacts"),
            daemon: root.join("state/daemon.json"),
            forwards: root.join("state/forwards.json"),
            location_journal: root.join("state/location-journal.json"),
        }
    }

    #[test]
    fn atomic_write_replaces_contents() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("config.json");
        atomic_write(&path, b"one").expect("first");
        atomic_write(&path, b"two").expect("second");
        assert_eq!(fs::read(&path).expect("read"), b"two");
    }

    #[test]
    fn config_default_is_unenrolled() {
        assert_eq!(Config::default().enrolled_serial(), None);
    }

    #[test]
    fn app_paths_have_distinct_state_locations() {
        let root = tempfile::tempdir().expect("temp");
        let paths = paths(root.path());
        assert_ne!(paths.config, paths.forwards);
    }

    #[test]
    fn corrupt_configuration_is_preserved_then_recovered() {
        let root = tempfile::tempdir().expect("temp");
        let paths = paths(root.path());
        fs::create_dir_all(&paths.state).expect("state");
        fs::write(&paths.config, "not-json").expect("write corrupt config");
        let recovered = load(&paths).expect("recover config");
        assert_eq!(recovered.enrolled_serial(), None);
        assert!(fs::read_to_string(&paths.config)
            .expect("canonical config")
            .contains("hardware_serial"));
        let preserved = fs::read_dir(&paths.state)
            .expect("state entries")
            .flatten()
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("config.corrupt-")
            });
        assert!(preserved);
    }

    #[test]
    fn legacy_migration_is_idempotent_and_keeps_one_backup() {
        let root = tempfile::tempdir().expect("temp");
        let paths = paths(root.path());
        fs::create_dir_all(&paths.state).expect("state");
        let legacy = root.path().join("legacy");
        fs::create_dir_all(&legacy).expect("legacy");
        fs::write(
            legacy.join("config.toml"),
            "serial=192.0.2.103:42511\nadb_path=C:\\old\\adb.exe\n",
        )
        .expect("legacy config");
        let mut config = Config::default();
        migrate_legacy_from(&paths, &mut config, &legacy).expect("first migration");
        let backup = config.migration.legacy_backup.clone().expect("backup path");
        migrate_legacy_from(&paths, &mut config, &legacy).expect("second migration");
        assert_eq!(
            config.selected_endpoint.as_deref(),
            Some("192.0.2.103:42511")
        );
        assert_eq!(config.known_wifi_endpoints, ["192.0.2.103:42511"]);
        assert_eq!(
            config.migration.legacy_backup.as_deref(),
            Some(backup.as_str())
        );
        assert!(std::path::Path::new(&backup).is_file());
    }

    #[test]
    fn empty_legacy_scan_is_marked_complete() {
        let root = tempfile::tempdir().expect("temp");
        let paths = paths(root.path());
        fs::create_dir_all(&paths.state).expect("state");
        let legacy = root.path().join("missing-legacy");
        let mut config = Config::default();
        migrate_legacy_from(&paths, &mut config, &legacy).expect("empty migration");
        assert!(config.migration.legacy_imported);
        assert!(config.migration.legacy_backup.is_none());
    }
}
