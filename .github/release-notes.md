# android-use 1.0.0

Android Use gives an AI agent bounded, structured access to one real Android device. Connect by USB, run `au setup`, approve Android's prompts, and let an agent read the interface, use apps, control supported Chrome sessions, and inspect supported device state.

## Install

Download the archive for your computer, unzip it, connect an unlocked Android device with USB debugging enabled, then run:

```console
au setup
```

The supported release archives are Windows x86_64, macOS Apple Silicon, and Linux x86_64. Each archive includes the matching Android helper. The helper APK is also included separately for audited or managed deployment.

## What is included

- `au`: the local CLI, MCP server, and JSONL agent transport.
- Compact semantic UI reading and generation-checked Android actions.
- Supported Chrome/CDP controls, screenshots, and local private artifacts.
- Optional camera, microphone, notification, location, and screen-recording capabilities when Android permission and device support are present.
- `SHA256SUMS`, an SPDX SBOM, a release manifest, and GitHub build attestations.

## Important security notes

USB debugging and access to `au serve` are device-control authority. Keep them local and private. Android Use has no built-in remote broker or multi-user authorization layer. Read [SECURITY.md](https://github.com/austinintelligence/android-use/blob/main/SECURITY.md) before exposing any transport beyond your computer.
