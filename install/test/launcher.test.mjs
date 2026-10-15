import test from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { writeManifest } from "../manifest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const platform = `${process.platform}-${process.arch}`;
const skip = process.platform === "win32" && "needs a POSIX shell script as the fake au";
const fake = (message) => `#!/bin/sh\necho ${message}\n`;

// A copy of the launcher with a fake au for this platform; `manifest` false leaves manifest.json out.
function pkg(t, { manifest = true, body = fake("ran-au") } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "android-use-launcher-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const f of ["cli.mjs", "manifest.mjs"]) copyFileSync(join(here, "..", f), join(dir, f));
  mkdirSync(join(dir, "bin", platform), { recursive: true });
  const bin = join(dir, "bin", platform, "au");
  writeFileSync(bin, body);
  chmodSync(bin, 0o755);
  if (manifest) writeManifest(dir, [`bin/${platform}/au`]);
  return { dir, bin, launch: (env = {}) => spawnSync(process.execPath, [join(dir, "cli.mjs"), "status"], { encoding: "utf8", env: { ...process.env, AU_BIN: "", ...env } }) };
}

test("a verified binary runs", { skip }, (t) => {
  const r = pkg(t).launch();
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout.trim(), "ran-au");
});

test("a missing manifest exits 1 without running the binary", { skip }, (t) => {
  const r = pkg(t, { manifest: false }).launch();
  assert.equal(r.status, 1);
  assert.match(r.stderr, /manifest\.json is missing/);
  assert.equal(r.stdout, "");
});

test("a manifest with no entry for this platform exits 1", { skip }, (t) => {
  const p = pkg(t);
  writeFileSync(join(p.dir, "manifest.json"), JSON.stringify({ files: { "bin/other-platform/au": "0".repeat(64) } }));
  const r = p.launch();
  assert.equal(r.status, 1);
  assert.match(r.stderr, new RegExp(`no entry for ${platform}`));
  assert.equal(r.stdout, "");
});

test("a tampered binary exits 1", { skip }, (t) => {
  const p = pkg(t);
  writeFileSync(p.bin, fake("tampered"));
  const r = p.launch();
  assert.equal(r.status, 1);
  assert.match(r.stderr, /integrity check/);
  assert.equal(r.stdout, "");
});

test("AU_BIN runs unchecked and warns on stderr every time", { skip }, (t) => {
  const p = pkg(t, { manifest: false });
  const other = join(p.dir, "other-au");
  writeFileSync(other, fake("override"));
  chmodSync(other, 0o755);
  for (let i = 0; i < 2; i++) {
    const r = p.launch({ AU_BIN: other });
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stdout.trim(), "override");
    assert.match(r.stderr, /AU_BIN is set/);
  }
});

test("signals reach the child and the launcher exits with its code", { skip, timeout: 15000 }, async (t) => {
  const p = pkg(t, { body: '#!/bin/sh\ntrap \'echo got-term; exit 7\' TERM\necho ready\nwhile :; do sleep 0.1; done\n' });
  const child = spawn(process.execPath, [join(p.dir, "cli.mjs"), "status"], { env: { ...process.env, AU_BIN: "" } });
  let out = "";
  const ready = new Promise((resolve) => child.stdout.on("data", (d) => ((out += d), out.includes("ready") && resolve())));
  await ready;
  const exited = new Promise((resolve) => child.on("exit", (code, signal) => resolve({ code, signal })));
  child.kill("SIGTERM");
  assert.deepEqual(await exited, { code: 7, signal: null });
  assert.match(out, /got-term/);
});
