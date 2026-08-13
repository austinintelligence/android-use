# Verify a release before installation

Checksums detect accidental or malicious changes only when the checksum itself comes from a trusted source. For a published android-use release, verify both the artifact bytes and the build provenance before running the installer.

## 1. Verify immutable release metadata

With a current GitHub CLI, from the canonical repository:

```text
gh release verify <tag>
```

Then verify each downloaded release asset is the exact asset attached to that release:

```text
gh release verify-asset <tag> <path-to-asset>
```

These commands require a release published as immutable. If the release is not immutable, stop and use a source checkout or another trusted build channel.

## 2. Verify build provenance

For each binary archive and helper APK that has a GitHub artifact attestation:

```text
gh attestation verify <path-to-artifact> -R <owner>/<repository>
```

Require the attestation subject digest to match the local artifact and require the signer repository/workflow to be the canonical android-use release workflow. An attestation proves provenance, not that the code is vulnerability-free.

## 3. Verify published SHA-256 values

Compare the local SHA-256 digest against the checksum manifest covered by the verified release/attestation. Do not trust a checksum file downloaded from the same unverified location as the artifact by itself.

## 4. Verify the Android helper

Before installation, the release validator must confirm:

- expected application ID and protocol metadata
- expected signer/certificate continuity for updates
- `android:debuggable=false`
- release bootstrap and bridge components are not exported to ordinary apps
- exact foreground-service permission/type mask
- no instrumentation/test package in the release bundle

After installation, `au doctor` must verify the enrolled device identity, helper version/protocol, certificate continuity, accessibility state, and absence of stale temporary forwards before any mutation.

## CI trust requirements

- Pin every GitHub Action to a full-length commit SHA.
- Grant the workflow token only `contents: read` by default; grant `id-token: write`, `attestations: write`, or release-write permissions only to the jobs that need them.
- Build release artifacts in CI from a clean commit, generate SBOM and provenance attestations there, and do not attest arbitrary local outputs.
- Publish checksums, SBOMs, and attestations for the exact final archives/APK—not intermediate files.
- Keep signing credentials out of logs and untrusted pull-request jobs.

Official references:

- [GitHub: verifying release integrity](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/verify-release-integrity)
- [GitHub: artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations)
- [GitHub: secure use of Actions](https://docs.github.com/en/actions/reference/security/secure-use)
