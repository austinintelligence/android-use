# Architecture

![Android Use architecture](../../images/how-it-works.svg)

The computer-side Rust binary owns device selection, operation journaling, MCP/JSONL adapters, Chrome CDP, and private artifacts. It talks to Android Debug Bridge through explicit argument arrays.

The Android helper exposes semantic accessibility, selected device capabilities, and bounded plan execution through an authenticated abstract-local socket. It has no Android internet permission. The host reaches it through an ADB forward tied to the enrolled device.

Every mutating plan includes the UI generation it observed. The helper parses the whole plan, checks limits, verifies the generation immediately before the first mutation, runs forward, and stops on the first failure. The host persists a digest before sending so interrupted operations become `unknown` instead of being silently repeated.

Chrome page control uses Chrome's loopback-only Android debugging socket. The browser adapter returns a compact page frontier rather than raw HTML.
