# Selector grammar

Selectors match an accessibility snapshot. The grammar is:

```text
selector   := term ("," term)* ("#" occurrence)?
term       := field ("=" | "~") value
field      := text | desc | id | class | pkg | clickable | enabled | scrollable | checked | bounds
occurrence := non-negative integer
```

`=` means exact match; `~` means substring match and is valid only for string fields. Boolean fields require `=true` or `=false`. Bounds are exact `LEFT,TOP,RIGHT,BOTTOM` pixels. Escape a literal comma, hash, equals, tilde, or backslash with `\`.

Examples:

```text
text~Allow,clickable=true#0
id=com.example:id/submit,enabled=true
desc~AU tap target
text~a\,b#2
```

Node IDs are session-scoped. A changed accessibility generation makes an old ID fail exactly as `err E_STALE stale node handle`; refresh with `ui snap` or `ui find` rather than retrying it.
