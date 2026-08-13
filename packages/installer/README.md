# Android Use installer

This package provides the intended guided setup command:

```sh
npx --yes android-use@latest setup --agent auto --wait
```

The package is not published to npm yet. The command above becomes the public path after publication; until then, use the skill-only command below or run the package from a trusted source checkout with an explicit manifest. The installer selects the correct native build, verifies every download, installs Android platform tools where supported, stages AU Bridge, connects the detected agent, and checks readiness.

Use `--json` for automation. Run `npx --yes android-use@latest --help` for update, rollback, repair, and uninstall commands.

The installed skill is also available on its own:

```sh
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
```

Read the [plain-language setup guide](https://github.com/austinintelligence/android-use/blob/main/docs/people/getting-started.md) or visit the [project repository](https://github.com/austinintelligence/android-use).

MIT licensed.
