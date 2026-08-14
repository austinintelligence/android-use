# Android Use

**Give AI an Android device.**

Android Use lets an AI see and control an Android phone or tablet using the same apps, buttons, text fields, and browser pages you use.

## What you need

- An Android phone or tablet.
- A USB cable and permission to approve this computer on the device.
- A Windows, macOS, or Linux computer.

You do not need to install Rust, Java, Gradle, or the Android SDK when using a release package.

## Install and connect

Download the Android Use release package for your computer, then run:

```text
android-use setup
```

The setup assistant finds one connected device, installs the Android Use helper, and checks that it is ready. If Android asks whether to trust this computer, unlock the device and tap **Allow**.

Android Use will tell you exactly when one approval is still needed:

```text
Settings → Accessibility → Android Use → On
```

Check the result at any time:

```text
android-use status
android-use doctor
```

Useful maintenance commands are:

```text
android-use update
android-use uninstall
```

`uninstall` removes the Android Use helper and Android Use's own local state. It does not remove unrelated Android tools or files.

If you are using the npm release, the same commands are available through `npx android-use setup`. A release archive is the easiest option until the npm package is published for your platform.

## Privacy and permissions

Android Use keeps normal control local to your computer and the connected device. It does not need Android internet access. Camera, microphone, location, notifications, screenshots, and screen recording are bounded and permission-aware. Android always controls the final permission prompt; Android Use never bypasses it.

Optional permissions stay optional. `android-use doctor` shows what is ready and what still needs attention.

## Using an AI agent

Start one of the agent transports when your client asks for it:

```text
au serve --mcp
au serve --jsonl
```

The agent has two small tools:

- `android.read` — status, semantic screen state, browser state, capabilities, notifications, location, artifacts, and visual hashes.
- `android.act` — short, generation-checked plans for Android, browser, and visual actions.

The safe loop is simple: read state, act with the returned generation, then verify when the task needs confirmation. Stale results must be observed again. An uncertain mutation is never replayed automatically.

Browser control uses a bounded Chrome connection. It returns compact tabs, page text, and interactive references instead of dumping raw HTML. Large screenshots, camera captures, audio, video, and diagnostics stay behind private artifact handles.

## Capabilities

| Area | Available |
| --- | --- |
| Android UI | Semantic observation, taps, text, scroll, keys, gestures, waits, assertions, app launch |
| Browser | Chrome tabs, navigation, page text, interactive elements, click/focus/type/key/scroll, waits, reload, back/forward, bounded evaluation |
| Media | Camera snapshots and microphone WAV artifacts when permission is granted |
| Device state | Location and compact notification reads; safe notification actions where Android exposes them |
| Screen recording | Bounded MP4 artifact after Android MediaProjection approval |
| Visual tools | Screenshot artifacts, bounded crop, structural hash, sampled diff |
| Interfaces | MCP, JSONL, and a small command-line interface |

## Project layout

The source tree is intentionally small and readable:

```text
computer/       computer-side Rust engine
device/         Android helper and example app
tools/          build, test, package, and release commands
install/        npm bootstrap package
skills/         agent instructions
images/         checked-in project images
```

Build output, local device state, and captures are ignored. They are not part of the release source tree.

## Developer checks

From a checkout with the development tools installed:

```text
cargo xtask verify
cargo xtask package
cargo xtask live
cargo xtask stress-live
cargo xtask benchmark
cargo xtask benchmark-live
```

`cargo xtask live` uses harmless Settings and Chrome flows. Privacy-sensitive capture tests require explicit permission on the connected device.

Read [SECURITY.md](SECURITY.md) before changing trust boundaries and [CONTRIBUTING.md](CONTRIBUTING.md) before sending a change.
