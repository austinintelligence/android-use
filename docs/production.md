# Production guidance

Android Use is designed for one locally enrolled device per server process. For unattended or shared environments:

- Give the Android device and host OS account a single clear owner.
- Keep ADB and the MCP/JSONL process private. Do not expose stdio through an unauthenticated network bridge.
- Treat USB debugging trust as device-control authority.
- Grant camera, microphone, location, notification, and screen-capture permissions only when required.
- Store artifacts on encrypted host storage and define a retention policy.
- Supervise the `au serve` process and start a fresh process when changing devices.
- Require human confirmation for purchases, submissions, account changes, deletion, and privacy-sensitive capture.
- Reconcile `partial` and `unknown` outcomes from observed device state. Do not build automatic mutation retries around them.

There is no built-in fleet scheduler, tenancy layer, or remote broker. Add your own authenticated transport and authorization boundary if you place Android Use behind a service.
