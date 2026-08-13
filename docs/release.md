# Release process

1. Run the Rust, helper, installer, APK, security, and skill validators.
2. Build native Windows x64/ARM64, macOS Intel/Apple Silicon, and Linux x64/ARM64 hosts plus the signed helper with pinned toolchains and pinned helper-certificate continuity.
3. Build deterministic portable ZIP/tar archives, Debian/RPM packages, Windows MSI packages, Homebrew metadata, and Winget metadata. Inspect package contents and smoke install/uninstall the x64 MSI.
4. Generate an Ed25519-signed release manifest with SHA-256/byte records for every required raw host, helper, portable archive, OS package, and package-manager manifest.
5. Run USB and matching Wi-Fi live lanes separately; classify every feature as passed, failed, or unsupported with direct evidence.
6. Run cold, daemon, persistent-shell, semantic, and batch benchmarks with warmup/sample counts and p50/p95/p99.
7. Scan the publish candidate for serials, tokens, private paths, recordings, screenshots, signing material, and stale canvas/HTTP surfaces.
8. Create a clean public commit/history and push only the audited public tree.
9. Publish immutable GitHub release assets and the signed manifest. Publish the npm installer from `packages/installer` through an owner-approved npm account/trusted publisher.
10. Install from release MSI/archive/package, the public GitHub skill, and NPX in clean Windows, macOS, and Linux roots; verify update, rollback, uninstall, and reinstall.
11. Record the final cleanup proof and keep any failed or unsupported gate visible.

The detailed artifact, checksum, SBOM, attestation, and npm provenance
procedure is in [`supply-chain.md`](supply-chain.md).

The workflow is tag-bound: the strict semver tag, event SHA, checkout SHA, and
installer version must identify the same commit. It requires owner-managed
helper-signing, release-manifest-signing, and npm credentials and fails closed
when any are absent. It uses Node 24, GitHub OIDC for attestations/provenance,
and an owner-managed `NPM_TOKEN`; no release is published from an arbitrary
branch dispatch.

Do not silently reclassify a failed wireless performance gate as stable. Release status is `STABLE`, `PRERELEASE`, `BLOCKED`, or `FAILED` and must be stated at the top of the final evidence report.
