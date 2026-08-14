# Get Android Use ready

The goal is simple: connect one Android device, approve two Android prompts, and let `au setup` handle the rest.

## What you need

- A phone or tablet running Android 8 or newer.
- A USB cable that carries data.
- A Windows, macOS, or Linux computer.
- Permission to change developer and accessibility settings on the Android device.

> Android Use v3 packages are not published yet. This page describes the finished package flow. If you are working from the current repository, [build it from source](development.md) first.

## 1. Turn on USB debugging

This is the only setting Android Use cannot open before the computer is trusted.

On Android:

1. Open **Settings → About phone**.
2. Tap **Build number** seven times.
3. Go back and open **Developer options**.
4. Turn on **USB debugging**.

The wording can vary slightly on Samsung, Pixel, and other devices. Search Settings for “Build number” or “USB debugging” if needed.

## 2. Plug in and approve

Connect the unlocked device. When Android asks **Allow USB debugging?**, tap **Allow**.

Only choose **Always allow from this computer** when it is your computer.

## 3. Run setup

From the unzipped Android Use package:

```console
au setup
```

Keep the device unlocked. Setup will:

- find and remember the connected device;
- install the Android Use helper;
- open the correct Accessibility screen;
- wait while you turn on **Android Use**;
- confirm when the device is ready.

You should finish on:

```text
Android Use is ready
```

That is it. You do not need to run a separate status or doctor command when setup succeeds.

## Connect your AI agent

Use Android Use as a local MCP server:

```console
au serve --mcp
```

Then ask your agent:

```text
Check the connected Android device and tell me what is on screen. Do not change anything yet.
```

See the [Agent Quickstart](agents/quickstart.md) for client setup.

## If setup stops

Run:

```console
au doctor
```

It tells you what is missing in plain language. The most common causes are a charge-only cable, a locked device, a USB debugging prompt waiting on Android, or Accessibility not yet approved.

Camera, microphone, notifications, location, and screen recording are optional. Normal app control does not require them.
