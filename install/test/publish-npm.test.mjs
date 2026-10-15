import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { publishArgs, verifyArgs } from "../manifest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const node = (script, ...args) => spawnSync(process.execPath, [join(here, "..", script), ...args], { encoding: "utf8", env: { PATH: "/nonexistent" } });

test("publish-npm refuses to publish outside CI without --dry-run", () => {
  const r = node("publish-npm.mjs", "v1.3.0");
  assert.equal(r.status, 2);
  assert.match(r.stderr, /outside GitHub Actions/);
});

test("a real publish passes --provenance and a dry run does not", () => {
  assert.ok(publishArgs(false).includes("--provenance"));
  assert.ok(!publishArgs(true).includes("--provenance"));
  assert.ok(publishArgs(true).includes("--dry-run"));
});

test("assets are verified against this repository's release workflow", () => {
  assert.deepEqual(verifyArgs("f", "o/r", "v1.3.0").slice(-4), ["--signer-workflow", "o/r/.github/workflows/release.yml", "--source-ref", "refs/tags/v1.3.0"]);
});

test("render-formula refuses checksums that were not verified", () => {
  for (const args of [["v1.3.0", "SHA256SUMS"], ["v1.3.0", "SHA256SUMS", "--trust-me"]]) {
    const r = node("render-formula.mjs", ...args);
    assert.equal(r.status, 2);
    assert.match(r.stderr, /not attestation-verified/);
  }
});
