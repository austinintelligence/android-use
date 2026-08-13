# Semantic UI

The v2 helper path also accepts `--contract` and returns bounded `choices` with
an opaque stable `ref`, role, label, visibility, bounds, and redaction state.
The host falls back to the v1 compact rows when an older helper is installed,
so upgrading the host does not invalidate the v1 helper wire format.

Use `ui snap --compact --frontier` for the default agent-facing event-aware tree, then `ui find SELECTOR` and a node handle. Frontier output adds `frontier:true` and keeps only visible labels, controls, and scroll owners; the helper still retains the full cached tree for query operations. It returns `v,g,complete,n`; each node row is `[id,text,description,role,flags,[left,top,right,bottom]]`, with flags `1=clickable,2=enabled,4=checked,8=scrollable`. `complete:false` means the 200-node source cap was reached. `ui snap --compact --delta` returns `{v:1,g:N,same:true}` when no accessibility event invalidated the cache; `--frontier` and `--delta` are separate evidence levels. After a change delta returns `{v:1,base:OLD,g:N,complete:BOOL,d:[[INDEX,NODE]...],r:[INDEX...]}`. Apply `d` and `r` to the previous node array; unchanged rows retain their handles, while changed/removed rows must be refreshed. Numeric handle actions refresh the dirty cache once before resolution, so a changed or removed handle returns `E_STALE` instead of a generic action failure; stable trees keep the no-traversal fast path. `ui snap` returns the expanded fielded representation; `--expanded` permits up to 800 nodes.

`exp f1 SELECTOR POSTSELECTOR [TIMEOUT_MS]` is the bounded proof transaction: unique-find, click by the session handle, semantic wait, and assertion. It is one authenticated helper frame and returns `receipt=find.unique>tap>wait>assert`. Use it only when the target and postcondition are deterministic.

The v2 executor generalizes that fast path: adjacent `find`, `tap`, `long`,
`set`, `scroll`, `global`, `wait`, `assert`, and `observe` steps compile to one
bounded `ui.run` frame. The helper caps a frame at 32 steps and 16 mutations.
A stopped frame returns the exact receipt prefix and committed mutation count
as `E_PARTIAL`; transport loss after possible mutation remains
`E_UNKNOWN_COMMIT`.

Action preference is direct node action, Accessibility gesture/global action, persistent-shell coordinate action, then UIAutomator fallback if the helper is temporarily unavailable. The helper invalidates affected state on accessibility events; it does not perform a full hierarchy dump after every action.

Use the deterministic helper test activity for validation, never a personal app. It contains editable text, tap and long-press targets, a toggle, a dialog, a notification trigger, and predictable scroll items.
