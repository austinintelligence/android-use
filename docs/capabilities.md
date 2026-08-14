# Capabilities

`au capabilities` reports what the connected device and current permission state actually support. Treat that output—not this page—as authoritative for a particular device.

## See the interface

Android Use returns a bounded semantic frontier with visible text, roles, state flags, and interaction references. Ask for more detail only when the frontier is insufficient. Screenshots are private artifacts.

## Act on Android

Plans support tap, long press, text entry, scroll, system keys, gestures, app launch, bounded waits, and assertions. Each plan names the UI generation it was built from. Stale plans fail before mutation.

## Use Chrome

Chrome control can list and select tabs; observe page titles, text, and interactive elements; navigate; click; focus; type; send keys; scroll; wait; reload; go back or forward; run bounded evaluation; and capture a page screenshot. Chrome must expose its Android debugging socket. Browser evaluation rejects network-capable JavaScript.

## Inspect supported device state

- Read location when the helper has location permission.
- Read notifications after the user enables Android notification access.
- Open, dismiss, or invoke one safely identifiable primary notification action.
- Capture rear or front camera images after Android permission.
- Capture bounded mono WAV audio after Android permission.
- Capture a bounded MP4 screen recording after the user grants Android MediaProjection for that process.
- Crop PNG artifacts and compare visual structure through bounded hashes and sampled diffs.

## Not included

Android Use does not include an LLM, cloud device farm, remote broker, root access, silent permission bypass, arbitrary shell tool, or unrestricted JavaScript runtime. App behavior and accessible labels vary by Android version and vendor.
