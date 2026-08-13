# Installation

## Skill installation

```sh
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
```

The skill payload is `skills/android-use`. It contains the short operational `SKILL.md`, generated protocol contract, `agents/openai.yaml`, and directly linked references. It intentionally does not contain release binaries, private state, or signing material.

## Host installer

```sh
npx --yes android-use@latest setup --agent auto --wait
npx --yes android-use@1.0.0 doctor --json
```

These commands require the matching version to be published to npm. Before
publication, use the GitHub skill or run `packages/installer/cli.mjs` from a
checkout; the installer will not silently fall back from an unavailable npm
package to an unverified source.

`setup` detects Windows, macOS, or Linux and x64/ARM64, selects that exact
asset from an Ed25519-signed release manifest, verifies its pinned signing key,
makes Unix binaries executable, installs a managed PATH entry,
keeps official platform-tools under the AU root when Google publishes a
compatible archive, installs the signed helper after ADB authorization, and
resumes the Rust setup journal. It is idempotent. Android-required settings
such as Accessibility and notification access still require the user to
approve them on the device:

```powershell
au setup --wait
au ready --json
au doctor --repair --json
```

Use `install --with-helper` when you want to stage an APK without installing it. `--install-helper` installs the staged APK using the active enrollment. Use `update` for a new release, `rollback` for the last verified version, `uninstall` to remove owned host state, and `uninstall --purge --yes` only when intentionally removing versioned binaries and the installed skill. Modified skill files are preserved rather than silently deleted.

The installer accepts an explicit manifest with `--manifest PATH` for
offline/release tests. Test-only OS/CPU overrides are honored only with that
local manifest, never for network installs. Network manifests require a
detached Ed25519 signature verified by the public key embedded in the package;
each public asset must use HTTPS and carry a SHA-256 plus byte count. Downloads
are streamed with a size bound, staged, hashed, and activated only after
verification. Activation uses a transaction journal and restores the entire
prior install after interruption rather than leaving a split host/helper state.

Default roots are `%LOCALAPPDATA%\Codex\android-use` on Windows,
`~/Library/Application Support/android-use` on macOS, and
`${XDG_DATA_HOME:-~/.local/share}/android-use` on Linux. `AU_INSTALL_ROOT` and
`AU_STATE_ROOT` are explicit overrides. macOS/Linux link the managed binary
from `~/.local/bin`; setup adds one marked PATH line to `.zprofile` or
`.profile` and never overwrites a non-owned link.

Official platform-tools archives are managed on Windows x64, macOS universal,
and Linux x64. Windows ARM64 and Linux ARM64 use an existing compatible `adb`
from `ANDROID_SDK_ROOT`, `ANDROID_HOME`, a standard SDK location, or PATH.

## Runtime-free packages

Tagged releases are prepared with per-user Windows x64/ARM64 MSI installers,
portable Windows ZIPs, macOS Intel/Apple Silicon tarballs, Linux x64/ARM64
tarballs, Debian packages, RPM packages, a Homebrew-ready formula, and a
Winget-ready three-file manifest. The portable archives and OS packages contain
the native host, the signed helper APK, license/notices, and a digest manifest;
they do not bundle Google platform-tools. MSI install/uninstall and user-PATH
ownership are release-gated. The NPX entrypoint remains the full resumable
bootstrapper when managed platform-tools and agent-skill installation are
desired.

## Helper

Build the helper with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-helper.ps1 -Release
```

The first build creates a persistent machine-local signing key under `%LOCALAPPDATA%\Codex\android-use\keys`, outside the checkout. Install the APK with `au app install PATH_TO_APK`, then grant only the capabilities required for the current task: Accessibility for semantic UI; camera/microphone runtime permissions for media; notification access for notifications; and mock-location authorization for location tests.

## ADB setup

Enable USB debugging or Wireless debugging on the Android device, authorize the host, then:

```sh
au d
au u SERIAL_OR_ENDPOINT
```

The public project does not assume a particular device. Wi-Fi/mDNS is accepted only after the endpoint reports the same hardware serial as the enrolled endpoint.

## Agent contract

The default agent surface is the one canonical contract shared by Codex, MCP-compatible clients, and generic JSONL agents:

```sh
au serve --mcp
au serve --jsonl
au schema --json
au agent configure auto --json
```

The contract is `android.status`, `android.observe`, `android.execute`, `android.artifact`, and `android.recipe`. The compatibility CLI, tape protocol, raw ADB, and raw shell remain available for explicit local use but are not part of the safe contract. See [`docs/agent-contract.md`](agent-contract.md).
