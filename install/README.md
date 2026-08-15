# Android Use installer

The official installation path is the matching archive from [GitHub Releases](https://github.com/austinintelligence/android-use/releases). It gives people one setup command, verifies the bundled files for their computer, then opens the Android screens that still need the owner's approval.

For agent-driven setup, use the [copy-paste installation prompt](../docs/agents/install.md). It also explains how to register the skill in Codex, Cursor, Claude Code, and OpenClaw, and how to resume after USB debugging or Accessibility approval.

From the extracted release folder, use `./au` on macOS/Linux or `.\au.exe` in Windows PowerShell. For example:

```powershell
.\au.exe setup
.\au.exe status
.\au.exe doctor
```

This directory is the optional npm launcher source. It is not the primary install route until the package is published by its owner. Developers can set `AU_BIN` to a trusted local build when testing it.
