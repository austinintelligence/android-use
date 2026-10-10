#!/usr/bin/env node
// Publish a GitHub release to npm: three @android-use/<platform> packages, then android-use.
// Usage: node install/publish-npm.mjs vX.Y.Z [--dry-run]
// Needs `gh` and an npm login that can publish both names. Archives are checked against the
// release's SHA256SUMS before anything is staged.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const [tag, ...flags] = process.argv.slice(2);
const dryRun = flags.includes("--dry-run");
if (!/^v\d+\.\d+\.\d+$/.test(tag ?? "")) {
  console.error("Usage: node install/publish-npm.mjs vX.Y.Z [--dry-run]");
  process.exit(2);
}
const version = tag.slice(1);
if (version !== readFileSync(join(here, "..", "VERSION"), "utf8").trim()) {
  console.error(`${tag} does not match VERSION in this checkout; check out the release first.`);
  process.exit(2);
}

const run = (cmd, args, cwd) => execFileSync(cmd, args, { cwd, stdio: ["ignore", "inherit", "inherit"] });
const work = mkdtempSync(join(tmpdir(), "au-npm-"));
run("gh", ["release", "download", tag, "--dir", work, "-p", "*.tar.gz", "-p", "*.zip", "-p", "SHA256SUMS"]);

const sums = new Map(readFileSync(join(work, "SHA256SUMS"), "utf8").trim().split("\n").map((l) => l.trim().split(/\s+/).reverse()));
const platforms = [
  ["darwin-arm64", `android-use-${tag}-macos-aarch64.tar.gz`, "au"],
  ["linux-x64", `android-use-${tag}-linux-x86_64.tar.gz`, "au"],
  ["win32-x64", `android-use-${tag}-windows-x86_64.zip`, "au.exe"],
];
const publish = (dir) => run("npm", ["publish", "--access", "public", ...(dryRun ? ["--dry-run"] : [])], dir);

for (const [key, archive, binary] of platforms) {
  const file = join(work, archive);
  const digest = createHash("sha256").update(readFileSync(file)).digest("hex");
  if (sums.get(archive) !== digest) throw new Error(`${archive} does not match SHA256SUMS`);
  const unpacked = join(work, key);
  mkdirSync(unpacked);
  if (archive.endsWith(".zip")) run("unzip", ["-q", file, binary, "-d", unpacked]);
  else run("tar", ["-xzf", file, "-C", unpacked, binary]);
  const staged = join(work, `pkg-${key}`);
  run("node", [join(here, "stage-platform.mjs"), key, join(unpacked, binary), staged]);
  publish(staged);
}

const main = join(work, "pkg-main");
mkdirSync(main);
for (const name of ["cli.mjs", "LICENSE", "package.json"]) cpSync(join(here, name), join(main, name));
const manifest = JSON.parse(readFileSync(join(main, "package.json"), "utf8"));
manifest.optionalDependencies = Object.fromEntries(platforms.map(([key]) => [`@android-use/${key}`, version]));
writeFileSync(join(main, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
publish(main);
console.log(`${dryRun ? "Dry run complete" : "Published"}: android-use@${version}`);
