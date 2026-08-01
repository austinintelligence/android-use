# Batch DSL

Separate actions with newlines or semicolons. Single and double quotes preserve whitespace; backslash escapes the next character. A `#` at the beginning of an otherwise empty statement starts a comment.

```text
# one persistent shell transaction
t 50% 50%; tx 'hello world'; k ENTER
retry 2 sw 50% 90% 50% 20% 350
repeat 3 k TAB
wait 250
wait ui:text~Ready 5000
assert ui:desc~AU\ tap\ target
ui tap 'text~Allow,clickable=true#0'
if ui:text~Ready then ui tap 'desc=AU\ tap\ target,clickable=true'
```

`retry N ACTION` means retry only after a failed attempt; N is bounded to 1..2, so one action runs at most three times and stops immediately after success. Shell retries are lowered into `attempt || (delay; attempt)` inside one framed transaction. Semantic retries are replayed only for read-only/query/synchronization actions, because a lost mutation response may mean the mutation already happened. `repeat N ACTION` intentionally executes the action N times; N is bounded to 1..20. A batch is capped at 64 instructions and 20 worst-case state-changing attempts (`repeat * (retry + 1)`); it fails before execution when either bound is exceeded. Shell-compatible runs are lowered into one framed persistent remote-shell transaction. Protocol boundaries occur at semantic UI, screenshots, web/CDP, camera, microphone, location, and other binary operations. A 20-action shell-only batch returns exactly `ok 20` on success.

Logical shell actions are joined with `&&`; the first failed final retry stops the batch and returns its error instead of allowing a later action to mask it. `||` is reserved for the retry chain inside one action. Typed app/system/file/notification/property/settings/process/forward actions, raw `adb`/`sh`, web/CDP, media, vision, and location commands create protocol boundaries; `app start` and `app stop` remain shell-compatible and can share the persistent transaction.

`wait ui:SELECTOR [MS]` and `assert ui:SELECTOR [MS]` lower to semantic waits/assertions; they never interpolate selector text into a shell command.

`if ui:SELECTOR then ACTION [ARGS]` is a bounded semantic branch. The selector is queried once; a missing node skips the action and counts `ok 0`, while a match executes exactly one action. It never evaluates arbitrary page text or shell syntax as a condition.

Shell-compatible batch actions use a 250 ms gap between actions by default. The first action starts immediately; retries and actions after a semantic/protocol boundary are paced inside the shell lowering. Override with the global `--delay MS` (or `--batch-delay MS`) option, where `0` disables pacing and `1..999` selects a sub-second gap:

```text
au --delay 200 b "t 50% 50%; tx 'ready'; k ENTER"
au --delay 0 b "home; back"
```

Pacing is lowered into the same persistent shell transaction for shell-compatible actions. Semantic, web, media, location, and binary boundaries are event-driven by default. If a device needs an inter-action settle window, an explicit `--delay 200` or `--delay 300` also paces those protocol boundaries; the default 250 ms is not imposed on them.
