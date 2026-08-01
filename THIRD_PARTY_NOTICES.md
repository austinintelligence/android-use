# Third-party notices

`android-use` uses third-party components whose licenses are included in their respective package metadata and lockfiles. Release archives must include a generated dependency notice before publication.

Important runtime components include:

- Rust crates from crates.io, including `serde`, `serde_json`, `sha2`, `base64`, `png`, `thiserror`, `windows-sys`, and test-only `tempfile`.
- AndroidX Core and Android build tooling from the Google Maven repository.
- Gradle distribution from services.gradle.org.
- Optional official scrcpy v4.1 for screen/camera preview and recording.

Do not redistribute private signing keys or device-specific artifacts. Keep the release notice synchronized with the exact lockfiles and downloaded tool versions.
