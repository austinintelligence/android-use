# Troubleshooting

Start with:

```console
au doctor
```

It separates required checks from optional capabilities and tells you the next Android-side action. Agents should use `au doctor --json`: it includes a `phase`, a machine-readable `next_step`, and the exact `resume` command.

## No device appears

- Use a data-capable cable and unlock Android.
- Enable Developer options and USB debugging.
- Reconnect the cable and approve **Allow USB debugging?**.
- Close other Android tools that may be restarting ADB.
- If platform tools are installed separately, set `AU_ADB` to the trusted `adb` executable.

## More than one device appears

Disconnect extras or run `au enroll ENDPOINT`. A server stays bound to the enrolled hardware identity.

## Setup opened Accessibility but did not finish

On the screen setup opened, tap **Android-Use**, turn it on, and approve Android's warning. If your device moved the page, look under **Installed apps** or **Downloaded apps**, then run `au setup` again. `au status` reports `Accessibility control is off` until this is done.

## The agent's MCP tools fail

The server starts even without a device, so the tool result explains the problem. Common ones:

- *Android platform tools (adb) were not found*: install Platform-Tools or set `AU_ADB`.
- *The enrolled Android device is not connected*: reconnect and unlock it.
- *Accessibility service is off*: enable it as above.

If the client cannot start the server at all, run `au install --dry-run` to see what is registered, and check that its configuration runs `au` (an absolute path for desktop apps) with the arguments `serve --mcp` ([client setup](../agents/clients.md)).

## "No item labeled … is on the current screen"

The label differs from what Android exposes. The result lists the visible labels; use one exactly, add `number N` for duplicates, or `scroll down` if the item is below the fold. Apps that draw their own UI (games, some cross-platform apps) expose little text; the attached screen image and `tap point X Y` are the fallback.

## Chrome is unavailable

Install or update Google Chrome on Android and open it once. Android Use uses Chrome's local debugging socket; it does not control other browsers.

## The agent reports `locked`, `screen_off`, `helper_killed`, or `stopped`

Before its first action, and after any failure, Android Use checks the phone. Unlock it (`locked`) or press the power button (`screen_off`), then resend the same command: nothing ran. `helper_killed` means the Android-Use helper does not answer (Android ended it, the phone restarted, or it was uninstalled). Android Use does not restart it behind your back; run `au setup`. `stopped` means someone pressed **Stop** on the Android-Use notification; run `au setup` to turn it back on. If Android keeps ending it, exempt Android-Use from battery optimization under Settings → Battery: `au status` and `au doctor` warn when it is not exempt, and when TalkBack is on, because TalkBack changes what the screen reports.

## What the Android-Use app shows

Open the Android-Use app on the phone to see each grant the computer depends on (Accessibility, Notification access, Notification permission, Battery optimization exemption), the helper version, and how long ago the computer last called. The notification's **Stop** action turns the helper off until `au setup` runs again.

## The agent reports `stale`

The screen changed between reading and acting, so nothing ran. Retry the same command; Android Use re-reads the screen itself.

## A command is `partial` or `unknown`

Do not retry the whole command. Read the screen and work out what already happened. `partial` means at least one change ran; `unknown` means the host cannot prove whether the command ran.

## Notifications are empty

`notifications` needs notification access: open the Android-Use app on the device and tap **Open notification access settings**. Notifications posted before access was granted are not listed.

## The helper APK is refused or a newer helper is installed

- *not signed by the Android Use release key*: a release `au` installs only its own helper or an APK signed with the pinned release key. Use the helper that came with `au`.
- *a newer helper is installed*: `au` never downgrades. Run `au uninstall`, then `au setup`, to go back to an older helper.

## Optional capture is unavailable

Run `au read capabilities`. Camera, microphone, and location need Android permission (a microphone that records only silence fails with an error); screen recording needs approval in the Android-Use app for the current helper process (on Android 14 and later, once per recording).

## Emulators

- *No Android SDK found*: install Android Studio, or the Android SDK command-line tools, then run `au emulator doctor`. *The emulator or a system image is not installed*: run `au emulator install` in a terminal (it needs a JDK for Google's `sdkmanager` and asks you to accept the SDK license).
- *The emulator did not come up*: the message ends with the last lines of `emulator.log`. On Linux, check that KVM is available; on any host, free some RAM (the small profile uses 2 GB).
- *another au serve is already driving this target*: stop that server, or use a different `--target`.
- *The device on this emulator's recorded port is not that emulator*: the emulator crashed or was closed outside `au`. Run `au emulator start NAME` again; nothing was stopped or bound.
- *Accessibility is off after a restart*: wait a few seconds for the service to bind, then run `au --target NAME status`.
