# Getting started

This setup normally takes one cable, one command, and a few taps on the Android device.

## Before you begin

You need:

- a Windows, macOS, or Linux computer with Node.js installed;
- an Android phone, tablet, emulator, or Android-based device;
- a USB cable that carries data, not only power;
- permission to control the device.

## 1. Prepare Android

1. Open **Settings** on the Android device.
2. Open **About phone** or **About tablet**.
3. Tap **Build number** seven times. Android may ask for the device PIN.
4. Go back to Settings and open **Developer options**.
5. Turn on **USB debugging**.
6. Connect the device to the computer and keep it unlocked.

Names vary slightly between device makers. Search Settings for “Build number,” “Developer options,” or “USB debugging” if needed.

## 2. Install the agent skill

Open PowerShell on Windows or Terminal on macOS/Linux. Copy and paste:

```sh
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
```

Then ask the agent to use Android Use and guide the setup. The skill is available now. The all-in-one `npx android-use@latest` installer is prepared but is not yet published to npm, so the agent must verify and explain the available host release instead of claiming that command works.

The current GitHub prerelease contains a Windows x64 host and the Android helper. Other host packages remain release work. If your computer is unsupported, setup should stop with that clear limitation.

## 3. Approve Android prompts

On the Android device:

1. Tap **Allow** on the “Allow USB debugging?” prompt. You may select **Always allow from this computer** on a computer you trust.
2. Open **AU Bridge** if it does not open automatically.
3. Tap the Accessibility setup action and enable **AU Bridge**.
4. Return to AU Bridge.

Only enable camera, microphone, notification, or location access when you want an agent to use that feature. They are not required for ordinary screen control.

## 4. Check that it worked

Run:

```sh
au ready
```

If it says ready, setup is complete. If it is waiting, run:

```sh
au doctor --repair
```

Then follow the plain-language instruction it prints. The host setup journal remembers completed steps, so rerunning host setup continues instead of starting over.

## Common problems

### The device does not appear

- Unlock it and keep the screen on.
- Try another USB port.
- Try a different data-capable cable.
- Change the USB mode from **Charge only** to **File transfer** if Android offers that choice.

### It says unauthorized

Look at the Android screen and approve the USB debugging prompt. If no prompt appears, turn USB debugging off and on, reconnect the cable, and run setup again.

### More than one device is connected

Run `au d` to list them, then select the exact endpoint with `au u DEVICE`. Android Use remembers the enrolled hardware identity and refuses a mismatched device.

### The screen cannot be understood

Open AU Bridge and confirm its Accessibility service is enabled. Some games and custom-drawn apps do not expose semantic controls; agents can use a bounded visual fallback for those screens.

### Setup is still stuck

Run `au doctor --json` and give the result to your agent or include it in a bug report. Do not post device serials, private screen text, tokens, or recordings publicly.
