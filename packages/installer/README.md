# android-use

Windows x64 installer for the `au` Android control stack.

```powershell
npx --yes android-use@latest install --agent codex
npx --yes android-use@latest doctor --json
```

The installer downloads only a published HTTPS release manifest, verifies the
asset byte counts and SHA-256 hashes, stages files atomically, and keeps a
rollback copy. Use `--with-helper` to retain the signed Android helper and
`--install-helper` to install it on the currently enrolled device.

Supported commands:

```text
install | update | doctor | rollback | uninstall | print-path | version
```

This package installs the Windows x64 host and Codex skill. Android semantic
UI, media, notifications, and mock-location features additionally require the
`dev.codex.aubridge` helper and the user-granted Android capabilities described
in the repository documentation.

Skill-only installation is also available through the Open Agent Skills CLI:

```powershell
npx skills add drperky20/android-use --skill android-use -g -a codex -y
```

Repository: https://github.com/drperky20/android-use

License: MIT.
