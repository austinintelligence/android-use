# Selector reference

Use selectors only to query semantic UI. Prefer `android.observe` with `mode=query` or `mode=choices`, then execute against the returned stable reference and generation. A stale reference is `E_STALE`; refresh once and re-query. Do not infer coordinates from labels or replay a selector after an unknown mutation result.

The compatibility selector grammar remains documented in [`selector-grammar.md`](selector-grammar.md).
