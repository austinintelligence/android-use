# Common workflows

Look once: `screen` (or `page` for Chrome). After that, each action's result shows the new screen.

Act and verify in one call: `type "TEXT" in "FIELD" then tap "TARGET" then verify text "EXPECTED RESULT" exists`.

Long lists: `scroll down` moves the main list; `scroll down in "AREA"` targets a labeled one. `find "TEXT"` checks whether something is visible without listing everything.

Chrome: `page open "https://example.com" then page wait for text "Example Domain" up to 10 seconds`, then `page click "More information"`.

Notifications: `notifications` lists them by title; `open notification "TITLE"` acts on one.

Recovery: after `stale`, retry; after `partial` or `unknown`, read and reconcile before changing anything. Use `capture screen` or `screen full` only when the summary is not enough.
