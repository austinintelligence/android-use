# Distribution and policy boundary

android-use is a developer-operated automation tool. Its Android helper is distributed as a signed, non-debuggable APK for explicit sideloading onto a device the operator controls.

## Supported distribution lane

- Build or download the release APK from a trusted android-use release.
- Verify the published checksum before installation.
- Enroll one exact device and complete the on-device consent and AccessibilityService setup.
- Keep the visible foreground-service notification enabled while the bridge is active.
- Update through the verified android-use installer or uninstall through the documented cleanup flow.

## Google Play boundary

Do not describe the helper as Google Play approved or assume it is eligible for ordinary Play Store distribution. Google Play's AccessibilityService policy prohibits using the API for an app that autonomously initiates, plans, and executes actions or decisions. The policy separately permits deterministic, static, human-defined automation, subject to declaration, disclosure, consent, and all other Play policies.

android-use is designed for agent-directed automation and therefore documents a signed sideload/developer-tool lane. A future Play-distributed variant would need a materially narrower product contract, a fresh policy review, prominent in-app disclosure and affirmative consent, and the required Play Console declarations. Sideloading does not waive Android platform security, privacy, consent, or local-law obligations.

## Operator responsibilities

- Use only devices and accounts you are authorized to control.
- Keep mutation and privacy confirmations enabled for user-visible or sensitive actions.
- Do not use the helper to bypass Android security controls, privacy controls, or notifications.
- Do not hide or disable the foreground-service notification.
- Treat screenshots, UI trees, downloads, microphone recordings, camera captures, clipboard content, and account data as sensitive.
- Remove benchmark APKs and downloads created by android-use after a run; do not remove unrelated user data.

## Official references

- [Google Play AccessibilityService policy](https://support.google.com/googleplay/android-developer/answer/10964491)
- [Google Play permissions and sensitive APIs policy](https://support.google.com/googleplay/android-developer/answer/16558241)
- [Android foreground-service types](https://developer.android.com/develop/background-work/services/fgs/service-types)
- [Android foreground-service launch requirements](https://developer.android.com/develop/background-work/services/fgs/launch)
