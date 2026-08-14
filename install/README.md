# Android Use installer

This small npm package is only a friendly launcher. It selects the bundled platform binary, verifies the bundled binary and helper APK when the release manifest is present, and lets the Rust-owned lifecycle do the real device work.

Normal commands:

```text
npx android-use setup
npx android-use status
npx android-use doctor
npx android-use update
npx android-use uninstall
```

Platform bundles place the signed `au` binary and `aubridge.apk` under `bin/PLATFORM-ARCH/`. `AU_BIN` is available for a trusted local development binary. The launcher never modifies `PATH`, downloads an archive, or creates a second installer state machine.
