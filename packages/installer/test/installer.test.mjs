import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { access, cp, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
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

async function runWith(cliPath, args, env = {}) {
  try {
    const result = await execFileAsync(process.execPath, [cliPath, ...args], {
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

async function run(args, env = {}) {
  return runWith(cli, args, env);
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function fixture({ bom = false, version = "1.0.0", hostText = "fake au executable" } = {}) {
  const root = await mkdtemp(join(tmpdir(), "android-use-installer-"));
  const installRoot = join(root, "install");
  const codexHome = join(root, "codex");
  const host = Buffer.from(hostText);
  const helper = Buffer.from("fake helper apk");
  const hostPath = join(root, "au.exe");
  const helperPath = join(root, "dev.codex.aubridge.apk");
  await writeFile(hostPath, host);
  await writeFile(helperPath, helper);
  const manifestPath = join(root, "release-manifest.json");
  const manifest = JSON.stringify({
    schema: 1,
    product: "android-use",
    version,
    protocol_version: 1,
    assets: {
      host_windows_x64: { url: pathToFileURL(hostPath).href, bytes: host.length, sha256: digest(host) },
      helper_apk: { url: pathToFileURL(helperPath).href, bytes: helper.length, sha256: digest(helper) },
    },
  });
  await writeFile(manifestPath, bom ? Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(manifest)]) : manifest);
  return { root, installRoot, codexHome, manifestPath };
}

async function packageCli(root, version) {
  const copy = join(root, `package-${version}`);
  await cp(join(packageRoot, ".."), copy, { recursive: true });
  const packageFile = join(copy, "package.json");
  const packageValue = JSON.parse(await readFile(packageFile, "utf8"));
  packageValue.version = version;
  await writeFile(packageFile, `${JSON.stringify(packageValue)}\n`);
  return join(copy, "cli.mjs");
}

test("reports version without contacting the release endpoint", async () => {
  const result = await run(["--version", "--json"]);
  assert.equal(result.code, 0);
  assert.deepEqual(JSON.parse(result.stdout), { ok: true, version: "1.0.0" });
});

test("selects a portable host asset and Unix executable name", async () => {
  const f = await fixture();
  try {
    const raw = JSON.parse(await readFile(f.manifestPath, "utf8"));
    raw.assets.host_linux_arm64 = raw.assets.host_windows_x64;
    await writeFile(f.manifestPath, JSON.stringify(raw));
    const result = await run([
      "install", "--dry-run", "--manifest", f.manifestPath,
      "--install-root", f.installRoot, "--json",
    ], { AU_PLATFORM: "linux", AU_ARCH: "arm64", CODEX_HOME: f.codexHome });
    assert.equal(result.code, 0, result.stdout + result.stderr);
    const value = JSON.parse(result.stdout);
    assert.equal(value.platform, "linux-arm64");
    assert.equal(value.executable, join(f.installRoot, "bin", "au"));
  } finally {
    await rm(f.root, { recursive: true, force: true });
  }
});

test("plans one-command setup without invoking an unstaged host", async () => {
  const f = await fixture();
  try {
    const result = await run([
      "setup", "--dry-run", "--manifest", f.manifestPath, "--agent", "auto",
      "--install-root", f.installRoot, "--json",
    ], { CODEX_HOME: f.codexHome });
    assert.equal(result.code, 0, result.stdout + result.stderr);
    const value = JSON.parse(result.stdout);
    assert.equal(value.dry_run, true);
    assert.equal(value.install_required, true);
    assert.equal(value.helper_install_requested, true);
    assert.equal(value.path_update, true);
    assert.equal(value.executable, join(f.installRoot, "bin", "au.exe"));
    assert.equal(value.helper, join(f.installRoot, "versions", "1.0.0", "dev.codex.aubridge.apk"));
    assert.equal(value.skill, join(f.codexHome, "skills", "android-use"));
    assert.deepEqual(await readdir(f.installRoot).catch(() => []), []);
  } finally {
    await rm(f.root, { recursive: true, force: true });
  }
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
  const f = await fixture({ bom: true });
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

test("rejects a local manifest supplied through the network override", async () => {
  const f = await fixture();
  try {
    const result = await run(["install", "--host-only", "--install-root", f.installRoot, "--json"], {
      AU_MANIFEST_URL: pathToFileURL(f.manifestPath).href,
      CODEX_HOME: f.codexHome,
    });
    assert.equal(result.code, 2);
    assert.match(result.stdout, /E_URL/);
  } finally {
    await rm(f.root, { recursive: true, force: true });
  }
});

test("verifies a detached Ed25519 manifest before trusting its asset hashes", async () => {
  const f = await fixture();
  try {
    const signedCli = await packageCli(f.root, "1.0.0-signed-test");
    const packageFile = join(dirname(signedCli), "package.json");
    const packageValue = JSON.parse(await readFile(packageFile, "utf8"));
    packageValue.version = "1.0.0";
    await writeFile(packageFile, `${JSON.stringify(packageValue)}\n`);
    const { privateKey, publicKey } = generateKeyPairSync("ed25519");
    await writeFile(
      join(dirname(signedCli), "release-public-key.pem"),
      publicKey.export({ format: "pem", type: "spki" }),
    );
    const keyId = createHash("sha256")
      .update(publicKey.export({ format: "der", type: "spki" }))
      .digest("hex");
    const manifest = JSON.parse(await readFile(f.manifestPath, "utf8"));
    manifest.signing_key_id = keyId;
    manifest.helper_signer_sha256 = "a".repeat(64);
    const encoded = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
    await writeFile(f.manifestPath, encoded);
    const signaturePath = `${f.manifestPath}.sig`;
    await writeFile(signaturePath, `${JSON.stringify({
      schema: 1,
      algorithm: "ed25519",
      key_id: keyId,
      signature: sign(null, encoded, privateKey).toString("base64"),
    })}\n`);
    const verified = await runWith(signedCli, [
      "install", "--dry-run", "--manifest", f.manifestPath,
      "--manifest-signature", signaturePath,
      "--install-root", f.installRoot, "--json",
    ], { CODEX_HOME: f.codexHome });
    assert.equal(verified.code, 0, verified.stdout + verified.stderr);
    assert.equal(JSON.parse(verified.stdout).manifest_authenticated, true);

    manifest.version = "1.0.0-tampered";
    await writeFile(f.manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    const rejected = await runWith(signedCli, [
      "install", "--dry-run", "--manifest", f.manifestPath,
      "--manifest-signature", signaturePath,
      "--install-root", f.installRoot, "--json",
    ], { CODEX_HOME: f.codexHome });
    assert.equal(rejected.code, 2);
    assert.match(rejected.stdout, /E_SIGNATURE/);
  } finally {
    await rm(f.root, { recursive: true, force: true });
  }
});

test("updates, rolls back, preserves skill state, and purges only with confirmation", async () => {
  const first = await fixture({ version: "1.0.0", hostText: "host-v1" });
  const second = await fixture({ version: "1.0.1", hostText: "host-v2" });
  try {
    const cliV2 = await packageCli(first.root, "1.0.1");
    const firstInstall = await run(["install", "--manifest", first.manifestPath, "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(firstInstall.code, 0, firstInstall.stdout + firstInstall.stderr);
    const secondInstall = await runWith(cliV2, ["update", "--manifest", second.manifestPath, "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(secondInstall.code, 0, secondInstall.stdout + secondInstall.stderr);
    assert.deepEqual(await readFile(join(first.installRoot, "bin", "au.exe")), Buffer.from("host-v2"));

    const rolled = await runWith(cliV2, ["rollback", "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(rolled.code, 0, rolled.stdout + rolled.stderr);
    const state = JSON.parse(await readFile(join(first.installRoot, "current.json"), "utf8"));
    assert.equal(state.version, "1.0.0");
    assert.equal(state.skill, join(first.codexHome, "skills", "android-use"));
    assert.deepEqual(await readFile(join(first.installRoot, "bin", "au.exe")), Buffer.from("host-v1"));

    const denied = await runWith(cliV2, ["uninstall", "--purge", "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(denied.code, 2);
    assert.match(denied.stdout, /E_CONFIRM/);

    const removed = await runWith(cliV2, ["uninstall", "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(removed.code, 0, removed.stdout + removed.stderr);
    assert.equal(JSON.parse(removed.stdout).skill, "removed");
    await assert.doesNotReject(() => access(join(first.installRoot, "versions")));

    const reinstall = await run(["install", "--manifest", first.manifestPath, "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(reinstall.code, 0, reinstall.stdout + reinstall.stderr);
    const purged = await runWith(cliV2, ["uninstall", "--purge", "--yes", "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(purged.code, 0, purged.stdout + purged.stderr);
    assert.equal(JSON.parse(purged.stdout).purged, true);
    assert.deepEqual(await readFile(join(first.installRoot, "bin", "au.exe")).catch(() => null), null);
  } finally {
    await rm(first.root, { recursive: true, force: true });
    await rm(second.root, { recursive: true, force: true });
  }
});

test("leaves the active install unchanged after a staged update fails", async () => {
  const first = await fixture({ version: "1.0.0", hostText: "host-v1" });
  const second = await fixture({ version: "1.0.1", hostText: "host-v2" });
  try {
    const cliV2 = await packageCli(first.root, "1.0.1");
    const installed = await run(["install", "--manifest", first.manifestPath, "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(installed.code, 0, installed.stdout + installed.stderr);
    const bad = JSON.parse(await readFile(second.manifestPath, "utf8"));
    bad.assets.host_windows_x64.sha256 = "0".repeat(64);
    await writeFile(second.manifestPath, JSON.stringify(bad));
    const failed = await runWith(cliV2, ["update", "--manifest", second.manifestPath, "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(failed.code, 2);
    assert.match(failed.stdout, /E_HASH/);
    const doctor = await run(["doctor", "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    const state = JSON.parse(doctor.stdout);
    assert.equal(state.current.version, "1.0.0");
    assert.deepEqual(await readFile(join(first.installRoot, "bin", "au.exe")), Buffer.from("host-v1"));
    assert.deepEqual(await readdir(join(first.installRoot, "staging")), []);
  } finally {
    await rm(first.root, { recursive: true, force: true });
    await rm(second.root, { recursive: true, force: true });
  }
});

test("recovers the whole prior install after a process dies during activation", async () => {
  const first = await fixture({ version: "1.0.0", hostText: "host-v1" });
  const second = await fixture({ version: "1.0.1", hostText: "host-v2" });
  try {
    const cliV2 = await packageCli(first.root, "1.0.1");
    const installed = await run([
      "install", "--manifest", first.manifestPath,
      "--install-root", first.installRoot, "--json",
    ], { CODEX_HOME: first.codexHome });
    assert.equal(installed.code, 0, installed.stdout + installed.stderr);

    const crashed = await runWith(cliV2, [
      "update", "--manifest", second.manifestPath,
      "--install-root", first.installRoot, "--json",
    ], {
      CODEX_HOME: first.codexHome,
      AU_TEST_CRASH_INSTALL_PHASE: "after-host",
    });
    assert.equal(crashed.code, 97);
    assert.deepEqual(await readFile(join(first.installRoot, "bin", "au.exe")), Buffer.from("host-v2"));

    const bad = JSON.parse(await readFile(second.manifestPath, "utf8"));
    bad.assets.host_windows_x64.sha256 = "0".repeat(64);
    await writeFile(second.manifestPath, JSON.stringify(bad));
    const resumed = await runWith(cliV2, [
      "update", "--manifest", second.manifestPath,
      "--install-root", first.installRoot, "--json",
    ], { CODEX_HOME: first.codexHome });
    assert.equal(resumed.code, 2);
    assert.match(resumed.stdout, /E_HASH/);
    assert.deepEqual(await readFile(join(first.installRoot, "bin", "au.exe")), Buffer.from("host-v1"));
    assert.equal(JSON.parse(await readFile(join(first.installRoot, "current.json"), "utf8")).version, "1.0.0");
    assert.equal(await access(join(first.installRoot, "install-transaction.json")).then(() => true).catch(() => false), false);
  } finally {
    await rm(first.root, { recursive: true, force: true });
    await rm(second.root, { recursive: true, force: true });
  }
});

test("fails closed on tampered rollback bytes and out-of-root skill ownership", async () => {
  const first = await fixture({ version: "1.0.0", hostText: "host-v1" });
  const second = await fixture({ version: "1.0.1", hostText: "host-v2" });
  try {
    const cliV2 = await packageCli(first.root, "1.0.1");
    const installed = await run(["install", "--manifest", first.manifestPath, "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(installed.code, 0, installed.stdout + installed.stderr);
    const updated = await runWith(cliV2, ["update", "--manifest", second.manifestPath, "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(updated.code, 0, updated.stdout + updated.stderr);

    await writeFile(join(first.installRoot, "versions", "1.0.0", "au.exe"), "tampered");
    const rollback = await runWith(cliV2, ["rollback", "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(rollback.code, 2);
    assert.match(rollback.stdout, /E_ROLLBACK/);
    assert.deepEqual(await readFile(join(first.installRoot, "bin", "au.exe")), Buffer.from("host-v2"));

    const outside = join(first.root, "outside-skill");
    await writeFile(join(first.installRoot, "current.json"), JSON.stringify({
      product: "android-use",
      version: "1.0.1",
      host: true,
      skill: outside,
      skill_hashes: {},
    }));
    const uninstall = await runWith(cliV2, ["uninstall", "--install-root", first.installRoot, "--json"], { CODEX_HOME: first.codexHome });
    assert.equal(uninstall.code, 2);
    assert.match(uninstall.stdout, /E_OWNERSHIP/);
    assert.equal(await access(join(first.installRoot, "bin", "au.exe")).then(() => true).catch(() => false), true);
  } finally {
    await rm(first.root, { recursive: true, force: true });
    await rm(second.root, { recursive: true, force: true });
  }
});
