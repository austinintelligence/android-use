# Recipes

Recipes are local, declarative semantic plans. They are stored under the AU state root in `recipes/*.json` and are bounded by the same operation, deadline, mutation, selector, and output limits as direct execution.

Example:

```json
{
  "schema": 1,
  "name": "dismiss-dialog",
  "description": "Dismiss the visible dialog when its Allow button is present.",
  "steps": [
    {"op":"tap","target":{"selector":"text=Allow,clickable=true"}},
    {"op":"wait","target":{"selector":"text=Done"},"timeout_ms":3000}
  ]
}
```

Recipes cannot contain raw shell, raw ADB, arbitrary code, unrestricted filesystem paths, or hidden network instructions. Community distribution and signing are intentionally not enabled until a trust and revocation policy is implemented.
