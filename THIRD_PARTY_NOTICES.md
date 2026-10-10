# Third-party notices

Android Use is MIT licensed. Its dependencies keep their own licenses; the Rust ones are pinned in `Cargo.lock` and checked against the allowlist in `deny.toml` on every change (`cargo deny`, `cargo audit`).

- **Host (`au`) runtime crates:** `serde`, `serde_json` (with `indexmap`, to keep key order when editing agent configuration), `sha1`, `sha2`, `base64`, `getrandom`, `png`, and `thiserror`. `sha2` is also used by the build script that embeds the helper APK. `tempfile` is test-only.
- **Android helper:** the Android SDK and the Android Gradle plugin at build time; no third-party libraries at runtime. JVM tests use JUnit and `org.json`.
- **npm launcher:** no runtime dependencies. `oxlint` is a development-only linter.
- **Release builds** attach an SPDX software bill of materials (`android-use.spdx.json`) listing the exact components.

The Android SDK and system images that `au emulator install` downloads are Google's, under the Android SDK License, which you accept in your terminal when `sdkmanager` asks. Android Use does not redistribute them.
