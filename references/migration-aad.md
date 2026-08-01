# Migration from aad

`android-use` replaces the old display-oriented skill. The temporary legacy `aad.cmd` forwards only argument-free discovery and status commands to the Rust `au.exe` binary:

| Old | Replacement |
| --- | --- |
| `aad d` / `aad devices` | `au.exe d` |
| `aad doc` / `aad doctor` | `au.exe doctor` |
| `aad st` / `aad status` | `au.exe st` |

Argument-bearing legacy calls return an explicit `err E_MIGRATION` naming `au.exe`, because a `cmd.exe` `%*` wrapper cannot preserve arbitrary structured arguments. Invoke `au.exe` directly or use `scripts\\au.ps1` for URLs, text, selectors, JavaScript, file paths, and other structured values.

Former display/scene commands return `err E_MIGRATION` and never invoke PowerShell. Runtime state migrates once to `%LOCALAPPDATA%\\Codex\\android-use\\config.json`; a recoverable legacy snapshot is preserved in `state\\legacy-config-backup.json` and is not repeatedly overwritten.
