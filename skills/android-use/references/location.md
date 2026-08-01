# Location

`loc set LAT LON` journals the original AU Bridge mock-location app-op and provider state before allowing mock location. The helper uses Android `LocationManager` native test-provider APIs with isolated `au_gps` and `au_network` provider names; it never replaces Android's built-in providers and records only providers it successfully created. Normal locations persist until `loc clear`.

`loc clear` removes only helper-owned test providers, restores the original app-op, and deletes the journal only after both succeed. `loc status` and `doctor` expose leftover journals, ownership, and app-op state. Route playback accepts CSV (`latitude,longitude,delay_ms`) or GPX track points, with `--speed N` and explicit `--loop`.

Location enable/set/route/clear require explicit confirmation. Test routes must always end with `loc clear`, including after a failure.
