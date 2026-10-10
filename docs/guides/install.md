# Install

Connect one Android device (or start an [emulator](emulators.md)), approve two Android prompts, and `au setup` does the rest.

## What you need

- Android 8 or newer, with a data-capable USB cable. No cable? Use an [emulator](emulators.md).
- macOS (Apple Silicon or Intel), Linux (x86_64 or arm64), or Windows (x86_64 or arm64).
- Permission to change developer and accessibility settings on the device.

## 1. Get `au`

Pick one. All of them give you the same `au` with the Android helper built in.

| Method | Command |
| --- | --- |
| npm (Node.js 20.11+) | `npx -y android-use setup` runs it without installing; `npm i -g android-use` keeps it |
| Homebrew (macOS, Linux) | `brew install austinintelligence/tap/android-use` |
| Script (macOS, Linux) | `curl -fsSL https://raw.githubusercontent.com/austinintelligence/android-use/main/install/install.sh \| sh` |
| Script (Windows PowerShell) | `irm https://raw.githubusercontent.com/austinintelligence/android-use/main/install/install.ps1 \| iex` |
| Archive | Download your platform's file from [Releases](https://github.com/austinintelligence/android-use/releases), check it against `SHA256SUMS`, and unpack it |

The scripts install the newest release (set `ANDROID_USE_VERSION` for another), verify the archive against `SHA256SUMS` before unpacking, and put `au` in `~/.local/bin` (`%LOCALAPPDATA%\AndroidUse\bin` on Windows; set `AU_INSTALL_DIR` to change it). The npm launcher picks the binary for your platform from `@androiduse/<platform>`.

## 2. Turn on USB debugging

On Android: **Settings → About phone**, tap **Build number** seven times, then **Developer options → USB debugging**. The wording varies by maker; search Settings for "Build number" if needed.

## 3. Plug in and approve

Connect the unlocked device and tap **Allow** on "Allow USB debugging?". Choose "Always allow from this computer" only on your own computer.

## 4. Run setup

```sh
au setup          # or: npx -y android-use setup
```

Keep the device unlocked. Setup remembers the device, installs the helper, opens the Accessibility screen, registers Android Use with every agent it finds, and waits while you tap **Android-Use**, turn it on, and approve Android's warning. It ends on:

```text
Android Use is ready
```

Nothing else is required when setup succeeds.

## Connect your agent

`au setup` already ran `au install` for every agent it found. To do one yourself: `au install claude` (also `codex`, `cursor`, `gemini`, `vscode`, `claude-desktop`, `opencode`). Then ask the agent: "Check the connected Android device and tell me what is on screen. Do not change anything yet." More in the [agent quickstart](../agents/quickstart.md).

## If setup stops

Run `au doctor`. It says what is missing in plain language. The usual causes are a charge-only cable, a locked device, a USB debugging prompt still waiting on Android, or Accessibility not yet approved. [Troubleshooting](troubleshooting.md) covers the rest.

Camera, microphone, notifications, location, and screen recording are optional; Android asks when a task first needs one.

## Update and remove

`au update` refreshes the helper after you upgrade `au`. `au uninstall` removes the helper from the device and Android Use's local state.
