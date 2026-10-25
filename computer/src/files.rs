//! `push file "NAME"` / `pull file "NAME"`: move one file between the folder `files.dir` in
//! `policy.toml` and the phone's Downloads folder. Off unless that folder is set, plain file names
//! only (no paths), nothing outside the folder, nothing over 64 MB, and a pull never overwrites.
use crate::{
    api::{Code, Error, Result},
    engine::Engine,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const REMOTE_DIR: &str = "/sdcard/Download";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Transfer {
    Push(String),
    Pull(String),
}

/// `None` when the text is another command.
pub(crate) fn parse(command: &str) -> Option<Result<Transfer>> {
    let command = command.trim();
    let (push, rest) = if let Some(rest) = command.strip_prefix("push file ") { (true, rest) } else { (false, command.strip_prefix("pull file ")?) };
    let usage = || Error::new(Code::Args, r#"use: push file "NAME" or pull file "NAME""#);
    let name = rest.trim().strip_prefix('"').and_then(|body| body.strip_suffix('"')).filter(|name| !name.contains('"'));
    Some(match name {
        Some(name) => safe_name(name).map(|name| if push { Transfer::Push(name) } else { Transfer::Pull(name) }),
        None => Err(usage()),
    })
}

/// A plain file name: no folders, no leading dot, nothing that edits an adb command line.
fn safe_name(name: &str) -> Result<String> {
    let ok = !name.is_empty()
        && name.chars().count() <= 100
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
        && !name.contains("..");
    if ok {
        Ok(name.to_owned())
    } else {
        Err(Error::new(Code::Args, "a file name is 1 to 100 letters, digits, spaces, . _ - with no folders"))
    }
}

fn sha256(path: &Path) -> Result<String> {
    Ok(Sha256::digest(fs::read(path)?).iter().map(|b| format!("{b:02x}")).collect())
}

/// The file to send, checked to be a regular file inside `dir` and small enough.
pub(crate) fn plan_push(dir: &Path, name: &str) -> Result<(PathBuf, String)> {
    let path = dir.join(name);
    let (real, base) = (fs::canonicalize(&path).map_err(|_| Error::plain(Code::Missing, format!("There is no file \"{name}\" in the transfer folder.")))?, fs::canonicalize(dir)?);
    if !real.starts_with(&base) || !real.is_file() {
        return Err(Error::plain(Code::Blocked, format!("\"{name}\" is not a plain file inside the transfer folder.")));
    }
    if fs::metadata(&real)?.len() > MAX_BYTES {
        return Err(Error::plain(Code::Bounds, "Files over 64 MB are not transferred."));
    }
    Ok((real.clone(), sha256(&real)?))
}

/// Where a pulled file lands; refuses to replace one.
pub(crate) fn plan_pull(dir: &Path, name: &str) -> Result<PathBuf> {
    let dest = dir.join(name);
    if fs::symlink_metadata(&dest).is_ok() {
        return Err(Error::plain(Code::Blocked, format!("\"{name}\" already exists in the transfer folder; a pull never replaces a file.")));
    }
    Ok(dest)
}

pub(crate) fn run(engine: &mut Engine, transfer: &Transfer) -> Result<String> {
    let dir = engine.guard.policy.files_dir.clone().ok_or_else(|| Error::plain(Code::Blocked, "File transfer is off. A person turns it on with files.dir in policy.toml."))?;
    let dir = PathBuf::from(dir);
    let slow = Duration::from_secs(120);
    match transfer {
        Transfer::Push(name) => {
            let (path, digest) = plan_push(&dir, name)?;
            let remote = format!("{REMOTE_DIR}/{name}");
            engine.adb(&["push", &path.display().to_string(), &remote], slow)?;
            Ok(format!("Pushed {name} to Downloads (sha256 {digest})."))
        }
        Transfer::Pull(name) => {
            let dest = plan_pull(&dir, name)?;
            let remote = format!("{REMOTE_DIR}/{name}");
            let stat = String::from_utf8_lossy(&engine.adb(&["shell", "stat", "-c", "%F|%s", &format!("'{remote}'")], Duration::from_secs(20))?).trim().to_owned();
            let size =
                remote_file_size(&stat).ok_or_else(|| Error::plain(Code::Missing, format!("There is no file \"{name}\" in the phone's Downloads (a folder is not pulled).")))?;
            if size > MAX_BYTES {
                return Err(Error::plain(Code::Bounds, "Files over 64 MB are not transferred."));
            }
            // Pulled aside first (names never start with a dot, so this is no one's file), checked
            // again once whole, since the phone's file can change after the stat, then copied to a
            // name that must still be free.
            let part = dir.join(format!(".{name}.part"));
            let _ = fs::remove_file(&part);
            let pulled = engine.adb(&["pull", &remote, &part.display().to_string()], slow).and_then(|_| publish(&part, &dest, name));
            let _ = fs::remove_file(&part);
            pulled?;
            Ok(format!("Pulled {name} from Downloads (sha256 {}).", sha256(&dest)?))
        }
    }
}

/// Moves a finished pull to `dest` without ever replacing a file there.
fn publish(part: &Path, dest: &Path, name: &str) -> Result<()> {
    if fs::metadata(part)?.len() > MAX_BYTES {
        return Err(Error::plain(Code::Bounds, "Files over 64 MB are not transferred."));
    }
    let mut out = fs::File::options()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|_| Error::plain(Code::Blocked, format!("\"{name}\" already exists in the transfer folder; a pull never replaces a file.")))?;
    std::io::Write::write_all(&mut out, &fs::read(part)?)?;
    Ok(())
}

/// The size from `stat -c '%F|%s'`, only for a regular file: a folder's own size says nothing about
/// what `adb pull` would copy out of it.
fn remote_file_size(stat: &str) -> Option<u64> {
    let (kind, size) = stat.split_once('|')?;
    kind.starts_with("regular").then(|| size.trim().parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_regular_remote_file_has_a_size() {
        assert_eq!(super::remote_file_size("regular file|1234"), Some(1234));
        assert_eq!(super::remote_file_size("regular empty file|0"), Some(0));
        assert_eq!(super::remote_file_size("directory|4096"), None);
        assert_eq!(super::remote_file_size("stat: No such file"), None);
    }

    use super::*;

    #[test]
    fn commands_parse_and_names_cannot_hold_paths_or_shell_text() {
        assert_eq!(parse(r#"push file "receipt.pdf""#).unwrap().unwrap(), Transfer::Push("receipt.pdf".into()));
        assert_eq!(parse(r#"pull file "scan 1.jpg""#).unwrap().unwrap(), Transfer::Pull("scan 1.jpg".into()));
        assert!(parse(r#"tap "push file""#).is_none() && parse("push files").is_none());
        for bad in [
            r#"push file receipt"#,
            r#"push file "../etc/passwd""#,
            r#"push file "a/b""#,
            r#"push file ".hidden""#,
            r#"push file "a;rm""#,
            r#"push file "a$(x)""#,
            r#"push file "a..b""#,
            r#"push file """#,
            r#"push file "a" extra"#,
        ] {
            assert!(parse(bad).unwrap().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_push_needs_a_plain_file_inside_the_folder() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.txt"), "hello").unwrap();
        fs::create_dir(d.path().join("sub")).unwrap();
        let (path, digest) = plan_push(d.path(), "a.txt").unwrap();
        assert!(path.ends_with("a.txt") && digest == "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824", "{digest}");
        assert_eq!(plan_push(d.path(), "missing.txt").unwrap_err().code, Code::Missing);
        assert_eq!(plan_push(d.path(), "sub").unwrap_err().code, Code::Blocked, "a folder is not a file");
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("secret.txt"), "x").unwrap();
            std::os::unix::fs::symlink(outside.path().join("secret.txt"), d.path().join("link.txt")).unwrap();
            assert_eq!(plan_push(d.path(), "link.txt").unwrap_err().code, Code::Blocked, "a link out of the folder is refused");
        }
    }

    #[test]
    fn a_pull_never_replaces_a_file() {
        let d = tempfile::tempdir().unwrap();
        assert!(plan_pull(d.path(), "new.jpg").unwrap().ends_with("new.jpg"));
        fs::write(d.path().join("old.jpg"), "x").unwrap();
        assert_eq!(plan_pull(d.path(), "old.jpg").unwrap_err().code, Code::Blocked);
        // A file that appears while the pull runs is not replaced either.
        fs::write(d.path().join(".new.jpg.part"), "pulled").unwrap();
        assert_eq!(publish(&d.path().join(".new.jpg.part"), &d.path().join("old.jpg"), "old.jpg").unwrap_err().code, Code::Blocked);
        assert_eq!(fs::read_to_string(d.path().join("old.jpg")).unwrap(), "x");
        publish(&d.path().join(".new.jpg.part"), &d.path().join("new.jpg"), "new.jpg").unwrap();
        assert_eq!(fs::read_to_string(d.path().join("new.jpg")).unwrap(), "pulled");
    }
}
