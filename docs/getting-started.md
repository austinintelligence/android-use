# Human Quickstart

This takes one Android device from “plugged in” to “ready for an AI agent.”

## Before you start

You need:

- Android 8 or newer.
- A Windows, macOS, or Linux computer with a release package available.
- A USB cable that carries data, not only power.
- Permission to enable Developer options, USB debugging, and the Android Use accessibility service.

On Android, open **Settings → About phone** and tap **Build number** seven times. Then open **System → Developer options** and turn on **USB debugging**. Names vary slightly by device maker.

## 1. Connect the device

Unlock Android, connect the cable, and approve **Allow USB debugging?**. Select “Always allow” only if you trust this computer.

```console
au devices
```

You should see one ready endpoint. If not, run `au doctor`.

## 2. Let Android Use finish setup

Keep the device unlocked:

```console
au setup
```

The command remembers the physical device, installs the Android Use helper, and starts it. Android Use cannot approve accessibility on your behalf. When prompted, open:

```text
Settings → Accessibility → Android Use → On
```

Return to the terminal and run:

```console
au status
```

## 3. Read the screen

```console
au observe
```

This returns a compact semantic frontier: a generation number plus visible labels, roles, and integer references. Machine-readable JSON is the default when output is redirected; add `--human` for friendly terminal output or `--json` to force JSON.

## 4. Connect your agent

Use MCP when the client supports it:

```console
au serve --mcp
```

Otherwise use one typed request per line:

```console
au serve --jsonl
```

Continue with the [Agent Quickstart](agents/quickstart.md) or [examples](../examples/README.md).

## Optional permissions

Camera, microphone, notifications, location, screenshots, and screen recording are not required for normal UI control. Android owns their permission screens. Check current support with:

```console
au capabilities
au doctor
```

Enable only what your task needs.
