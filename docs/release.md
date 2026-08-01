# Release process

1. Run the Rust, helper, installer, APK, security, and skill validators.
2. Build the Windows x64 release and the signed helper with the pinned toolchain.
3. Generate SHA-256/byte manifests and a dependency notice from the exact artifacts.
4. Run USB and matching Wi-Fi live lanes separately; classify every feature as passed, failed, or unsupported with direct evidence.
5. Run cold, daemon, persistent-shell, semantic, and batch benchmarks with warmup/sample counts and p50/p95.
6. Scan the publish candidate for serials, tokens, private paths, recordings, screenshots, signing material, and stale canvas/HTTP surfaces.
7. Create a clean public commit/history and push only the audited public tree.
8. Publish immutable GitHub release assets and the release manifest. Publish the npm installer from `packages/installer` through an owner-approved npm account/trusted publisher.
9. Install from the public GitHub skill and NPX package in clean temporary roots; verify update, rollback, uninstall, and reinstall.
10. Record the final cleanup proof and keep any failed or unsupported gate visible.

The detailed artifact, checksum, SBOM, attestation, and npm provenance
procedure is in [`supply-chain.md`](supply-chain.md).

The npm release workflow uses Node 24, requests GitHub OIDC, and supports either
an owner-configured trusted publisher or an owner-managed `NPM_TOKEN`. npm's
current trusted-publishing flow requires a public repository/package and an
OIDC relationship configured on npm before attempting `npm publish`.

Do not silently reclassify a failed wireless performance gate as stable. Release status is `STABLE`, `PRERELEASE`, `BLOCKED`, or `FAILED` and must be stated at the top of the final evidence report.
