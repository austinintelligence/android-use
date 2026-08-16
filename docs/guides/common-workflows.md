# Common workflows

Read state: `screen` or `page`.

Act and verify in one bounded call: `type "TEXT" in "FIELD" then tap "TARGET" then verify text "EXPECTED RESULT" exists`.

Chrome: `page open "https://example.com" then page wait for text "Example Domain" up to 10 seconds`, then `page click "More information"`.

Recovery: retry a stale pre-send action after a read; read and reconcile partial or unknown results before mutating again. Use `capture screen` or `screen full` only when semantic state is insufficient.
