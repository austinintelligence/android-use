# Examples

Send each command as the `command` string to the matching MCP tool, or run it with `au read` / `au act` in a shell.

```text
android_read: status
android_read: screen
android_act: open app "DISPLAY NAME" then wait for "EXPECTED LABEL" up to 5 seconds
android_act: tap "TARGET"
android_act: type "TEXT" in "FIELD" then tap "TARGET" then verify text "EXPECTED TEXT" exists
android_act: scroll down
android_act: page open "https://example.invalid" then page wait for text "EXPECTED TEXT" up to 10 seconds
android_read: page text matching "SEARCH TEXT"
android_act: page click "TARGET"
android_read: notifications
android_act: capture screen
```

The same from a shell:

```bash
au read screen
au act 'open app "DISPLAY NAME" then tap "TARGET"'
```

Use labels, not guessed coordinates. If a label is duplicated, the screen shows the `number N` to add. Each successful action reports the new screen. If a result is partial or unknown, read first and reconcile; never replay blindly.

To use an emulator instead of your phone, add `--target NAME` to any `au` command or to the server: `au --target demo serve --mcp`.
