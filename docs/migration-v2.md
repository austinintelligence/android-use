# V2 migration

V2 is additive. Existing `au` commands, the v1 daemon protocol, tape format,
helper protocol, and `dev.codex.aubridge` package remain in place while agents
move to `au serve --jsonl` or `au serve --mcp`.

## Host state

The current `%LOCALAPPDATA%\Codex\android-use` root is retained on Windows for
v1 compatibility. macOS uses `~/Library/Application Support/android-use` and
Linux uses `${XDG_DATA_HOME:-~/.local/share}/android-use`. New files are isolated under `state/setup.json`,
`state/agent.json`, `state/agents/`, `state/operations/`, `state/remote.json`,
`recipes/`, and `artifacts/`. No migration changes the enrolled
`ro.serialno`, selected endpoint, helper token, forwards, saves, or artifacts.

Any future Windows root change requires an explicit copy-and-verify migration:
write the new root, compare file hashes, retain the old root as rollback, then
switch the configured root. The current release does not silently move it.

## Helper package

The package name and signing identity remain `dev.codex.aubridge` so Android
does not treat the helper as a new app, discard its private token, or require
the user to re-enable Accessibility and notification access. Product branding
can be neutral while the package migration is deferred until a signed,
user-visible export/import path exists.

## Skills and adapters

The generated core skill is still installed at the existing Codex location.
`au agent configure` writes only AU-owned adapter metadata under `state/agents`;
the installer does not guess or overwrite third-party configuration files.
Existing v1 skills continue to work, and the generated v2 contract is added as
the preferred path.

## Remote

`state/remote.json` is policy metadata only. Remote access remains disabled
until a separately audited broker, pairing flow, Keystore identity, encrypted
frame implementation, and revoke/replay test suite are shipped. A local helper
upgrade never grants Internet permission or enables remote access.
