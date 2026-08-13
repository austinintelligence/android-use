# Output and errors

Normal success is one line:

```text
ok
ok N
ok PATH
```

Normal errors are one line: `err CODE message`. `-j` emits a single minified JSON object. `-q` suppresses success proof. Large results are capped or written to a non-clobbering artifact with its path, byte count, and SHA-256.

`-c`/`--compact` is the tokenizer-dense machine mode: success is `{"o":1,"d":...}`, `{"o":1,"n":N,"d":...}`, `{"o":1,"p":"PATH"}`, or `{"o":1,"t":"TEXT"}`; errors are `{"o":0,"e":"CODE","m":"MESSAGE"}` and may include `"d":{...}` recovery details. `-c -q` emits nothing on success. The compact envelope is deliberately separate from `-j` so existing integrations keep their schema.

`-w`/`--wire` is the preferred versioned agent envelope: success is `{"v":1,"o":1,"d":...}` (or `n`, `p`, `t`), and errors are `{"v":1,"o":0,"e":"CODE","m":"MESSAGE"}` with optional `"d":{...}` recovery details. If `-w` and `-c` are both supplied, `-w` wins. Use `-q` when only the side effect matters.

`pipe` is a foreground line protocol. With `--jsonl`, send one request per
line as `{"c":"COMMAND","a":["ARG",...]}` or `{"b":"DSL"}`; each
non-empty line produces one response before the next line is read. A line
failure is bounded and recoverable, so later lines still use the warm shell,
helper, and CDP pools.

For diagnostic timing only, add `--trace PATH`. It appends bounded JSONL
spans across the CLI, daemon, device selection, bounded child execution,
helper/CDP operation, action result, and output boundary. The trace ID is
propagated across the named pipe; raw arguments, tokens, page text, and media
bytes are excluded.

Machine-readable success/error lines are bounded by the host output limit. If a structured response would exceed it, AU returns `E_OUTPUT_LIMIT` with the measured byte count; request a file-backed result with `--out PATH` or a narrower query. Default human-mode success remains the one-token proof even when the underlying result contains structured data.

Screenshots, recordings, camera files, microphone files, DOM dumps, and raw binary never enter a normal terminal/transcript. `--binary` is required for `cam pipe` or `mic pipe`; without `--out`, it emits only the requested bytes and no textual prefix. With `--out PATH`, even `--binary` returns compact artifact proof and never duplicates bytes into the agent transcript.
