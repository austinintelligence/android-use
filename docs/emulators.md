# Emulators and targets

Android Use can drive an Android emulator it creates for you, side by side with your real phone. Each emulator is its own **target**: it has its own enrollment, journal, artifacts, and serve lock, so one agent session per target never interferes with another.

An emulator is a test device. It has no accounts, no personal data, and no real purchases, which makes it the safest place to let an agent explore.

## Prerequisites

The Android SDK with the `emulator` package and one `google_apis` system image that matches your computer (arm64 on Apple silicon, x86_64 elsewhere). `au emulator doctor` checks this and prints the exact `sdkmanager` command for whatever is missing. No JDK or `avdmanager` is needed; Android Use writes the virtual device files itself.

## Create, start, use

```bash
au emulator doctor
au emulator create bench --profile small
au emulator start bench
au --target bench setup --no-agents
```

`setup` installs the helper and opens Android's Accessibility screen. Android requires you to turn the service on yourself, on the emulator window or through any visible flow; Android Use never enables it silently. After that:

```bash
au --target bench read screen
au --target bench act 'open app "Settings"'
```

For an agent, register the target once with its MCP command: `au --target bench serve --mcp`.

| Profile | Screen | RAM | Cores |
| --- | --- | --- | --- |
| `small` (default) | 720×1600, 320 dpi | 2 GB | 2 |
| `standard` | 1080×2400, 420 dpi | 4 GB | 4 |

The emulator starts headless; add `--window` to `au emulator start` to watch it. `AU_TARGET=NAME` is the same as `--target NAME`. Without either, commands use the default target, which is your enrolled phone.

## Commands

| Command | Effect |
| --- | --- |
| `au emulator doctor` | Check the SDK, emulator binary, and system images. |
| `au emulator images` | List installed images and whether each runs on this computer. |
| `au emulator create NAME [--image ID] [--profile small\|standard]` | Create a virtual device. The newest usable `google_apis` image is chosen by default. |
| `au emulator start NAME [--window]` | Boot it on a free port pair and wait until Android finishes booting. |
| `au emulator stop NAME` | Shut it down. Its data is kept. |
| `au emulator status NAME` / `au emulator list` | Show state and endpoint. |
| `au emulator destroy NAME --yes` | Delete the device, its data, and its target state. |
| `au targets` | List the default target and every emulator target. |

## How targets stay separate

- **Identity.** An emulator's serial changes with its port, so Android Use writes a random per-target identifier into the emulator's global settings and enrolls that instead. A restart on a different port is still the same device. A different emulator, or your phone, is never mistaken for it.
- **One session per target.** `au serve` holds an exclusive lock on its target. A second `serve` for the same target fails with a clear message; a serve for another target is unaffected.
- **Ports.** Emulators take even console ports from 5554 upward (limit: 16 instances). Port choice and spawn happen under one lock, so two simultaneous starts cannot pick the same pair.
- **State.** Everything lives under `<AU_HOME>/targets/NAME` and `<AU_HOME>/emulators`. `au emulator destroy` removes it all.

## Measured on one Mac

Apple silicon, Android 16 (API 36) `google_apis` arm64 image, `small` profile, headless: cold boot to `sys.boot_completed` in about 38 s. It ran beside another emulator already enrolled as the default target on 5554 without interference, and stopped and restarted with its enrollment intact. Your numbers depend on disk, RAM, and image.

## What this does not do

Android Use does not turn on Accessibility for you and does not bypass Android's consent screens, on emulators or phones. Provisioning that would skip them is deliberately not built. A shared or remote emulator broker is out of scope; see [Security](../SECURITY.md).
