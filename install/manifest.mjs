// Checksum and manifest helpers shared by the launcher and the staging and publishing scripts.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const sha256 = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");

/** The `files` map ({ "relative/path": sha256 }) of `<dir>/manifest.json`; throws when it is missing or malformed. */
export function readManifest(dir) {
  let files;
  try {
    files = JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8")).files;
  } catch (e) {
    throw new Error(e.code === "ENOENT" ? "manifest.json is missing" : "manifest.json is unreadable");
  }
  if (files?.constructor !== Object) throw new Error("manifest.json lists no files");
  return files;
}

/** Writes `<dir>/manifest.json` with the SHA-256 of each named file (paths relative to `dir`). */
export function writeManifest(dir, names) {
  const files = Object.fromEntries(names.map((name) => [name, sha256(join(dir, name))]));
  writeFileSync(join(dir, "manifest.json"), `${JSON.stringify({ files }, null, 2)}\n`);
}

/** Parses `sha256sum` output into a Map of file name to digest. */
export const parseSums = (text) => new Map(text.trim().split("\n").map((line) => line.trim().split(/\s+/).reverse()));

/** The `gh attestation verify` arguments for a release asset built by this repository's release workflow from the tag. */
export const verifyArgs = (file, repo, tag) => ["attestation", "verify", file, "--repo", repo, "--signer-workflow", `${repo}/.github/workflows/release.yml`, "--source-ref", `refs/tags/${tag}`];

/** The `npm publish` arguments: provenance on every real publish (it needs the CI identity, so not on a dry run). */
export const publishArgs = (dryRun) => ["publish", "--access", "public", ...(dryRun ? ["--dry-run"] : ["--provenance"])];
