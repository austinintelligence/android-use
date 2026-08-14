# Third-party notices

Rust dependencies and licenses are pinned in `Cargo.lock` and governed by `deny.toml`. Runtime crates are `serde`, `serde_json`, `sha2`, `base64`, `png`, and `thiserror`; `tempfile` is test-only. Android uses the Android SDK and Gradle Android plugin; JVM tests use JUnit and `org.json`. Node is used only for the package bootstrap and its tests.
