# Common workflows

## Observe, act, verify

1. `android.read {"q":"observe"}` returns generation `g` and refs.
2. `android.act` sends a unique `id`, that `g`, and a short operation array.
3. End the plan with a bounded wait or assertion when the result matters.
4. Observe again only when another decision is needed.

## Open an app

Use a plan operation such as `["launch","com.android.settings"]`, then wait for a known label. Package names are Android identifiers, not display names.

## Enter text

Observe the current UI, use the input's integer ref, and send `["text",ref,"value"]`. Do not tap guessed coordinates when a semantic ref exists.

## Use a web page

Read browser tabs, select the intended tab, observe page references, then use a browser-targeted plan. Page screenshots are useful for visual confirmation; page text and refs are better for interaction.

## Recover safely

| Result | Next move |
| --- | --- |
| `stale` | Observe again and rebuild with the new generation. |
| `partial` | Observe. Do not repeat the plan. Some mutation already occurred. |
| `unknown` | Observe and reconcile. Never replay the same operation blindly. |
| timeout before mutation | Inspect once, then retry only if current state proves it is safe. |
