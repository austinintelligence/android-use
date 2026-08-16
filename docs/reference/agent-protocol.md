# Agent protocol

MCP advertises exactly two tools. Each new call has one required string argument, `command`:

- `android.read` is non-mutating. Commands include `status`, `screen`, focused `screen matching "TEXT"` or `find "TEXT"`, `browser tabs`, `page`, `page text`, `page text matching "TEXT"`, `capabilities`, `location`, `notifications`, and image hash or difference.
- `android.act` performs bounded actions such as `tap "TARGET"`, `toggle "TARGET"`, `type "TEXT" in "FIELD"`, `open app "DISPLAY NAME"`, `page click "TARGET"`, `wait for text "EXPECTED TEXT" up to 5 seconds`, and `verify text "EXPECTED TEXT" exists`.

Use straight double quotes for variable text and `then` between short actions. The host owns state, target resolution, operation identity, safety limits, journals, artifacts, and image content. Do not construct generations, refs, package names, tab IDs, or JSON plans for normal calls.

`stale` is safe to retry after a fresh read. `partial` means a mutation already ran. `unknown` means dispatch may have happened. Read and reconcile both before another mutation. A semantic miss may include a current screenshot; coordinates are a bounded fallback only while that screen remains current.

The old structured JSON forms remain accepted during deprecation for CLI, JSONL, MCP, and protocol-golden callers. They are compatibility-only and are not advertised by the new schemas. Full grammar and limits: [the installed protocol reference](../../skills/android-use/references/protocol.md).
