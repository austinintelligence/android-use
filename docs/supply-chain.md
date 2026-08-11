# Supply chain and release verification

The release process is designed to make the installed host and Android helper traceable. A Windows x64 GitHub runner builds the Rust host and Java 17 helper, generates a manifest with byte counts and SHA-256 digests, emits dependency inventories, writes `checksums.txt`, and creates an artifact attestation before publication.

## Local verification

Run these checks from the repository root:

```powershell
cargo audit --file Cargo.lock
cargo deny check
npm audit --audit-level=high
npm run docs:check
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-public.ps1
```

Build release assets in a private temporary directory, not inside the repository. Verify every row in `checksums.txt` before distributing an asset. The host and helper remain separate artifacts: the helper keeps its machine-local signing identity, while the host installer activates a binary only after manifest, hash, byte-count, staging, and atomic-replacement checks succeed.

## GitHub and npm publication

The release workflow creates a draft release first, attaches the assets, and publishes only after maintainer review. When an attestation is available, verify a downloaded host asset with:

```powershell
gh attestation verify .\au-windows-x64.exe -R austinintelligence/android-use
```

The npm package is published separately from `packages/installer` with provenance. Until that owner-managed publication step succeeds, GitHub skill installation remains the canonical public installation path.

## Public-tree gate

Run the public scan on the exact checkout that will be pushed. It rejects long device identities, absolute user paths, legacy canvas listener markers, and credential-shaped literals. Keep live-device traces, screenshots, recordings, serials, tokens, location journals, signing keys, and generated release output outside the public tree.

