# Install and connect android-use

This page is the practical first-run guide. If you are only trying to understand the project, start with the [root README](../README.md).

## 1. Install the skill

Install the Codex skill from the public repository:

```powershell
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
```

The skill contains the short operational instructions, the generated protocol contract, and the references an agent needs. It does not contain private device state, release binaries, or signing material.

## 2. Install the Windows host

When the npm package is available for the release you want:

```powershell
npx --yes android-use@latest install --agent codex
npx --yes android-use@latest doctor --json
```

The installer downloads a published HTTPS release manifest, checks the asset size and SHA-256 digest, stages the files, and activates them atomically. Use `update` for a newer verified release, `rollback` for the last verified version, and `uninstall` to remove the host state.

Until the package is published, use the GitHub skill source and the release assets managed by the repository owner. Do not treat an npm command as proof that a package is already available.

## 3. Connect Android

On the device, enable USB debugging or Wireless debugging and authorize the computer. Then discover and enroll the exact endpoint:

```powershell
au d
au u SERIAL_OR_ENDPOINT
au st
```

`au u` records the endpoint's reported `ro.serialno`. Later USB, Wi-Fi, and mDNS candidates are accepted only when they identify the same physical device.

## 4. Add the optional Android helper

The helper is needed for capabilities that Android does not expose through ordinary ADB alone:

- accessibility-based semantic UI actions;
- camera and microphone capture;
- notification access;
- mock-location testing.

Build it from the repository when needed:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-helper.ps1 -Release
au app install PATH_TO_APK
```

Grant only the capabilities required for the current task. The helper does not open its own network service; it communicates through the authenticated ADB-forwarded path owned by `au`.

## 5. Verify the first session

```powershell
au ui snap --compact --frontier
au ui find "text~Allow,clickable=true#0"
```

If the helper is not installed, basic coordinate control and read-only ADB commands can still be used.

## Updating and uninstalling

Use the installer commands for lifecycle operations:

```text
install | update | doctor | rollback | uninstall | print-path | version
```

`uninstall --purge --yes` removes versioned binaries and the installed skill. Use it only when you intentionally want to remove those files. Modified skill files are preserved rather than silently deleted.

## Common first-run problems

- **No device appears:** confirm debugging is enabled, the authorization prompt was accepted, and the device is visible to ADB.
- **The endpoint changed:** run discovery again and enroll the endpoint only after checking the reported hardware identity.
- **Semantic actions fail:** install the helper and grant Android Accessibility access.
- **Media or notification features fail:** check the specific Android permission and whether the OEM permits that capability in the background.
- **A Wi-Fi endpoint is slow:** use USB when possible. Identity-safe failover does not guarantee identical latency.

For the security model and release verification, see [`security.md`](security.md) and [`supply-chain.md`](supply-chain.md).

