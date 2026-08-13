# Supply-chain and release verification

The public release is assembled from the tagged source tree on native GitHub
hosted Windows, macOS, and Linux x64/ARM64 runners. The workflow builds six
host artifacts plus the Java 17 Android helper. It then builds portable
archives, Debian/RPM packages, x64/ARM64 MSIs, and Homebrew/Winget metadata.
Every required artifact is listed by byte count and SHA-256 in an exact-byte
Ed25519-signed release manifest. The workflow also emits SPDX dependency
inventories, writes `checksums.txt`, and creates an artifact attestation before
publishing the release.

## Local verification

Run these checks from the repository root:

```powershell
cargo audit --file Cargo.lock
cargo deny check
npm audit --audit-level=high
npm run docs:check
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-public.ps1
```

Build the release asset set into a private temporary directory, not into the
repository, then verify each row in `checksums.txt` before distributing it.
The host and helper are intentionally separate raw artifacts. Public helper
releases must match `android/aubridge/release-signer.sha256`; changing that pin
is an explicit key-rotation event. The installer pins the release-manifest
public key, verifies the detached signature before trusting asset metadata,
then verifies each streamed asset's hash and byte count. Activation is journaled
and whole-install rollback is exercised with process-death fault injection.

## GitHub and npm publication

The release workflow creates a draft first, attaches all assets, then publishes
it. This gives the maintainer a review point before release visibility. GitHub
artifact attestations require the workflow's OIDC and attestations permissions;
verify a downloaded artifact with the GitHub CLI when an attestation is
available:

```powershell
gh attestation verify .\au-windows-x64.exe -R austinintelligence/android-use
```

The npm package is published separately from `packages/installer` with
provenance. The maintainer must configure npm trusted publishing for the exact
GitHub repository/workflow or provide the explicitly managed `NPM_TOKEN`.
Until that owner action succeeds, GitHub skill installation remains the public
installation path and npm publication must not be represented as complete.

## Public-tree gate

Run the public scan on the exact checkout that will be pushed. It rejects long
device identities, absolute user paths, legacy canvas listener markers, and
credential-shaped literals. Keep live-device traces, screenshots, recordings,
serials, tokens, location journals, signing keys, and generated release output
outside the public tree.
