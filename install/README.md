# Android Use installer

This package gives people one setup command. It selects and verifies the files for their computer, then opens the Android screens that still need the owner's approval.

Normal commands:

```text
npx android-use setup
npx android-use status
npx android-use doctor
npx android-use update
npx android-use uninstall
```

`npx android-use setup` is reserved for the published package and must not be documented as available until npm publication succeeds. Developers can set `AU_BIN` to a trusted local build.
