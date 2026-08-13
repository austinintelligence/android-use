<p align="center">
  <img src="assets/wordmark.svg" alt="Android Use" width="720">
</p>

# Give your agent an Android device

Android Use lets AI agents see and control Android phones, tablets, emulators, and Android-based devices. It turns the screen into small, meaningful observations and uses bounded, verifiable actions instead of flooding the agent with screenshots or raw ADB output.

## Set it up

Install the agent skill with one copy-and-paste command:

```sh
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
```

Then tell the agent: **“Use Android Use to set up this computer and my connected Android device. Walk me through anything you cannot safely automate.”** The skill checks the available release path, handles the computer-side work it can verify, and gives plain-language directions for the Android steps.

The guided NPX installer is built and tested in this repository but is not yet published to npm. Do not use `npx android-use@latest` until the package is published. The current GitHub prerelease only contains the Windows x64 host and helper; the agent will explain that limitation instead of pretending a missing package exists.

Android will ask you to approve a few things on the device. Accept the USB debugging prompt. When AU Bridge opens, enable Accessibility so the agent can understand the screen. Camera, microphone, notifications, and location stay off until you choose to enable them.

When a supported host is installed, check the connection:

```sh
au ready
```

If it is not ready, run:

```sh
au doctor --repair
```

See the [step-by-step setup guide](docs/people/getting-started.md) for pictures-in-words instructions and common fixes.

## Copy this to an agent

```text
Set up Android Use from https://github.com/austinintelligence/android-use.
First install the skill with:
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
Then use $android-use. Verify which public installer or release assets are
actually available before running them. Do everything you safely can on the
computer. Pause only when Android needs me
to unlock the device, approve USB debugging, or enable an AU Bridge permission.
After each pause, tell me exactly what to tap in plain language, then continue.
Finish by running `au ready --json` when `au` is installed, and explain any
capability or release asset that is not ready.
Do not bypass Android security prompts or change unrelated device settings.
```

## What agents can do

- Read the visible interface as compact labels, controls, and stable references.
- Tap, type, scroll, swipe, use system buttons, and wait for a result.
- Open and manage apps, links, browser tabs, files, and notifications.
- Capture bounded screenshots, recordings, camera, microphone, and location data after approval.
- Reuse a warm connection for fast multi-step work and multiple devices.
- Prove outcomes with assertions and receipts instead of assuming a tap worked.

The user remains in control. Android Use does not bypass USB authorization, Accessibility consent, runtime permissions, lock screens, enterprise policy, or app security.

## Choose your documentation

- [People](docs/people/README.md): setup, daily use, permissions, and troubleshooting in plain language.
- [Agents](docs/agents/README.md): the operating loop, contract, safety rules, recipes, and adapter setup.
- [Developers](docs/developers/README.md): source layout, architecture, testing, and releases.
- [Skill source](skills/android-use/SKILL.md): the compact instructions installed into compatible agents.

## How it works

The native `au` command keeps ADB, the Android helper, and optional browser sessions warm. An agent observes the current screen, executes one bounded semantic plan, then verifies the postcondition. Large files and media stay in managed artifact storage instead of being copied into chat.

For MCP-compatible tools and generic agents:

```sh
au serve --mcp
# or
au serve --jsonl
```

The stable methods are `android.status`, `android.observe`, `android.execute`, `android.artifact`, and `android.recipe`.

## Supported systems

Android Use ships native host builds for Windows, macOS, and Linux on x64 and ARM64. Managed Android platform tools are available on Windows x64, macOS, and Linux x64. Windows ARM64 and Linux ARM64 use a compatible existing ADB installation.

The AU Bridge helper supports Android 8 and newer. Device makers customize Android, so individual media, notification, browser, and location features can vary. `au doctor --json` reports the capabilities available on the connected device.

## Build from source

You need Rust, Node.js, JDK 17, and the Android SDK. Start with the [developer guide](docs/developers/README.md), then run:

```sh
cargo test --workspace --all-targets
npm test
```

Android Use is available under the [MIT License](LICENSE). Security issues should be reported through [the private security process](SECURITY.md).
