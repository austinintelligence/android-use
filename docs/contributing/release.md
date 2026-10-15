# Releasing

A release is a version bump merged to `main`, a tag, and then three publishing steps. Do them in this order; tags cannot be moved or deleted once pushed.

## 1. Prepare

- Move `CHANGELOG.md`'s unreleased entries under `## X.Y.Z — DATE` with a short **Highlights** paragraph. Breaking changes need a major bump and an upgrade note.
- Bump `VERSION`, the workspace version in `Cargo.toml`, `install/package.json`, `package-lock.json`, `install/cli.mjs`, the plugin and marketplace manifests, and the README status line. Also update the pins: `.mcp.json` (`android-use@X.Y.Z`), the `npx -y android-use@X.Y.Z` commands in the README, `install/README.md`, and the install guides, and the release-tag install URLs (`.../android-use/vX.Y.Z/install/...`). `cargo xtask version` fails until every one agrees, and also checks that `install/platforms.json`, `release.yml`, and both install scripts name the same platforms.
- Open the PR and let CI pass.

## 2. Rehearse

After the PR merges, run the release workflow by hand on `main` (Actions → Release → Run workflow). Without a tag it builds the helper, all six platform archives, and the SBOM, runs the release gates, and publishes nothing. Fix anything that fails now; a failure after tagging costs a version number.

## 3. Tag

```sh
git tag -a vX.Y.Z origin/main -m "android-use vX.Y.Z" && git push origin vX.Y.Z
```

The helper's `versionCode` is computed from `VERSION` (major×1,000,000 + minor×1,000 + patch), so a release needs no manual bump; `cargo xtask version` checks the formula. Only the release build type signs with the pinned key, and only when `AUBRIDGE_SIGNING_PROPERTIES` is set (the workflow sets it; a path that does not exist fails the build). A debug build is signed with the debug key, so uninstall a helper signed with the release key (`au uninstall`) before installing it. The pinned certificate hash in `device/release-signer.sha256` is compiled into `au`; rotating the key means changing that file in the same release. A newer `au` upgrades an older helper by itself on connect; going back to an older helper needs `au uninstall` first.

The workflow validates, builds the six archives with the signed helper embedded, writes `SHA256SUMS` and `release-manifest.json`, attests the assets, and creates the GitHub release with notes generated from the changelog.

## 4. Publish

Between the bump merging and this step, `main` names `android-use@X.Y.Z` and the `vX.Y.Z` install URLs before npm and the release exist, so the `npx` launcher in `.mcp.json` and the docs fail for anyone who pulls `main` then. Keep that window short: publish right after the tag, and do not announce the release until the npm step succeeds.

From a fresh clone of `main`, after the release exists:

```sh
node install/publish-npm.mjs vX.Y.Z --dry-run
node install/publish-npm.mjs vX.Y.Z        # needs `npm login` with 2FA; skips packages already published
node install/render-formula.mjs vX.Y.Z > Formula/android-use.rb   # in a checkout of austinintelligence/homebrew-tap
```

Commit and push the formula. npm can take several minutes to show new packages, and the first publish of each platform package needs the `androiduse` organization to exist.

## Lessons from 1.1

- The release job once failed because it never ran `npm ci`, and once because a size limit calibrated on macOS rejected the larger Linux binary. Both would have shown in a rehearsal.
- Linux binaries are about 30% larger than macOS ones from the same source; limits are per platform in `tools/src/main.rs`.
- The macOS Intel and Windows arm64 archives are cross-built, so the workflow can't run them; test them on real hardware when you can.
