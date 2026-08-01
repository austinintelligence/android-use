# Installation

## Skill installation

```powershell
npx skills add drperky20/android-use --skill android-use -g -a codex -y
```

The skill payload is `skills/android-use`. It contains the short operational `SKILL.md`, generated protocol contract, `agents/openai.yaml`, and directly linked references. It intentionally does not contain release binaries, private state, or signing material.

## Host installer

```powershell
npx --yes android-use@latest install --agent codex
npx --yes android-use@1.0.0 doctor --json
```

Use `--with-helper` to retain a verified helper APK in the version store and `--install-helper` to install it using the active `au` enrollment. Use `update` for a new release, `rollback` for the last verified version, `uninstall` to remove owned host state, and `uninstall --purge --yes` only when intentionally removing versioned binaries and the installed skill. Modified skill files are preserved rather than silently deleted.

The installer accepts an explicit manifest with `--manifest PATH` for offline/release tests. Public release assets must use HTTPS and a manifest SHA-256 plus byte count. Downloads are streamed with a size bound, staged, hashed, and activated only after verification.

## Helper

Build the helper with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-helper.ps1 -Release
```

The first build creates a persistent machine-local signing key under `%LOCALAPPDATA%\Codex\android-use\keys`, outside the checkout. Install the APK with `au app install PATH_TO_APK`, then grant only the capabilities required for the current task: Accessibility for semantic UI; camera/microphone runtime permissions for media; notification access for notifications; and mock-location authorization for location tests.

## ADB setup

Enable USB debugging or Wireless debugging on the Android device, authorize the host, then:

```powershell
au d
au u SERIAL_OR_ENDPOINT
```

The public project does not assume a particular device. Wi-Fi/mDNS is accepted only after the endpoint reports the same hardware serial as the enrolled endpoint.
