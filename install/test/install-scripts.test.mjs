import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const installSh = join(dirname(fileURLToPath(import.meta.url)), "..", "install.sh");
const sh = spawnSync("sh", ["-c", ":"]);

test("install.sh has valid POSIX sh syntax", { skip: sh.error ? "sh is not available" : false }, () => {
  const r = spawnSync("sh", ["-n", installSh], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
});

test("install.sh verifies the checksum before extracting", async () => {
  const text = await readFile(installSh, "utf8");
  const verified = text.indexOf("SHA256 mismatch");
  assert.ok(verified > 0, "missing checksum mismatch check");
  assert.ok(verified < text.indexOf("tar -xzf"), "archive is extracted before it is verified");
});

test("install.sh refuses bad versions, http, bad checksums, and a missing required attestation", { skip: sh.error ? "sh is not available" : false }, () => {
  const r = spawnSync("sh", [join(dirname(installSh), "test", "install-sh.test.sh")], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stdout + r.stderr);
});

test("install.ps1 keeps the same guards as install.sh", async () => {
  const text = await readFile(join(dirname(installSh), "install.ps1"), "utf8");
  for (const part of ["Tls12", "AU_REQUIRE_ATTESTATION", "attestation verify", "Move-Item", "releases/latest"]) assert.ok(text.includes(part), part);
  assert.doesNotMatch(text, /api\.github/, "install.ps1 must not use the GitHub API");
});
