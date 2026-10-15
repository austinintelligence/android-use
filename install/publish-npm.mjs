#!/usr/bin/env node
// Publish a GitHub release to npm: one @androiduse/<platform> package per supported platform, then android-use.
// Usage: node install/publish-npm.mjs vX.Y.Z [--dry-run]
// Runs in the release workflow's `npm` job, where npm trusted publishing (OIDC) signs in and --provenance
// records the build; no npm token exists. Outside CI it refuses unless --dry-run. Every downloaded asset must pass
// `gh attestation verify`, and the archives must match the attested SHA256SUMS, before anything is staged.
import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, readdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseSums, publishArgs, sha256, verifyArgs, writeManifest } from "./manifest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const [tag, ...flags] = process.argv.slice(2);
const dryRun = flags.includes("--dry-run");
if (!/^v\d+\.\d+\.\d+$/.test(tag ?? "")) {
  console.error("Usage: node install/publish-npm.mjs vX.Y.Z [--dry-run]");
  process.exit(2);
}
if (!dryRun && process.env.GITHUB_ACTIONS !== "true") {
  console.error("Refusing to publish outside GitHub Actions: releases publish from the `npm` job. Use --dry-run to rehearse.");
  process.exit(2);
}
const version = tag.slice(1);
if (version !== readFileSync(join(here, "..", "VERSION"), "utf8").trim()) {
  console.error(`${tag} does not match VERSION in this checkout; check out the release first.`);
  process.exit(2);
}

const run = (cmd, args, cwd) => execFileSync(cmd, args, { cwd, stdio: "inherit" });
const work = mkdtempSync(join(tmpdir(), "au-npm-"));
run("gh", ["release", "download", tag, "--dir", work, "-p", "*.tar.gz", "-p", "*.zip", "-p", "SHA256SUMS"]);

const repo = process.env.GITHUB_REPOSITORY ?? "austinintelligence/android-use";
for (const name of readdirSync(work)) run("gh", verifyArgs(join(work, name), repo, tag));

const sums = parseSums(readFileSync(join(work, "SHA256SUMS"), "utf8"));
const platforms = JSON.parse(readFileSync(join(here, "platforms.json"), "utf8")).map((p) => [p.npm, `android-use-${tag}-${p.asset}.${p.archive}`, p.binary]);
const published = (name) => {
  try {
    execFileSync("npm", ["view", `${name}@${version}`, "version", "--prefer-online"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
};
// Skips a package that is already on npm, so a run that stopped part-way can be repeated.
const publish = (dir, name) => {
  if (!dryRun && published(name)) return console.log(`${name}@${version} is already published`);
  run("npm", publishArgs(dryRun), dir);
};

for (const [key, archive, binary] of platforms) {
  const file = join(work, archive);
  if (sums.get(archive) !== sha256(file)) throw new Error(`${archive} does not match SHA256SUMS`);
  const unpacked = join(work, key);
  mkdirSync(unpacked);
  if (archive.endsWith(".zip")) run("unzip", ["-q", file, binary, "-d", unpacked]);
  else run("tar", ["-xzf", file, "-C", unpacked, binary]);
  const staged = join(work, `pkg-${key}`);
  run("node", [join(here, "stage-platform.mjs"), key, join(unpacked, binary), staged]);
  publish(staged, `@androiduse/${key}`);
}

const main = join(work, "pkg-main");
mkdirSync(main);
for (const name of ["cli.mjs", "manifest.mjs", "LICENSE", "README.md", "package.json"]) cpSync(join(here, name), join(main, name));
const manifest = JSON.parse(readFileSync(join(main, "package.json"), "utf8"));
manifest.optionalDependencies = Object.fromEntries(platforms.map(([key]) => [`@androiduse/${key}`, version]));
writeFileSync(join(main, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
writeManifest(main, ["cli.mjs", "manifest.mjs"]);
publish(main, "android-use");
console.log(`${dryRun ? "Dry run complete" : "Published"}: android-use@${version}`);
