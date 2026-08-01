# Vision escalation ladder

Use the cheapest sufficient evidence, in order:

1. `vision inspect` asks for the semantic frontier and returns `complete`, generation, node count, and the next recommended level. It does not capture pixels.
2. `vision hash` captures only when needed and returns a decoded RGBA scene fingerprint plus dimensions. Passing a local PNG hashes it without touching the device.
3. `vision diff BASE_PNG [THRESHOLD]` captures the current screen, compares decoded pixels, and returns changed-pixel count, ratio, and one bounding rectangle. The default threshold is 8/255.
4. `vision crop X Y W H` captures and writes only the requested crop. Coordinates accept pixels or percentages; `--out` is non-clobbering unless `--force` is explicit.

`vision region X Y W H` records a hash-bound region handle without emitting image bytes. `vision check REGION` re-hashes the current scene and returns `E_STALE` if any pixel-level scene fingerprint changed; never reuse a stale visual handle. `vision clear` removes AU-owned region-handle state and is idempotent.

All screenshot operations are explicit protocol boundaries. Normal output is metadata, not binary image data. Use `--binary` only for a separately requested binary stream. Secure or unavailable capture must surface a typed capability error; do not retry vision blindly.
