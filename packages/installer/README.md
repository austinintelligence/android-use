# android-use

Verified Windows, macOS, and Linux x64/ARM64 installer for the `au` Android control stack.

```sh
npx --yes android-use@latest install --agent codex
npx --yes android-use@latest doctor --json
```

These commands are the published-package path. If npm does not yet contain the
requested version, install the skill from GitHub or run this package from the
repository checkout instead; do not treat an unpublished `@latest` command as
an available installation method.

The installer downloads only a published HTTPS release manifest, verifies the
asset byte counts and SHA-256 hashes, stages files atomically, and keeps a
rollback copy. Use `--with-helper` to retain the signed Android helper and
`--install-helper` to install it on the currently enrolled device.

Supported commands:

```text
install | update | setup | ready | doctor | rollback | uninstall | print-path | version
```

This package selects a native host asset and installs the Codex skill. Android semantic
UI, media, notifications, and mock-location features additionally require the
`dev.codex.aubridge` helper and the user-granted Android capabilities described
in the repository documentation.

Skill-only installation is also available through the Open Agent Skills CLI:

```sh
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
```

Repository: https://github.com/austinintelligence/android-use

License: MIT.
