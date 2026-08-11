# android-use installer

This package installs the Windows x64 host and Codex skill for the `au` Android control stack.

```powershell
npx --yes android-use@latest install --agent codex
npx --yes android-use@latest doctor --json
```

Those commands are available after the release owner publishes the package to npm. Until then, install the skill from GitHub or run this package from the repository checkout; the GitHub release assets are already hash-pinned.

The installer downloads only a published HTTPS release manifest, verifies asset byte counts and SHA-256 hashes, stages files atomically, and keeps a rollback copy. Use `--with-helper` to retain the signed Android helper and `--install-helper` to install it on the enrolled device.

Supported commands:

```text
install | update | doctor | rollback | uninstall | print-path | version
```

Android semantic UI, media, notification, and mock-location features additionally require the `dev.codex.aubridge` helper and the user-granted capabilities described in the [installation guide](../../docs/installation.md).

Skill-only installation is also available through the Open Agent Skills CLI:

```powershell
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
```

Repository: https://github.com/austinintelligence/android-use

License: MIT.

