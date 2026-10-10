import test from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const run = promisify(execFile);
const here = dirname(fileURLToPath(import.meta.url));
const stage = join(here, "..", "stage-platform.mjs");
const body = "fake au binary";

// Stages a fake binary for one platform key and returns the parsed package, the manifest, and the output dir.
async function stageFake(t, key, binName) {
  const dir = await mkdtemp(join(tmpdir(), "android-use-stage-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const source = join(dir, "built-au");
  const out = join(dir, "out");
  await writeFile(source, body);
  await run(process.execPath, [stage, key, source, out]);
  const pkg = JSON.parse(await readFile(join(out, "package.json"), "utf8"));
  const manifest = JSON.parse(await readFile(join(out, "manifest.json"), "utf8"));
  return { pkg, manifest, out, binName };
}

test("stages a darwin-arm64 package with every required field", async (t) => {
  const { pkg, manifest, out } = await stageFake(t, "darwin-arm64");
  const install = JSON.parse(await readFile(join(here, "..", "package.json"), "utf8"));
  const version = (await readFile(join(here, "..", "..", "VERSION"), "utf8")).trim();
  assert.equal(pkg.name, "@androiduse/darwin-arm64");
  assert.equal(pkg.version, version);
  assert.deepEqual(pkg.os, ["darwin"]);
  assert.deepEqual(pkg.cpu, ["arm64"]);
  assert.deepEqual(pkg.files, ["bin", "README.md"]);
  assert.equal(pkg.license, "MIT");
  assert.deepEqual(pkg.repository, install.repository);
  assert.deepEqual(pkg.publishConfig, { access: "public" });
  assert.equal(await readFile(join(out, "bin", "au"), "utf8"), body);
  if (process.platform !== "win32") assert.equal((await stat(join(out, "bin", "au"))).mode & 0o777, 0o755);
  assert.deepEqual(manifest, { files: { "bin/au": createHash("sha256").update(body).digest("hex") } });
});

test("stages a win32-x64 package with au.exe", async (t) => {
  const { pkg, manifest, out } = await stageFake(t, "win32-x64");
  assert.equal(pkg.name, "@androiduse/win32-x64");
  assert.deepEqual(pkg.os, ["win32"]);
  assert.deepEqual(pkg.cpu, ["x64"]);
  assert.equal(await readFile(join(out, "bin", "au.exe"), "utf8"), body);
  assert.deepEqual(Object.keys(manifest.files), ["bin/au.exe"]);
});

test("rejects an unknown platform key", async (t) => {
  const { out } = await stageFake(t, "linux-x64");
  await assert.rejects(run(process.execPath, [stage, "freebsd-x64", join(out, "bin", "au"), out]), (e) => e.code === 2);
});

test("every platform in platforms.json can be staged", async () => {
  const { readFile } = await import("node:fs/promises");
  const platforms = JSON.parse(await readFile(new URL("../platforms.json", import.meta.url), "utf8"));
  assert.ok(platforms.length >= 6);
  for (const { npm } of platforms) {
    const [os, cpu] = npm.split("-");
    assert.ok(["darwin", "linux", "win32"].includes(os) && ["arm64", "x64"].includes(cpu), npm);
  }
});
