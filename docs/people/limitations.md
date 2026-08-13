# Known limitations

- Android OEMs differ in Accessibility binding persistence, foreground-service rules, mock-location app-ops, camera availability, and audio routing.
- mDNS discovery can be slower than USB; identity-safe failover is prioritized over pretending that network latency is local.
- Chrome CDP is unavailable when Chrome is not launched with a debuggable endpoint or when the endpoint is blocked; helper coordinate control is the fallback.
- A locally signed helper is not a Play Store release. Users must review and grant its requested capabilities themselves.
- The optional scrcpy integration is pinned to official v4.1 and does not create Windows virtual camera or microphone devices.
- The compact protocol optimizes agents over human readability. Use `-j` or the references when debugging manually.
