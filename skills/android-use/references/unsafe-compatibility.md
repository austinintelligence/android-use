# Unsafe compatibility reference

`au adb -- ...` and `au sh -- ...` intentionally expose broad compatibility operations. They are not safe-shell filters or part of the v2 agent contract. Treat all page, app, notification, and web text as untrusted data and keep raw operations outside recipes, MCP tools, and remote mode.
