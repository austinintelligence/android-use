# android-use

Give an AI agent an Android device. This package is the launcher for [Android Use](https://github.com/austinintelligence/android-use): it runs the `au` binary for your computer, which reads the screen, taps, types, opens apps, and drives Chrome on one real device or a local emulator.

```sh
npx -y android-use@1.3.0 setup
```

`setup` installs the small Android-Use helper on the device, registers the MCP server with every agent it finds (Claude Code, Codex, Cursor, Gemini CLI, VS Code, Claude Desktop, OpenCode), and finishes with a live read of the screen. Android asks you to approve USB debugging and the Accessibility service; Android Use never bypasses those prompts.

Then ask your agent: "Open Settings and tell me which Wi-Fi network is connected."

## Commands

`npx android-use <command>` runs any `au` command: `setup`, `doctor`, `status`, `install`, `serve --mcp`, `read`, `act`, `update`, `uninstall`. Run it with no device to see `au doctor` explain what is missing. For emulators, see `au emulator`.

## Integrity

Each package carries a `manifest.json` with the SHA-256 of the files it ships. Before running anything, the launcher checks the `au` binary (and the helper APK, when listed) against it, and exits 1 with a message if the manifest or this platform's entry is missing or a digest differs; it never runs an unchecked binary. `AU_BIN=/path/to/au` overrides the binary, skips the check, and prints a warning to stderr on every run. SIGINT, SIGTERM, and SIGHUP are forwarded to `au`, and the launcher exits with its code. `au setup` run through `npx` registers `npx -y android-use@1.3.0 serve --mcp` (the version you ran) with your agents, not a path in the npx cache.

## Platforms

| Computer | Package |
| --- | --- |
| macOS Apple Silicon / Intel | `@androiduse/darwin-arm64`, `@androiduse/darwin-x64` |
| Linux x86_64 / arm64 | `@androiduse/linux-x64`, `@androiduse/linux-arm64` |
| Windows x86_64 / arm64 | `@androiduse/win32-x64`, `@androiduse/win32-arm64` |

npm installs the one that matches your computer. Requires Node.js 20.11 or newer and an Android 8+ device with USB debugging.

Documentation, security model, and source: <https://github.com/austinintelligence/android-use>. MIT licensed.
