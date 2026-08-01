import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import test from "node:test";
import assert from "node:assert/strict";

const execFileAsync = promisify(execFile);
const packageRoot = dirname(fileURLToPath(import.meta.url));
const cli = join(packageRoot, "..", "cli.mjs");

async function run(args, env = {}) {
  try {
    const result = await execFileAsync(process.execPath, [cli, ...args], {
      env: { ...process.env, AU_PLATFORM: "win32", AU_ARCH: "x64", ...env },
      windowsHide: true,
      timeout: 20_000,
      maxBuffer: 512 * 1024,
    });
    return { code: 0, stdout: result.stdout.trim(), stderr: result.stderr.trim() };
  } catch (error) {
    return { code: error.code ?? 1, stdout: String(error.stdout || "").trim(), stderr: String(error.stderr || "").trim() };
  }
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "android-use-installer-"));
  const installRoot = join(root, "install");
  const codexHome = join(root, "codex");
  const host = Buffer.from("fake au executable");
  const helper = Buffer.from("fake helper apk");
  const hostPath = join(root, "au.exe");
  const helperPath = join(root, "dev.codex.aubridge.apk");
  await writeFile(hostPath, host);
  await writeFile(helperPath, helper);
  const manifestPath = join(root, "release-manifest.json");
  await writeFile(manifestPath, JSON.stringify({
    schema: 1,
    product: "android-use",
    version: "1.0.0",
    protocol_version: 1,
    assets: {
      host_windows_x64: { url: pathToFileURL(hostPath).href, bytes: host.length, sha256: digest(host) },
      helper_apk: { url: pathToFileURL(helperPath).href, bytes: helper.length, sha256: digest(helper) },
    },
  }));
  return { root, installRoot, codexHome, manifestPath };
}

test("reports version without contacting the release endpoint", async () => {
  const result = await run(["--version", "--json"]);
  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), { ok: true, version: "1.0.0" });
});

test("installs and removes a skill-only payload without touching the network", async () => {
  const f = await fixture();
  try {
    const installed = await run(["install", "--skill-only", "--install-root", f.installRoot, "--json"], { CODEX_HOME: f.codexHome });
    assert.equal(installed.code, 0, installed.stdout + installed.stderr);
    const skill = join(f.codexHome, "skills", "android-use", "SKILL.md");
    assert.match(await readFile(skill, "utf8"), /name: android-use/);
    const doctor = await run(["doctor", "--install-root", f.installRoot, "--json"], { CODEX_HOME: f.codexHome });
    assert.equal(doctor.code, 0);
    assert.equal(JSON.parse(doctor.stdout).skill_exists, true);
    const removed = await run(["uninstall", "--install-root", f.installRoot, "--json"], { CODEX_HOME: f.codexHome });
    assert.equal(removed.code, 0, removed.stdout + removed.stderr);
    assert.equal(JSON.parse(removed.stdout).skill, "removed");
  } finally {
    await rm(f.root, { recursive: true, force: true });
  }
});

test("streams and verifies local release assets before activation", async () => {
  const f = await fixture();
  try {
    const result = await run(["install", "--manifest", f.manifestPath, "--with-helper", "--install-root", f.installRoot, "--json"], { CODEX_HOME: f.codexHome });
    assert.equal(result.code, 0, result.stdout + result.stderr);
    const staged = await run(["doctor", "--install-root", f.installRoot, "--json"], { CODEX_HOME: f.codexHome });
    assert.equal(staged.code, 0);
    const state = JSON.parse(staged.stdout);
    assert.equal(state.executable_exists, true);
    assert.equal(state.current.helper, true);
  } finally {
    await rm(f.root, { recursive: true, force: true });
  }
});

test("rejects a bad manifest hash without activating a binary", async () => {
  const f = await fixture();
  try {
    const raw = JSON.parse(await readFile(f.manifestPath, "utf8"));
    raw.assets.host_windows_x64.sha256 = "0".repeat(64);
    await writeFile(f.manifestPath, JSON.stringify(raw));
    const result = await run(["install", "--manifest", f.manifestPath, "--install-root", f.installRoot, "--json"], { CODEX_HOME: f.codexHome });
    assert.equal(result.code, 2);
    assert.match(result.stdout, /E_HASH/);
    const doctor = await run(["doctor", "--install-root", f.installRoot, "--json"], { CODEX_HOME: f.codexHome });
    assert.equal(JSON.parse(doctor.stdout).executable_exists, false);
  } finally {
    await rm(f.root, { recursive: true, force: true });
  }
});
