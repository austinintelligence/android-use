# Capabilities

`au read capabilities` reports what the connected device and its current permissions actually support. Trust that output over this page for a particular device.

## See the screen

`screen` lists what is visible, one row per line, with controls marked and their state shown (`sw Wi-Fi=on`, `edit Email="x"`). `screen full` adds descriptive subtitles. Controls with no label get a number (`e7`), and a screen with little text comes with an image that carries the same numbers on boxes. After an action, the result is the new screen or only what changed. [Reading a screen](agent-protocol.md#reading-a-screen) has the format.

## Act on Android

Tap and long press by label or number; type into a field; scroll the main list or a labeled area, or `scroll … until "TEXT"`; system keys, swipes, and point taps on an attached image; open an app by its display name; bounded waits; assertions (`verify text "…" exists`). Up to 16 steps join with `then`, and a read can end the chain.

Each plan names the UI it was built from; if the screen changed first, nothing runs (`stale`). A label that is only a close match is used when it is safe and reported; risky words (delete, pay, send) never match loosely.

## Use Chrome

List and select tabs; read a page's title, text, and interactive elements; open a link; click, focus, type, send keys, scroll; wait for text; reload; go back or forward; capture a page screenshot. Chrome must expose its Android debugging socket (open it once). Agents cannot run arbitrary page JavaScript.

## Device state

- Notifications: list by title and text, newest first, and open, dismiss, or invoke one by its title, after you enable notification access.
- Location: a one-shot read, when the helper has permission.
- Camera: rear or front still, after Android permission.
- Microphone: bounded mono WAV, after Android permission.
- Screen recording: a bounded MP4 of up to 30 seconds, after you grant Android's screen-capture prompt for that process.
- Images: crop PNG captures and compare two screens through bounded hashes and sampled differences.

## Targets

One session drives one device. Besides your phone, `au emulator` creates local emulators that each act as a separate target, with snapshots for quick resets ([emulators](../guides/emulators.md)).

## Not included

No language model, cloud device farm, remote broker, root access, silent permission bypass, arbitrary shell tool, or page-JavaScript runtime. Behavior and accessible labels vary by Android version and maker.
