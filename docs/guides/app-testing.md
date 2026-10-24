# Testing apps with `au test`

The command grammar an agent uses is also a readable test language. A test file is one command per line; nothing needs a model to run it.

```text
# login.au
reset app "dev.codex.aubench"
open app "AU Benchmark Fixture"
type "order 42" in "Fixture text" then tap "SUBMIT FORM"
expect screen "Submitted: order 42"
tap "Benchmark toggle"
expect screen "=on" within 8 seconds
expect no screen "Loading"
```

Run files with `au test tests/*.au --junit out.xml`. Each file is one test case; a failing line stops that file, reports its number and the screen as it was, and the other files still run. The exit code is 1 when anything failed.

| Line | Meaning |
| --- | --- |
| any `android_act` command | Runs it; an error or a miss fails the line. `then` chains work. |
| `reset app "PKG"` / `clear app "PKG"` | `am force-stop` / `pm clear` through adb, for a known start. |
| `expect screen "TEXT" [within N seconds]` | Waits up to N seconds (default 5, at most 60) for the text on screen. |
| `expect no screen "TEXT" [within N seconds]` | The same, for text that must be gone. |
| `expect result "TEXT"` | The previous command's reply contains the text. |

`#` starts a comment. The same session layer runs the lines, so `policy.toml` applies to tests too.

## From an agent session to a test

Let an agent explore once, then `au trace to-test ID > tests/login.au`. The file holds the commands that ran to the end, without the misses; add `expect screen` lines after the steps that matter. A trace that typed redacted text (`***`) is refused: edit that step by hand so the test does not depend on a secret.

## The lab screens

`cargo xtask android` also builds the fixture app's lab (`OPEN LAB` on its main screen, or `am start -n dev.codex.aubench/.LabActivity --es screen late|popup|ask|catalog|risky|injected|form|web|persist`): a late-loading list, a popup menu and bottom sheet, a location-style dialog, a 200-row catalog, risky buttons that only count their taps, text that looks like instructions, a form with different keyboards, a web view, and a toggle that survives a force-stop. They give tests a deterministic app to aim at.
