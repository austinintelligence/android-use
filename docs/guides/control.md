# Control what an agent may do

Android Use keeps a person in charge. Everything here is optional, off until you use it, and enforced by the program on your computer or by the helper on the phone, never by the agent's own good behavior.

## policy.toml

Put `policy.toml` in the state folder (`AU_HOME`). The file is a small TOML subset: `key = "text"`, `true`, a number, or `["a", "b"]`, with `#` comments. A file that does not parse stops the session with the line number; it never means "no policy".

```toml
apps.allow = ["com.android.settings", "Clock"]   # only these may be opened (name or package)
apps.deny = ["com.example.bank"]                 # these never may
risky.extra = ["unsubscribe", "cancel"]          # more words that make a guessed label a miss
actions.max_per_session = 200                    # steps; a command that would pass it is refused whole
capture.allow = false                            # no camera, microphone, screen capture or recording
codes.apps = ["com.example.bank"]                # apps `read code from notification` may read
files.dir = "/home/me/au-files"                  # the one folder push file and pull file may use
```

Values cannot contain backslashes, so on Windows write paths with forward slashes: `files.dir = "C:/Users/me/au-files"`.

An app is matched on its package and its launcher name (and on the name the agent gave when that name is a package or "Settings"), so denying `Settings` also stops `open app "com.android.settings"` and `open setting "wifi"`. The rule covers the app on screen, not only `open app`: while an app the policy keeps out is in front, only `press back` and `press home` run, and a command that leads into one (a link, a notification, a home-screen tap) is answered with Home. Packages with no launcher entry (the home screen, system UI) are not restricted. A blocked command returns `blocked_by_policy`, names the rule, and runs nothing. `android_read policy` shows what is in force. Policy, secrets, routines, traces and the `au watch` control file live in `AU_HOME` itself, also for emulator targets selected with `--target`.

`codes.apps` limits the convenience command only. Notification text is still readable through `notifications` once you grant notification access; keep that access off for phones where it matters.

## Approve on the phone

`ask phone "Pay $25 to Alex?" then tap "Pay $25"` shows a full-screen **Allow once / Deny** prompt on the phone with the question and the exact steps that will run (they must fit in 300 characters, so you always see all of them). While it waits, the helper runs no actions at all, so no agent or process can press its buttons; it is not shown over the lock screen. `denied` comes back if you deny, if nobody answers in two minutes, or if the screen's labels changed while you decided, and a Stop or pause pressed meanwhile still applies. This needs a helper from this release (`au update`).

## Secrets the agent never sees

```text
printf %s "$BANK_PIN" | au secret add "bank pin" --apps com.example.bank
```

The value goes to the system store (macOS Keychain, Secret Service through `secret-tool` on Linux, a per-user DPAPI file on Windows). Only the name and the allowed apps are kept in `AU_HOME`. The agent then sends `type secret "bank pin" in "PIN"` as the first step of a command. It is typed only into a single field the helper itself flags as a password field, in an allowed app, and only when no plain field on screen imitates one; the value is removed from every reply, error, trace and routine, and a line holding any piece of it is hidden whole. `android_read secrets` lists names, never values. On macOS the add command briefly shows the value in the `security` process arguments; use a terminal you trust.

## Pause and stop

`au watch` prints a link to a page on this computer (loopback only, with a token) that shows each command and its result as they happen. **Pause** makes the next change come back as `paused`; **Stop** ends the session's access until you restart it. A new `au serve` starts running again.

## Traces

Every session writes a trace you can review: `au trace list`, `au trace show ID`, `au trace export ID` (one page, no scripts), and `au trace to-test ID` ([testing apps](app-testing.md)). Text typed into password, PIN, code, token and similar fields, or into a field with no name to judge by (`e12`), clipboard text, and routine values are replaced by `***` before anything is written, whatever the spelling (`type`/`TYPE`, `in`/`into`, curly quotes). Traces are private to you and deleted after 7 days.

## Waiting and files

- `wait for notification [from "APP"] up to N seconds` returns the next new matching notification; `read code from notification "APP"` returns its 4 to 8 digit code if `codes.apps` lists the app.
- `push file "NAME"` and `pull file "NAME"` copy between `files.dir` and the phone's Downloads. Only plain file names and regular files (not folders), nothing over 64 MB, links out of the folder are refused, a pull never replaces a file, and each transfer counts against `actions.max_per_session`. Agents should ask first.
- `read audit` lists unnamed and duplicate controls on the screen; `eN` already names every unlabeled control.

## Routines

After a command works, in the same session, `save routine "set timeout" from last with "30 seconds"` stores it with that value as a parameter; `run routine "set timeout" with "1 minute"` runs it again in one call. A routine is text: it is parsed, checked and journaled like any command, and a changed screen is a miss. `android_read routines` lists them.
