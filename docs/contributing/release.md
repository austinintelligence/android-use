# Releasing

A release is a version bump merged to `main`, a tag, and then three publishing steps. Do them in this order; tags cannot be moved or deleted once pushed.

## 1. Prepare

- Move `CHANGELOG.md`'s unreleased entries under `## X.Y.Z — DATE` with a short **Highlights** paragraph. Breaking changes need a major bump and an upgrade note.
- Bump `VERSION`, the workspace version in `Cargo.toml`, `install/package.json`, `package-lock.json`, `install/cli.mjs`, the plugin and marketplace manifests, and the README status line. `cargo xtask version` fails until every one agrees, and also checks that `install/platforms.json`, `release.yml`, and both install scripts name the same platforms.
- Open the PR and let CI pass.

## 2. Rehearse

After the PR merges, run the release workflow by hand on `main` (Actions → Release → Run workflow). Without a tag it builds the unsigned helper, all six platform archives (embedding the unsigned helper), and the SBOM, runs the release gates, and stops before signing: the signing and publishing jobs need a `v*` tag, so nothing is signed or published. It can run from any branch. Fix anything that fails now; a failure after tagging costs a version number.

## 3. Tag

```sh
git tag -a vX.Y.Z origin/main -m "android-use vX.Y.Z" && git push origin vX.Y.Z
```

The workflow first checks that the tag's commit is on `main` and that CI passed on it. It then validates and builds the unsigned helper, signs it in an isolated job, builds the six archives with the signed helper embedded, writes `SHA256SUMS` and `release-manifest.json`, attests the assets, and creates the GitHub release with notes generated from the changelog.

### How signing is isolated

The `release-signing` job is the only one with the signing secrets. It has no checkout and runs no repository code, no npm, no cargo: it downloads the unsigned APK artifact and runs the SDK's `apksigner`. It reads `device/release-signer.sha256` at the tagged commit through the GitHub API, then runs `apksigner verify --print-certs` and fails unless there is exactly one signer whose certificate SHA-256 equals that pin. `cargo xtask workflow-policy` fails if a job with signing secrets is outside the `release-signing` environment, or if that job uses a checkout, setup action, cargo, npm, gradle, or xtask.

### What the repository owner sets up once

These are repository settings; the workflow cannot create them.

1. **Environments** (Settings → Environments): create `release-signing`, `release`, and `npm`. For each, set deployment branches and tags to *Selected branches and tags* and add one tag rule, `v*`, so a branch or a non-release tag can never use them.
2. **Required reviewer** on `release-signing` (and preferably `release`), with "Prevent self-review" on, so a person approves each signing.
3. **Signing secrets**: move `AUBRIDGE_KEYSTORE_B64`, `AUBRIDGE_STORE_PASSWORD`, and `AUBRIDGE_KEY_PASSWORD` out of the repository secrets into the `release-signing` environment, then delete the repository-level copies.
4. **Tag ruleset** (Settings → Rules → Rulesets): target tags `v*`, restrict creation to maintainers, and block updates and deletions.
5. `npm` is the environment of the workflow's `npm` job, the only job that publishes to npm. It holds no secret: npm trusted publishing signs in with the workflow's OIDC identity. Register the trusted publisher on npmjs.com (step 4 below).

Steps 1 to 4 are preconditions, not guards the workflow enforces. GitHub creates a missing environment on first use, and repository-level secrets stay readable by any job until step 3 is done. Until `npm` is limited to `v*` tags, a writer can run a modified `release.yml` from a branch in that environment, and npm's trusted-publisher match (workflow file and environment, not ref) would accept it. Finish the settings before the first tag.

## 4. Publish

The `npm` job runs after the GitHub Release exists. It runs `node install/publish-npm.mjs`, which downloads the release assets, runs `gh attestation verify` on each (any failure stops it), checks the archives against the verified `SHA256SUMS`, stages the packages, and publishes all seven (`android-use` and the six `@androiduse/<platform>`) with `--provenance`. It skips a package already on npm, so a re-run completes a partial publish. Outside GitHub Actions it refuses to publish; `--dry-run` rehearses everything but the publish.

The job also renders the Homebrew formula from the verified `SHA256SUMS` and uploads it as the `homebrew-formula` artifact. Commit it to `Formula/android-use.rb` in `austinintelligence/homebrew-tap`. To render one by hand, run `node install/render-formula.mjs vX.Y.Z`, which verifies the checksums itself; it refuses a `SHA256SUMS` file unless you pass `--verified` after checking it.

`cargo xtask workflow-policy` allows `id-token: write` only in the `release` and `npm` environments and npm publishing only in `npm`.

**Owner steps (once, on npmjs.com).** For each of the seven packages, open Settings → Trusted publisher and register GitHub Actions with repository `austinintelligence/android-use`, workflow `release.yml`, environment `npm`. A package that does not exist yet needs a first publish by an owner (or the `androiduse` organization) before it can have a trusted publisher. Then set each package to *Require two-factor authentication and disallow tokens*, and revoke any old automation token.

## Lessons from 1.1

- The release job once failed because it never ran `npm ci`, and once because a size limit calibrated on macOS rejected the larger Linux binary. Both would have shown in a rehearsal.
- Linux binaries are about 30% larger than macOS ones from the same source; limits are per platform in `tools/src/main.rs`.
- The macOS Intel and Windows arm64 archives are cross-built, so the workflow can't run them; test them on real hardware when you can.
