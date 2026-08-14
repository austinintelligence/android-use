# Examples

These examples use the public CLI and typed agent contract. Run `au status` first.

## Read the visible Android interface

```console
au --json observe
```

Look for `g` (generation) and `n` (visible semantic nodes). Agents should act on integer refs from this result rather than guessing coordinates.

## Inspect Chrome

```console
au --json browser tabs
au --json browser observe
au --json browser text
```

## Launch Settings and verify it appeared

PowerShell:

```powershell
$state = au --json observe | ConvertFrom-Json
$plan = '{"id":"open-settings-1","g":' + $state.g + ',"p":[["launch","com.android.settings"],["wait",["text","Settings"],5000]]}'
au --json act $plan
```

## Navigate Chrome and capture the page

First read `au --json browser observe`. Use its browser generation in this plan:

```json
{
  "target": "browser",
  "id": "yellowstone-demo-1",
  "g": 1,
  "p": [
    ["navigate", "https://www.nps.gov/yell/index.htm"],
    ["wait", ["text", "Yellowstone"], 10000],
    ["screenshot"]
  ]
}
```

The receipt returns a private artifact id. Fetch only the byte range you need with `android.read q=artifact` or `au artifact`.

## Connect through JSONL

```console
au serve --jsonl
```

Send one request per line:

```jsonl
{"tool":"android.read","arguments":{"q":"status"}}
{"tool":"android.read","arguments":{"q":"observe"}}
```

Use MCP instead when your agent supports it; the tool schemas and recovery semantics are identical.

## Safety note

The examples are read-only or use public, reversible navigation. Ask before adapting them to deletion, account changes, purchases, submissions, notification actions, location-sensitive work, or privacy-sensitive capture.
