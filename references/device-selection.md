# Device selection and failover

AU starts unenrolled. `au u ENDPOINT` records the endpoint's exact `ro.serialno`; AU then groups USB, Wi-Fi, and mDNS endpoints by that identity, never by model/product/alias alone. A public fixture identity must be synthetic.

Automatic order:

1. online USB endpoint with the exact serial;
2. configured known Wi-Fi endpoint with the exact serial;
3. matching mDNS endpoint with the exact serial.

`-s ENDPOINT` still verifies the same hardware identity. For convenience, `-s usb`,
`-s wifi`/`-s wireless`, and `-s mdns` select a transport only when its endpoint
reports the exact pinned identity. `u ENDPOINT` records a known Wi-Fi endpoint
only after identity verification. `d -j` provides endpoint and identity grouping
proof.
