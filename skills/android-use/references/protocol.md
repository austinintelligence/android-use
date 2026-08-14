# Protocol

`android.read` accepts `{"q":"status"}`, `{"q":"observe"}`, `{"q":"observe","base":"7"}`, or `{"q":"artifact","id":"a3","range":{"start":0,"end":2800}}`.

It also accepts `{"q":"capabilities"}`, `{"q":"location"}`, and `{"q":"notifications"}`. Visual reads use `{"q":"visual","op":"hash","id":"h..."}` or `{"q":"visual","op":"diff","a":"h...","b":"h..."}` for host PNG artifacts.

It also accepts `{"q":"browser","op":"tabs|observe|text"}`. Browser observations contain a generation, selected tab metadata, and at most 64 compact interactive DOM refs; browser text is capped and never returns HTML or a DevTools WebSocket URL.

Changed observations are `{"o":"8","g":42,"n":[[3,"Save","b",3]]}`. Node tuples are `[ref,label,role,flags]`; roles are `b` button, `i` input, `t` text, `c` checkable, `s` scroll, `m` clickable item, or `u` unknown. Flags are clickable 1, enabled 2, checked 4, and scrollable 8. Unchanged observations are `{"=":1,"o":"8","g":42}`.

`android.act` accepts `id`, generation `g`, and plan `p`, plus optional `deadline_ms` and `max_mutations`. Operations are `tap`, `long`, `text`, `scroll`, `key`, `gesture`, `wait`, `assert`, `launch`, `capture`, `camera`, `microphone`, `screen_record`, `notification_open`, `notification_dismiss`, and `notification_action`. Predicates are `exists`, `missing`, `text`, and `generation_after`. Plans have at most 32 operations, 16 mutations, and a 30-second deadline. Camera, microphone, and screen-record actions require explicit user grants and return private artifact handles.

Set `target:"browser"` for CDP plans. Browser operations are `navigate`, `back`, `forward`, `reload`, `click`, `focus`, `text`, `key`, `scroll`, `wait`, `screenshot`, `select`, `close`, and `new`. Arbitrary page JavaScript evaluation is intentionally unavailable. Browser plans use the same 32-operation, 16-mutation, 30-second limits and host journal; screenshots become private artifact handles.

Set `target:"visual"` for one bounded `crop` operation: `["crop","h...",x,y,w,h]`. It returns a private host artifact handle.

Success is `{"id":"9","ok":1,"g":45,"m":2}`. Failure adds `e` and `at`; `partial` marks committed mutations, and `unknown` adds `next:"observe"`. No per-step success receipts are returned.
