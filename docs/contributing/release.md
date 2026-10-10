# Releasing

A release is a version bump merged to `main`, a tag, and then three publishing steps. Do them in this order; tags cannot be moved or deleted once pushed.

## 1. Prepare

- Move `CHANGELOG.md`'s unreleased entries under `## X.Y.Z — DATE` with a short **Highlights** paragraph. Breaking changes need a minor or major bump and an upgrade note.
- Bump `VERSION`, the workspace version in `Cargo.toml`, `install/package.json`, `package-lock.json`, `install/cli.mjs`, the plugin and marketplace manifests, and the README status line. `cargo xtask version` fails until every one agrees, and also checks that `install/platforms.json`, `release.yml`, and both install scripts name the same platforms.
- Open the PR and let CI pass.

## 2. Rehearse

After the PR merges, run the release workflow by hand on `main` (Actions → Release → Run workflow). Without a tag it builds the helper, all six platform archives, and the SBOM, runs the release gates, and publishes nothing. Fix anything that fails now; a failure after tagging costs a version number.

## 3. Tag

```sh
git tag -a vX.Y.Z origin/main -m "android-use vX.Y.Z" && git push origin vX.Y.Z
```

The workflow validates, builds the six archives with the signed helper embedded, writes `SHA256SUMS` and `release-manifest.json`, attests the assets, and creates the GitHub release with notes generated from the changelog.

## 4. Publish

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
