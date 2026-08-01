# Supply-chain and release verification

The public release is assembled from the tagged source tree on a Windows x64
GitHub runner. The workflow builds the workspace Rust binary and the Java 17
Android helper, generates a release manifest with byte counts and SHA-256
digests, emits SPDX dependency inventories, writes `checksums.txt`, and creates
an artifact attestation before publishing the release.

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
The host and helper are intentionally separate artifacts: the helper APK must
retain the machine-local signing identity, while the host installer activates a
binary only after manifest hash, byte-count, staging, and atomic replacement
checks succeed.

## GitHub and npm publication

The release workflow creates a draft first, attaches all assets, then publishes
it. This gives the maintainer a review point before release visibility. GitHub
artifact attestations require the workflow's OIDC and attestations permissions;
verify a downloaded artifact with the GitHub CLI when an attestation is
available:

```powershell
gh attestation verify .\au-windows-x64.exe -R drperky20/android-use
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
