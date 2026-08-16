# Setup

Connect one unlocked Android 8+ device with USB debugging authorized. Keep `au` beside `aubridge.apk`; run `au setup`, approve Settings → Accessibility → Android Use, then run `au doctor` and `au serve --mcp`.

No device: use a data cable, enable Developer options and USB debugging, accept the trust prompt, rerun doctor. Missing ADB: install Platform-Tools or set `AU_ADB`. Accessibility off: enable it and rerun doctor. Broken helper: `au repair PATH`. Optional grants are requested only when used.
