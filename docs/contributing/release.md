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

The `release-signing` job is the only one with the signing secrets. It has no checkout and runs no repository code, no npm, no cargo: it downloads the unsigned APK artifact and runs the SDK's `apksigner`, using only the artifact actions. It reads `device/release-signer.sha256` at the tagged commit through the GitHub API, then runs `apksigner verify --print-certs` and fails unless there is exactly one signer whose certificate SHA-256 equals that pin. `cargo xtask workflow-policy` fails if secrets are referenced outside the `release-signing` job, or if that job uses a checkout, setup action, cargo, npm, gradle, xtask, git, curl, wget, a local script, an interpreter, a pipe into a shell, or any action but the artifact actions, or lacks the ready switch.

### What the repository owner sets up once

These are repository settings; the workflow cannot create them.

1. **Environments** (Settings → Environments): create `release-signing`, `release`, and `npm`. For each, set deployment branches and tags to *Selected branches and tags* and add one tag rule, `v*`, so a branch or a non-release tag can never use them.
2. **Required reviewer** on `release-signing` (and preferably `release`), with "Prevent self-review" on, so a person approves each signing.
3. **Signing secrets**: move `AUBRIDGE_KEYSTORE_B64`, `AUBRIDGE_STORE_PASSWORD`, and `AUBRIDGE_KEY_PASSWORD` out of the repository secrets into the `release-signing` environment, then delete the repository-level copies.
4. **Tag ruleset** (Settings → Rules → Rulesets): target tags `v*`, restrict creation to maintainers, and block updates and deletions.
5. **Ready switch**: only after steps 1 to 4, add the *environment variable* `RELEASE_SIGNING_READY` = `true` to `release-signing` (not a repository variable). The `sign` job refuses to run without it. A repository or organization variable of the same name would also satisfy it and re-open the unprotected path, so check Settings → Secrets and variables → Actions → Variables for one and delete it. This matters because GitHub creates a missing environment on first use with no protection, and repository-level secrets stay visible to environment jobs: skipping the setup would otherwise let a `v*` tag sign with no reviewer.
6. `npm` is reserved for npm publishing. Step 4 below is still run by hand, with 2FA; put any future npm token in this environment, not in repository secrets.

The `gate` job and the policy check live in the workflow at the tagged commit, so someone who can push a `v*` tag can tag a commit whose workflow has neither. They catch mistakes, not deliberate misuse; the required reviewer and the tag ruleset (steps 2 and 4) are what stop that.

The policy check reads the YAML as text and finds jobs by indentation: it is a tripwire for ordinary mistakes, and changes under `.github/` still need a careful human review (and a CODEOWNERS entry).

If you push the tag while CI is still running on its commit, `gate` fails; re-run the failed jobs once CI is green. Runs are serialised per tag. A dry run's archives embed the unsigned helper and are named like the real ones: never distribute them.

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
