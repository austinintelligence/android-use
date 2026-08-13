# Remote mode boundary

Remote mode is intentionally separate from the local AU Bridge. `dev.codex.aubridge` remains the high-authority, networkless local helper. A future remote companion owns only outbound relay connectivity, pairing state, and encrypted semantic envelopes; it does not own accessibility, raw ADB, raw shell, unrestricted files, camera, microphone, or location authority.

Remote operations carry a device identity, operation ID, plan hash, expected generation, deadline, nonce, and mutation count. A relay may forward opaque ciphertext and bounded metadata, but it must not be able to issue raw device commands. Unknown mutation outcomes are not replayed.

The current repository contains the stable contract, transport vocabulary, strict
remote operation validation, and local adapter foundation. `au remote status`
and `au remote protocol` expose the boundary; `au remote pair` intentionally
returns `E_REMOTE_NOT_READY` until a real broker and Android Keystore-backed
pairing implementation are present. Remote cryptographic transport remains
behind this authority boundary until its audited dependency, pairing,
downgrade, replay, revoke, and retention tests are complete.
