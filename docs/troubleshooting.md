# Troubleshooting

Start with:

```console
au doctor
```

It separates required checks from optional capabilities and tells you the next Android-side action.

## No device appears

- Use a data-capable cable and unlock Android.
- Enable Developer options and USB debugging.
- Reconnect the cable and approve **Allow USB debugging?**.
- Close other Android tools that may be restarting ADB.
- If platform tools are installed separately, set `AU_ADB` to the trusted `adb` executable.

## More than one device appears

Disconnect extras or explicitly run `au enroll ENDPOINT`. A server session remains fixed to the enrolled hardware identity.

## Helper installed, but not ready

Open **Settings → Accessibility → Android Use → On**. Android may move this setting under **Installed apps** or **Downloaded apps**.

## Chrome is unavailable

Install or update Google Chrome on Android and open it once. Android Use uses Chrome's local debugging socket; it does not control arbitrary browsers through CDP.

## The agent reports `stale`

The interface changed after it was observed. This is expected safety behavior. Observe once, use the new generation and refs, then rebuild the plan.

## A plan is `partial` or `unknown`

Do not retry. Observe the device and determine what already happened. `partial` means at least one mutation ran; `unknown` means the host cannot prove the final outcome.

## Optional capture is unavailable

Run `au capabilities`. Android permissions, hardware, and process-local screen-record approval can differ even when normal UI control is ready.
