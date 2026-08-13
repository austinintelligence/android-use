#!/usr/bin/env node

import { createHash, createPublicKey, randomUUID, verify as verifySignature } from "node:crypto";
import { execFile } from "node:child_process";
import { createReadStream, existsSync } from "node:fs";
import { appendFile, chmod, copyFile, lstat, mkdir, open, readFile, readdir, readlink, realpath, rename, rm, stat, symlink, writeFile } from "node:fs/promises";
import { homedir, platform, arch } from "node:os";
import { dirname, basename, isAbsolute, join, resolve, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const packageRoot = dirname(fileURLToPath(import.meta.url));
const packageJson = JSON.parse(await readFile(join(packageRoot, "package.json"), "utf8"));
const PRODUCT = "android-use";
const VERSION = packageJson.version;
const REPOSITORY = process.env.AU_REPOSITORY || "austinintelligence/android-use";
const DEFAULT_TIMEOUT_MS = 30_000;
const MAX_MANIFEST_BYTES = 256 * 1024;
const MAX_SIGNATURE_BYTES = 16 * 1024;
const MAX_ASSET_BYTES = 100 * 1024 * 1024;
const RELEASE_PUBLIC_KEY = createPublicKey(await readFile(join(packageRoot, "release-public-key.pem")));
const RELEASE_KEY_ID = createHash("sha256")
  .update(RELEASE_PUBLIC_KEY.export({ format: "der", type: "spki" }))
  .digest("hex");
const PLATFORM_TOOLS = Object.freeze({
  win32: Object.freeze({
    revision: "37.0.1",
    url: "https://dl.google.com/android/repository/platform-tools_r37.0.1-win.zip",
    bytes: 8_044_989,
    sha256: "45f4d63113e895ebde0c90f194099a4676b6ac653bd28d54314a9e022bbc1a99",
  }),
  darwin: Object.freeze({
    revision: "37.0.1",
    url: "https://dl.google.com/android/repository/platform-tools_r37.0.1-darwin.zip",
    bytes: 16_110_554,
    sha256: "ee39ad5967e95c2a07f04dbcbde96b1a0c916ba376096db5d2f498b7727a5d1d",
  }),
  linux: Object.freeze({
    revision: "37.0.1",
    url: "https://dl.google.com/android/repository/platform-tools_r37.0.1-linux.zip",
    bytes: 9_054_187,
    sha256: "d230f13842f60f782a8645f9c813f8f845bf36089ea7289f28c48f17979313f1",
  }),
});

class InstallerError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = "InstallerError";
    this.code = code;
    this.details = details;
  }
}

function parseArgs(argv) {
  const options = { json: false, dryRun: false, yes: false, quiet: false, addPath: false };
  const positional = [];
  let command = "help";
  let index = 0;
  if (argv[0] && !argv[0].startsWith("-")) {
    command = argv[0];
    index = 1;
  }
  const values = new Map([
    ["--agent", "agent"],
    ["--install-root", "installRoot"],
    ["--manifest", "manifestFile"],
    ["--manifest-signature", "manifestSignatureFile"],
  ]);
  while (index < argv.length) {
    const token = argv[index++];
    const key = values.get(token);
    if (key) {
      const value = argv[index++];
      if (!value || value.startsWith("-")) throw new InstallerError("E_ARGS", `${token} requires a value`);
      options[key] = value;
      continue;
    }
    if (token === "--json") options.json = true;
    else if (token === "--dry-run") options.dryRun = true;
    else if (token === "--yes") options.yes = true;
    else if (token === "--quiet") options.quiet = true;
    else if (token === "--with-helper") options.withHelper = true;
    else if (token === "--install-helper") options.installHelper = true;
    else if (token === "--skill-only") options.skillOnly = true;
    else if (token === "--host-only") options.hostOnly = true;
    else if (token === "--no-path") options.noPath = true;
    else if (token === "--add-path") options.addPath = true;
    else if (token === "--purge") options.purge = true;
    else if (token === "--repair") options.repair = true;
    else if (token === "--wait") options.wait = true;
    else if (token === "--version") options.version = true;
    else if (token === "--help" || token === "-h") options.help = true;
    else if (token === "--") positional.push(...argv.slice(index));
    else if (token.startsWith("-")) throw new InstallerError("E_ARGS", `unknown option ${token}`);
    else positional.push(token);
  }
  return { command, options, positional };
}

function installRoot(options) {
  if (options.installRoot || process.env.AU_INSTALL_ROOT) {
    return resolve(options.installRoot || process.env.AU_INSTALL_ROOT);
  }
  if (platform() === "win32") {
    const local = process.env.LOCALAPPDATA || join(homedir(), "AppData", "Local");
    return resolve(join(local, "Codex", PRODUCT));
  }
  if (platform() === "darwin") {
    return resolve(join(homedir(), "Library", "Application Support", PRODUCT));
  }
  const data = process.env.XDG_DATA_HOME || join(homedir(), ".local", "share");
  return resolve(join(data, PRODUCT));
}

function platformSpec(options = {}) {
  // Test-only overrides are accepted only with an explicit local manifest.
  // A network install always uses the real host OS and CPU.
  const localFixture = Boolean(options.manifestFile);
  const runtimePlatform = localFixture && process.env.AU_PLATFORM ? process.env.AU_PLATFORM : platform();
  const runtimeArch = localFixture && process.env.AU_ARCH ? process.env.AU_ARCH : arch();
  const osName = { win32: "windows", darwin: "macos", linux: "linux" }[runtimePlatform];
  const cpuName = { x64: "x64", arm64: "arm64" }[runtimeArch];
  if (!osName || !cpuName) {
    throw new InstallerError("E_PLATFORM", `unsupported host ${runtimePlatform}/${runtimeArch}; supported hosts are Windows, macOS, and Linux on x64 or arm64`);
  }
  return {
    runtimePlatform,
    runtimeArch,
    osName,
    cpuName,
    hostKey: `host_${osName}_${cpuName}`,
    platformToolsKey: `platform_tools_${osName}_${cpuName}`,
    executableName: runtimePlatform === "win32" ? "au.exe" : "au",
    adbName: runtimePlatform === "win32" ? "adb.exe" : "adb",
    archiveTool: runtimePlatform === "win32" ? "tar.exe" : "tar",
    // Google publishes Windows and macOS archives usable on both supported
    // CPU families. The Linux archive is x64-only; Linux ARM64 must provide a
    // native adb until Google publishes an official matching archive.
    officialPlatformTools: runtimePlatform === "linux" && runtimeArch === "arm64"
      ? null
      : PLATFORM_TOOLS[runtimePlatform] || null,
  };
}

function executablePath(options = {}) {
  return join(installRoot(options), "bin", platformSpec(options).executableName);
}

function paths(root) {
  return {
    root,
    versions: join(root, "versions"),
    staging: join(root, "staging"),
    bin: join(root, "bin"),
    platformTools: join(root, "platform-tools"),
    current: join(root, "current.json"),
    history: join(root, "history.json"),
    manifest: join(root, "release-manifest.json"),
    transaction: join(root, "install-transaction.json"),
  };
}

function samePath(left, right) {
  const normalize = (value) => resolve(value).replace(/[\\/]+$/, "").toLowerCase();
  return normalize(left) === normalize(right);
}

async function isProcessAlive(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

async function withInstallLock(root, callback) {
  await mkdir(root, { recursive: true });
  const lock = join(root, ".installer.lock");
  const nonce = randomUUID();
  let acquired = false;
  for (let attempt = 0; attempt < 2 && !acquired; attempt++) {
    try {
      await mkdir(lock);
      acquired = true;
    } catch (error) {
      if (error.code !== "EEXIST" || attempt !== 0) {
        throw new InstallerError("E_LOCK", "another android-use installer operation is active");
      }
      const owner = await readJson(join(lock, "owner.json"), null).catch(() => null);
      if (!owner) {
        const ageMs = Date.now() - (await stat(lock)).mtimeMs;
        if (ageMs < 5 * 60_000) {
          throw new InstallerError("E_LOCK", "another android-use installer is initializing its lock");
        }
      } else if (await isProcessAlive(Number(owner.pid))) {
        throw new InstallerError("E_LOCK", `another android-use installer operation is active (pid ${owner.pid})`);
      }
      await rm(lock, { recursive: true, force: true });
    }
  }
  if (!acquired) throw new InstallerError("E_LOCK", "could not acquire the android-use installer lock");
  try {
    await writeFile(join(lock, "owner.json"), `${JSON.stringify({ pid: process.pid, nonce, started_at: new Date().toISOString() })}\n`, { encoding: "utf8", flag: "wx" });
    return await callback();
  } finally {
    const owner = await readJson(join(lock, "owner.json"), null).catch(() => null);
    if (owner?.nonce === nonce) await rm(lock, { recursive: true, force: true });
  }
}

async function recoverAtomicBackups(root) {
  const parents = [root, join(root, "bin"), join(root, "versions")];
  for (const parent of parents) {
    const entries = await readdir(parent, { withFileTypes: true }).catch((error) => {
      if (error.code === "ENOENT") return [];
      throw error;
    });
    const backups = new Map();
    for (const entry of entries) {
      const marker = entry.name.indexOf(".previous-");
      if (marker <= 0) continue;
      const base = entry.name.slice(0, marker);
      const list = backups.get(base) || [];
      list.push(entry.name);
      backups.set(base, list);
    }
    for (const [base, names] of backups) {
      names.sort().reverse();
      const destination = join(parent, base);
      if (!existsSync(destination)) await rename(join(parent, names[0]), destination);
      for (const name of names.slice(existsSync(destination) ? 0 : 1)) {
        await rm(join(parent, name), { recursive: true, force: true });
      }
    }
  }
}

function pathIsWithin(root, candidate) {
  const value = relative(resolve(root), resolve(candidate));
  return value === "" || (!value.startsWith("..") && !isAbsolute(value));
}

async function snapshotTransactionTarget(rollbackRoot, target) {
  const backup = join(rollbackRoot, target.name);
  const existed = existsSync(target.path);
  if (existed) {
    if (target.kind === "directory") await copyTree(target.path, backup);
    else {
      await mkdir(dirname(backup), { recursive: true });
      await copyFile(target.path, backup);
    }
  }
  return { ...target, backup, existed };
}

async function beginInstallTransaction(root, p, staging, spec, versionDir, skillDestination, options) {
  const rollbackRoot = join(staging, "rollback");
  await mkdir(rollbackRoot, { recursive: true });
  const targets = [
    { name: "current", path: p.current, kind: "file" },
    { name: "history", path: p.history, kind: "file" },
    { name: "manifest", path: p.manifest, kind: "file" },
  ];
  if (!options.skillOnly) {
    targets.push(
      { name: "version", path: versionDir, kind: "directory" },
      { name: "host", path: join(p.bin, spec.executableName), kind: "file" },
      { name: "platform-tools", path: p.platformTools, kind: "directory" },
    );
  }
  if (!options.hostOnly) {
    targets.push({ name: "skill", path: skillDestination, kind: "directory" });
  }
  const snapshots = [];
  for (const target of targets) snapshots.push(await snapshotTransactionTarget(rollbackRoot, target));
  const journal = {
    schema: 1,
    operation: "install",
    phase: "activating",
    version: VERSION,
    agent: resolveAgent(options.agent || "codex"),
    staging,
    targets: snapshots,
    started_at: new Date().toISOString(),
  };
  await writeJsonAtomic(p.transaction, journal);
  return journal;
}

async function recoverInstallTransaction(root) {
  const p = paths(root);
  const journal = await readJson(p.transaction, null);
  if (!journal) return false;
  if (journal.schema !== 1 || journal.operation !== "install" || !Array.isArray(journal.targets)) {
    throw new InstallerError("E_RECOVERY", "install transaction journal is invalid");
  }
  if (!pathIsWithin(p.staging, journal.staging)) {
    throw new InstallerError("E_RECOVERY", "install transaction staging path is outside managed storage");
  }
  const expectedSkill = skillRoot(journal.agent || "codex", { installRoot: root });
  for (const target of journal.targets) {
    const owned = pathIsWithin(root, target.path)
      || (target.name === "skill" && samePath(target.path, expectedSkill));
    if (!owned || !pathIsWithin(journal.staging, target.backup)) {
      throw new InstallerError("E_RECOVERY", `install transaction target is not owned: ${target.name || "unknown"}`);
    }
    if (target.existed) {
      if (!existsSync(target.backup)) {
        throw new InstallerError("E_RECOVERY", `install transaction backup is missing: ${target.name}`);
      }
      if (target.kind === "directory") {
        const restore = `${target.backup}.restore-${process.pid}`;
        await rm(restore, { recursive: true, force: true });
        await copyTree(target.backup, restore);
        await replaceDirectoryAtomic(restore, target.path);
      } else {
        await copyBinaryAtomically(target.backup, target.path, false);
      }
    } else {
      await rm(target.path, { recursive: target.kind === "directory", force: true });
    }
  }
  await rm(p.transaction, { force: true });
  await rm(journal.staging, { recursive: true, force: true });
  return true;
}

async function assertOwnedSkillPath(skill, options) {
  const expected = resolve(skillRoot(options.agent || "codex", options));
  const actual = resolve(String(skill));
  if (!samePath(actual, expected)) {
    throw new InstallerError("E_OWNERSHIP", "stored skill path is outside the android-use-owned skill root");
  }
  if (existsSync(actual)) {
    const [actualReal, expectedReal] = await Promise.all([realpath(actual), realpath(expected)]);
    if (!samePath(actualReal, expectedReal)) {
      throw new InstallerError("E_OWNERSHIP", "stored skill path resolves outside the android-use-owned skill root");
    }
  }
  return actual;
}

function resolveAgent(agent) {
  if (!agent || agent !== "auto") return agent || "codex";
  return process.env.CODEX_HOME || existsSync(join(homedir(), ".codex")) ? "codex" : "jsonl";
}

function skillRoot(agent, options = {}) {
  const selected = resolveAgent(agent);
  if (selected === "codex") {
    const home = process.env.CODEX_HOME || join(homedir(), ".codex");
    return resolve(join(home, "skills", PRODUCT));
  }
  if (!["claude", "cursor", "gemini", "opencode", "mcp", "jsonl"].includes(selected)) {
    throw new InstallerError("E_AGENT", `unsupported agent ${selected}`);
  }
  // Do not guess or mutate third-party configuration locations. The portable
  // adapter payload lives under the AU install root and can be connected by
  // the agent-specific integration without weakening ownership boundaries.
  return resolve(join(installRoot(options), "agents", selected, "skill"));
}

function parseJsonLine(output) {
  const lines = String(output || "").split(/\r?\n/).map((line) => line.trim()).filter(Boolean).reverse();
  for (const line of lines) {
    try { return JSON.parse(line); } catch { /* continue to the previous line */ }
  }
  return null;
}

async function invokeHost(executable, args, options = {}) {
  try {
    const environment = { ...process.env };
    if (options.installRoot || process.env.AU_INSTALL_ROOT) {
      environment.AU_INSTALL_ROOT = installRoot(options);
    }
    const result = await execFileAsync(executable, args, {
      windowsHide: true,
      timeout: Number(process.env.AU_SETUP_TIMEOUT_MS || DEFAULT_TIMEOUT_MS),
      maxBuffer: 512 * 1024,
      env: environment,
    });
    return { ok: true, data: parseJsonLine(result.stdout) || { raw: result.stdout.trim() } };
  } catch (error) {
    const data = parseJsonLine(error.stdout);
    if (data) return { ok: false, data };
    throw new InstallerError("E_SETUP", `au ${args.join(" ")} failed: ${error.message}`);
  }
}

async function hostCommand(options, command, extra = []) {
  const executable = executablePath(options);
  if (!existsSync(executable)) throw new InstallerError("E_SETUP", "android-use host is not installed; run install first");
  return { executable, ...(await invokeHost(executable, [command, "-j", ...extra], options)) };
}

function output(ok, value, options) {
  if (options.quiet && ok) return;
  if (options.json) {
    process.stdout.write(`${JSON.stringify(ok ? { ok: true, ...value } : { ok: false, ...value })}\n`);
    return;
  }
  if (!ok) {
    process.stdout.write(`err ${value.code} ${value.message}\n`);
    return;
  }
  if (value.path) process.stdout.write(`ok ${value.path}\n`);
  else if (value.version) process.stdout.write(`ok ${value.version}\n`);
  else if (value.count !== undefined) process.stdout.write(`ok ${value.count}\n`);
  else process.stdout.write("ok\n");
}

function fail(error) {
  if (error instanceof InstallerError) return { code: error.code, message: error.message, ...error.details };
  return { code: "E_INSTALL", message: error instanceof Error ? error.message : String(error) };
}

async function readBytes(source, limit, allowFile = false) {
  if (source.startsWith("file://") && allowFile) {
    const file = new URL(source);
    const metadata = await stat(file);
    if (metadata.size > limit) throw new InstallerError("E_SIZE", `${source} exceeds ${limit} bytes`);
    return readFile(file);
  }
  if (!/^https:\/\//i.test(source)) {
    throw new InstallerError("E_URL", `release URL must use HTTPS: ${source}`);
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), Number(process.env.AU_INSTALL_TIMEOUT_MS || DEFAULT_TIMEOUT_MS));
  try {
    const response = await fetch(source, { signal: controller.signal, redirect: "follow" });
    if (!/^https:\/\//i.test(response.url)) {
      throw new InstallerError("E_URL", `release URL redirected away from HTTPS: ${source}`);
    }
    if (!response.ok) throw new InstallerError("E_DOWNLOAD", `${response.status} ${response.statusText}: ${source}`);
    const length = Number(response.headers.get("content-length") || 0);
    if (length > limit) throw new InstallerError("E_SIZE", `${source} exceeds ${limit} bytes`);
    const chunks = [];
    let total = 0;
    for await (const chunk of response.body) {
      total += chunk.byteLength;
      if (total > limit) throw new InstallerError("E_SIZE", `${source} exceeds ${limit} bytes`);
      chunks.push(Buffer.from(chunk));
    }
    return Buffer.concat(chunks, total);
  } catch (error) {
    if (error instanceof InstallerError) throw error;
    throw new InstallerError("E_DOWNLOAD", `${source}: ${error instanceof Error ? error.message : String(error)}`);
  } finally {
    clearTimeout(timer);
  }
}

async function downloadToFile(source, destination, limit) {
  await mkdir(dirname(destination), { recursive: true });
  const output = await open(destination, "wx");
  const digest = createHash("sha256");
  let total = 0;
  try {
    const write = async (chunk) => {
      total += chunk.byteLength;
      if (total > limit) throw new InstallerError("E_SIZE", `${source} exceeds ${limit} bytes`);
      digest.update(chunk);
      await output.write(chunk);
    };
    if (source.startsWith("file://")) {
      const input = await open(new URL(source), "r");
      try {
        const buffer = Buffer.allocUnsafe(64 * 1024);
        while (true) {
          const read = await input.read(buffer, 0, buffer.byteLength, null);
          if (read.bytesRead === 0) break;
          await write(buffer.subarray(0, read.bytesRead));
        }
      } finally {
        await input.close();
      }
    } else {
      if (!/^https:\/\//i.test(source)) {
        throw new InstallerError("E_URL", `release URL must use HTTPS: ${source}`);
      }
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), Number(process.env.AU_INSTALL_TIMEOUT_MS || DEFAULT_TIMEOUT_MS));
      try {
        const response = await fetch(source, { signal: controller.signal, redirect: "follow" });
        if (!/^https:\/\//i.test(response.url)) throw new InstallerError("E_URL", `release URL redirected away from HTTPS: ${source}`);
        if (!response.ok) throw new InstallerError("E_DOWNLOAD", `${response.status} ${response.statusText}: ${source}`);
        const length = Number(response.headers.get("content-length") || 0);
        if (length > limit) throw new InstallerError("E_SIZE", `${source} exceeds ${limit} bytes`);
        for await (const chunk of response.body) await write(chunk);
      } catch (error) {
        if (error instanceof InstallerError) throw error;
        throw new InstallerError("E_DOWNLOAD", `${source}: ${error instanceof Error ? error.message : String(error)}`);
      } finally {
        clearTimeout(timer);
      }
    }
    await output.sync();
    return { bytes: total, sha256: digest.digest("hex") };
  } catch (error) {
    await rm(destination, { force: true });
    throw error;
  } finally {
    await output.close();
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function readManifest(options) {
  const source = options.manifestFile
    ? pathToFileURL(resolve(options.manifestFile)).href
    : process.env.AU_MANIFEST_URL || `https://github.com/${REPOSITORY}/releases/download/v${VERSION}/release-manifest.json`;
  const trustedAssetPrefix = `https://github.com/${REPOSITORY}/releases/download/v${VERSION}/`;
  if (!/^https:\/\//i.test(source) && !options.manifestFile) {
    throw new InstallerError("E_URL", "release manifest must use HTTPS unless --manifest names a local fixture");
  }
  const raw = await readBytes(source, MAX_MANIFEST_BYTES, Boolean(options.manifestFile));
  let authenticated = false;
  if (!options.manifestFile || options.manifestSignatureFile) {
    const signatureSource = options.manifestSignatureFile
      ? pathToFileURL(resolve(options.manifestSignatureFile)).href
      : `${source}.sig`;
    if (!/^https:\/\//i.test(signatureSource) && !options.manifestSignatureFile) {
      throw new InstallerError("E_URL", "release manifest signature must use HTTPS");
    }
    const encodedSignature = await readBytes(
      signatureSource,
      MAX_SIGNATURE_BYTES,
      Boolean(options.manifestSignatureFile),
    );
    let signatureRecord;
    try {
      signatureRecord = JSON.parse(encodedSignature.toString("utf8"));
    } catch (error) {
      throw new InstallerError("E_SIGNATURE", `invalid release manifest signature record: ${error.message}`);
    }
    if (signatureRecord?.schema !== 1
        || signatureRecord.algorithm !== "ed25519"
        || signatureRecord.key_id !== RELEASE_KEY_ID
        || typeof signatureRecord.signature !== "string") {
      throw new InstallerError("E_SIGNATURE", "release manifest signature metadata is not trusted");
    }
    const signature = Buffer.from(signatureRecord.signature, "base64");
    if (signature.length !== 64 || !verifySignature(null, raw, RELEASE_PUBLIC_KEY, signature)) {
      throw new InstallerError("E_SIGNATURE", "release manifest signature verification failed");
    }
    authenticated = true;
  }
  let manifest;
  try {
    const text = raw.toString("utf8").replace(/^\uFEFF/, "");
    manifest = JSON.parse(text);
  } catch (error) {
    throw new InstallerError("E_MANIFEST", `invalid release manifest: ${error.message}`);
  }
  if (manifest.product !== PRODUCT || manifest.schema !== 1 || manifest.version !== VERSION) {
    throw new InstallerError("E_MANIFEST", `manifest does not match ${PRODUCT}@${VERSION}`);
  }
  if (authenticated && manifest.signing_key_id !== RELEASE_KEY_ID) {
    throw new InstallerError("E_SIGNATURE", "release manifest signing key id does not match the pinned key");
  }
  if (authenticated && !/^[a-f0-9]{64}$/i.test(String(manifest.helper_signer_sha256 || ""))) {
    throw new InstallerError("E_SIGNATURE", "authenticated release manifest has no valid helper signer continuity fingerprint");
  }
  if (!manifest.assets || typeof manifest.assets !== "object") throw new InstallerError("E_MANIFEST", "manifest has no assets");
  for (const [name, asset] of Object.entries(manifest.assets)) {
    if (!asset || typeof asset.url !== "string" || !/^[a-f0-9]{64}$/i.test(asset.sha256)) {
      throw new InstallerError("E_MANIFEST", `invalid asset record ${name}`);
    }
    if (!/^https:\/\//i.test(asset.url) && !(options.manifestFile && /^file:\/\//i.test(asset.url))) {
      throw new InstallerError("E_URL", `release asset URL must use HTTPS: ${asset.url}`);
    }
    if (!options.manifestFile && !asset.url.startsWith(trustedAssetPrefix)) {
      throw new InstallerError("E_URL", `release asset is outside the immutable ${trustedAssetPrefix} release prefix`);
    }
    if (asset.bytes !== undefined && (!Number.isSafeInteger(asset.bytes) || asset.bytes < 0)) {
      throw new InstallerError("E_MANIFEST", `invalid byte count for ${name}`);
    }
  }
  return { manifest, source, authenticated, keyId: authenticated ? RELEASE_KEY_ID : null };
}

async function writeJsonAtomic(destination, value) {
  await mkdir(dirname(destination), { recursive: true });
  const temporary = `${destination}.${process.pid}.${Date.now()}.tmp`;
  await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, { encoding: "utf8", flag: "wx" });
  await replaceFileAtomic(temporary, destination);
}

async function replaceFileAtomic(temporary, destination) {
  const backup = `${destination}.previous-${process.pid}-${Date.now()}`;
  let moved = false;
  try {
    try {
      await rename(destination, backup);
      moved = true;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    await rename(temporary, destination);
    if (moved) await rm(backup, { force: true });
  } catch (error) {
    await rm(temporary, { force: true });
    if (moved && !existsSync(destination)) await rename(backup, destination).catch(() => {});
    throw new InstallerError("E_ATOMIC", `cannot replace ${destination}: ${error.message}`);
  }
}

async function readJson(destination, fallback = null) {
  try {
    return JSON.parse(await readFile(destination, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") return fallback;
    throw new InstallerError("E_STATE", `invalid ${destination}: ${error.message}`);
  }
}

async function copyTree(source, destination) {
  await mkdir(destination, { recursive: true });
  const entries = await readdir(source, { withFileTypes: true });
  for (const entry of entries) {
    const from = join(source, entry.name);
    const to = join(destination, entry.name);
    if (entry.isDirectory()) await copyTree(from, to);
    else if (entry.isFile()) await copyFile(from, to);
    else throw new InstallerError("E_SKILL", `unsupported skill entry ${entry.name}`);
  }
}

async function copyBinaryAtomically(source, destination, makeExecutable = false) {
  await mkdir(dirname(destination), { recursive: true });
  const temporary = `${destination}.${process.pid}.${Date.now()}.tmp`;
  await copyFile(source, temporary);
  if (makeExecutable) await chmod(temporary, 0o755);
  await replaceFileAtomic(temporary, destination);
}

async function extractPlatformTools(zipPath, stagingRoot, spec) {
  const listing = await execFileAsync(spec.archiveTool, ["-tf", zipPath], {
    windowsHide: true,
    timeout: DEFAULT_TIMEOUT_MS,
    maxBuffer: 512 * 1024,
  });
  const entries = String(listing.stdout || "").split(/\r?\n/).map((value) => value.trim()).filter(Boolean);
  if (!entries.length || entries.some((entry) => entry.startsWith("/") || entry.includes("..\\") || entry.includes("../") || /^[A-Za-z]:/.test(entry))) {
    throw new InstallerError("E_PLATFORM_TOOLS", "platform-tools archive contains an unsafe path");
  }
  if (!entries.some((entry) => entry.replaceAll("\\", "/") === `platform-tools/${spec.adbName}`)) {
    throw new InstallerError("E_PLATFORM_TOOLS", `platform-tools archive does not contain platform-tools/${spec.adbName}`);
  }
  const destination = join(stagingRoot, "platform-tools-extracted");
  await mkdir(destination, { recursive: true });
  await execFileAsync(spec.archiveTool, ["-xf", zipPath, "-C", destination], {
    windowsHide: true,
    timeout: DEFAULT_TIMEOUT_MS,
    maxBuffer: 512 * 1024,
  });
  const extracted = join(destination, "platform-tools");
  const adb = join(extracted, spec.adbName);
  if (!existsSync(adb)) throw new InstallerError("E_PLATFORM_TOOLS", "extracted platform-tools is incomplete");
  if (spec.runtimePlatform !== "win32") await chmod(adb, 0o755);
  return extracted;
}

async function replaceDirectoryAtomic(staged, destination) {
  await mkdir(dirname(destination), { recursive: true });
  const backup = `${destination}.previous-${process.pid}-${Date.now()}`;
  let moved = false;
  try {
    try {
      await rename(destination, backup);
      moved = true;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    await rename(staged, destination);
    if (moved) await rm(backup, { recursive: true, force: true });
  } catch (error) {
    if (moved && !existsSync(destination)) await rename(backup, destination).catch(() => {});
    throw new InstallerError("E_ATOMIC", `cannot replace ${destination}: ${error.message}`);
  }
}

async function hashFile(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

async function verifyVersionIntegrity(versionDir, spec) {
  const metadataPath = join(versionDir, "integrity.json");
  const metadata = await readJson(metadataPath, null);
  if (!metadata || metadata.product !== PRODUCT || typeof metadata.version !== "string") {
    throw new InstallerError("E_ROLLBACK", "version has no authenticated integrity metadata");
  }
  const executable = join(versionDir, spec.executableName);
  if (!existsSync(executable) || !/^[a-f0-9]{64}$/i.test(String(metadata.host_sha256 || ""))) {
    throw new InstallerError("E_ROLLBACK", "version integrity metadata is incomplete");
  }
  const bytes = (await stat(executable)).size;
  const digest = await hashFile(executable);
  if (digest !== metadata.host_sha256.toLowerCase() || (metadata.host_bytes !== undefined && bytes !== metadata.host_bytes)) {
    throw new InstallerError("E_ROLLBACK", "previous host asset failed integrity verification");
  }
  if (metadata.helper_sha256) {
    const helper = join(versionDir, "dev.codex.aubridge.apk");
    if (!existsSync(helper) || await hashFile(helper) !== metadata.helper_sha256.toLowerCase()) {
      throw new InstallerError("E_ROLLBACK", "previous helper asset failed integrity verification");
    }
  }
  return metadata;
}

async function hashTree(root) {
  const entries = {};
  async function visit(directory, prefix = "") {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      const name = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) await visit(path, name);
      else if (entry.isFile()) entries[name] = await hashFile(path);
      else throw new InstallerError("E_SKILL", `unsupported skill entry ${name}`);
    }
  }
  await visit(root);
  return Object.fromEntries(Object.entries(entries).sort(([a], [b]) => a.localeCompare(b)));
}

async function stageSkill(stagingRoot, destination) {
  const source = join(packageRoot, "skill");
  if (!existsSync(source)) throw new InstallerError("E_SKILL", "package does not contain the android-use skill payload");
  const staged = join(stagingRoot, "skill");
  await copyTree(source, staged);
  return { path: destination, staged, changed: true, hashes: await hashTree(source) };
}

async function updateUserPath(binDirectory) {
  if (platform() !== "win32") return updateUnixPath(binDirectory);
  let query = { stdout: "" };
  try {
    query = await execFileAsync("reg.exe", ["QUERY", "HKCU\\Environment", "/v", "Path"], { windowsHide: true });
  } catch (error) {
    const output = `${error.stdout || ""}\n${error.stderr || ""}`;
    if (!/unable to find the specified registry (?:key|value)/i.test(output)) throw error;
  }
  const line = query.stdout.split(/\r?\n/).find((value) => /\sPath\s+REG_/i.test(value));
  const existing = line ? line.replace(/^.*?REG_(?:EXPAND_)?SZ\s+/i, "").trim() : "";
  const values = existing.split(";").map((value) => value.trim()).filter(Boolean);
  if (!values.some((value) => value.toLowerCase() === binDirectory.toLowerCase())) values.push(binDirectory);
  await execFileAsync("reg.exe", ["ADD", "HKCU\\Environment", "/v", "Path", "/t", "REG_EXPAND_SZ", "/d", values.join(";"), "/f"], { windowsHide: true });
  return { type: "windows-user-path", entry: binDirectory };
}

async function updateUnixPath(binDirectory) {
  const userBin = join(homedir(), ".local", "bin");
  await mkdir(userBin, { recursive: true });
  const executable = join(binDirectory, "au");
  const link = join(userBin, "au");
  if (existsSync(link)) {
    const metadata = await lstat(link);
    if (!metadata.isSymbolicLink()) {
      throw new InstallerError("E_PATH", `${link} already exists and is not owned by android-use`);
    }
    const target = resolve(dirname(link), await readlink(link));
    const ownedRoot = resolve(binDirectory, "..");
    if (!target.startsWith(`${ownedRoot}${process.platform === "win32" ? "\\" : "/"}`)) {
      throw new InstallerError("E_PATH", `${link} points outside the android-use install root`);
    }
    await rm(link, { force: true });
  }
  await symlink(executable, link, "file");
  const shell = String(process.env.SHELL || "");
  const profile = shell.endsWith("/zsh") ? join(homedir(), ".zprofile") : join(homedir(), ".profile");
  const marker = '# android-use managed PATH';
  const current = await readFile(profile, "utf8").catch((error) => error.code === "ENOENT" ? "" : Promise.reject(error));
  if (!current.includes(marker)) {
    await appendFile(profile, `${current.endsWith("\n") || current.length === 0 ? "" : "\n"}${marker}\nexport PATH="$HOME/.local/bin:$PATH"\n`, "utf8");
  }
  return { type: "unix-link", entries: [userBin], profile, link };
}

async function removeManagedPath(root, pathState) {
  if (!pathState) return false;
  if (pathState.type === "windows-user-path") {
    const entry = resolve(String(pathState.entry || ""));
    if (!samePath(entry, join(root, "bin"))) throw new InstallerError("E_OWNERSHIP", "stored Windows PATH entry is outside the android-use install root");
    const query = await execFileAsync("reg.exe", ["QUERY", "HKCU\\Environment", "/v", "Path"], { windowsHide: true }).catch(() => ({ stdout: "" }));
    const line = String(query.stdout || "").split(/\r?\n/).find((value) => /\sPath\s+REG_/i.test(value));
    const existing = line ? line.replace(/^.*?REG_(?:EXPAND_)?SZ\s+/i, "").trim() : "";
    const values = existing.split(";").map((value) => value.trim()).filter(Boolean);
    const retained = values.filter((value) => !samePath(value, entry));
    if (retained.length !== values.length) {
      await execFileAsync("reg.exe", ["ADD", "HKCU\\Environment", "/v", "Path", "/t", "REG_EXPAND_SZ", "/d", retained.join(";"), "/f"], { windowsHide: true });
      return true;
    }
    return false;
  }
  const link = resolve(String(pathState.link || ""));
  const profile = resolve(String(pathState.profile || ""));
  if (!samePath(link, join(homedir(), ".local", "bin", "au"))) {
    throw new InstallerError("E_OWNERSHIP", "stored Unix launcher path is not the android-use managed launcher");
  }
  if (existsSync(link)) {
    const metadata = await lstat(link);
    if (!metadata.isSymbolicLink()) throw new InstallerError("E_OWNERSHIP", "managed Unix launcher was replaced by a non-symlink");
    const target = resolve(dirname(link), await readlink(link));
    if (!pathIsWithin(root, target)) throw new InstallerError("E_OWNERSHIP", "managed Unix launcher points outside android-use");
    await rm(link, { force: true });
  }
  const marker = '# android-use managed PATH';
  const block = `${marker}\nexport PATH="$HOME/.local/bin:$PATH"`;
  const current = await readFile(profile, "utf8").catch((error) => error.code === "ENOENT" ? "" : Promise.reject(error));
  const updated = current.replace(`${block}\r\n`, "").replace(`${block}\n`, "").replace(block, "");
  if (updated !== current) await writeFile(profile, updated, "utf8");
  return true;
}

function testCrashAt(options, phase) {
  if (options.manifestFile && process.env.AU_TEST_CRASH_INSTALL_PHASE === phase) {
    process.exit(97);
  }
}

async function install(options) {
  const spec = platformSpec(options);
  if (options.installHelper && !options.withHelper) {
    throw new InstallerError("E_ARGS", "--install-helper requires --with-helper");
  }
  if (options.skillOnly && options.hostOnly) throw new InstallerError("E_ARGS", "--skill-only and --host-only are mutually exclusive");
  if (options.installHelper && options.skillOnly) throw new InstallerError("E_ARGS", "--install-helper requires host installation");
  const root = installRoot(options);
  const p = paths(root);
  const release = options.skillOnly ? { manifest: null, source: null } : await readManifest(options);
  const manifest = release.manifest;
  const source = release.source;
  const host = manifest?.assets?.[spec.hostKey];
  if (!host && !options.skillOnly) throw new InstallerError("E_MANIFEST", `manifest lacks ${spec.hostKey}`);
  const helper = manifest?.assets?.helper_apk;
  if (options.withHelper && !helper) throw new InstallerError("E_MANIFEST", "manifest lacks helper_apk");
  const platformTools = manifest?.assets?.[spec.platformToolsKey]
    || manifest?.assets?.[`platform_tools_${spec.osName}_universal`]
    || (!options.manifestFile ? spec.officialPlatformTools : null);
  const skillDestination = skillRoot(options.agent || "codex", options);
  if (options.dryRun) {
    return { version: VERSION, platform: `${spec.osName}-${spec.cpuName}`, executable: join(p.bin, spec.executableName), root, manifest: source, manifest_authenticated: release.authenticated, skill: options.hostOnly ? null : skillDestination, host: !options.skillOnly, helper: Boolean(options.withHelper), platform_tools: Boolean(platformTools), dry_run: true };
  }
  await mkdir(p.staging, { recursive: true });
  const staging = join(p.staging, `${VERSION}-${process.pid}-${Date.now()}`);
  await mkdir(staging, { recursive: true });
  let transactionStarted = false;
  try {
    const versionDir = join(p.versions, VERSION);
    const versionStage = join(staging, "version");
    let stagedPlatformTools = null;
    const integrity = {
      product: PRODUCT,
      version: VERSION,
      host_sha256: null,
      host_bytes: null,
      helper_sha256: null,
      helper_signer_sha256: manifest?.helper_signer_sha256 || null,
      platform_tools_revision: null,
      platform_tools_sha256: null,
      created_at: new Date().toISOString(),
    };
    if (!options.skillOnly) await mkdir(versionStage, { recursive: true });
    if (!options.skillOnly) {
      const hostPath = join(staging, spec.executableName);
      const hostResult = await downloadToFile(host.url, hostPath, MAX_ASSET_BYTES);
      if (hostResult.sha256 !== host.sha256.toLowerCase() || (host.bytes !== undefined && hostResult.bytes !== host.bytes)) {
        throw new InstallerError("E_HASH", "host asset integrity verification failed");
      }
      integrity.host_sha256 = hostResult.sha256;
      integrity.host_bytes = hostResult.bytes;
      await copyFile(hostPath, join(versionStage, spec.executableName));
      if (spec.runtimePlatform !== "win32") await chmod(join(versionStage, spec.executableName), 0o755);
      if (options.withHelper) {
        const apkPath = join(staging, "dev.codex.aubridge.apk");
        const apkResult = await downloadToFile(helper.url, apkPath, MAX_ASSET_BYTES);
        if (apkResult.sha256 !== helper.sha256.toLowerCase() || (helper.bytes !== undefined && apkResult.bytes !== helper.bytes)) {
          throw new InstallerError("E_HASH", "helper APK integrity verification failed");
        }
        integrity.helper_sha256 = apkResult.sha256;
        await copyFile(apkPath, join(versionStage, "dev.codex.aubridge.apk"));
      }
      if (platformTools) {
        const archive = join(staging, "platform-tools.zip");
        const result = await downloadToFile(platformTools.url, archive, MAX_ASSET_BYTES);
        if (result.sha256 !== platformTools.sha256.toLowerCase() || (platformTools.bytes !== undefined && result.bytes !== platformTools.bytes)) {
          throw new InstallerError("E_HASH", "platform-tools asset integrity verification failed");
        }
        stagedPlatformTools = await extractPlatformTools(archive, staging, spec);
        integrity.platform_tools_revision = platformTools.revision || null;
        integrity.platform_tools_sha256 = result.sha256;
      }
      await writeFile(join(versionStage, "integrity.json"), `${JSON.stringify(integrity, null, 2)}\n`, { encoding: "utf8", flag: "wx" });
    }
    const skill = options.hostOnly ? null : await stageSkill(staging, skillDestination);
    const previous = await readJson(p.current, null);
    await beginInstallTransaction(root, p, staging, spec, versionDir, skillDestination, options);
    transactionStarted = true;
    if (!options.skillOnly) {
      await replaceDirectoryAtomic(versionStage, versionDir);
      testCrashAt(options, "after-version");
      if (existsSync(join(p.bin, spec.executableName))) {
        await invokeHost(join(p.bin, spec.executableName), ["x", "stop", "-j"], options).catch(() => null);
      }
      await copyBinaryAtomically(join(versionDir, spec.executableName), join(p.bin, spec.executableName), spec.runtimePlatform !== "win32");
      testCrashAt(options, "after-host");
      if (stagedPlatformTools) {
        await replaceDirectoryAtomic(stagedPlatformTools, p.platformTools);
        testCrashAt(options, "after-platform-tools");
      }
    }
    if (skill) {
      await mkdir(dirname(skillDestination), { recursive: true });
      await replaceDirectoryAtomic(skill.staged, skillDestination);
      testCrashAt(options, "after-skill");
    }
    await writeJsonAtomic(p.history, { previous, installed_at: new Date().toISOString(), version: VERSION });
    testCrashAt(options, "after-history");
    const current = {
      ...(previous || {}),
      product: PRODUCT,
      version: VERSION,
      host: !options.skillOnly || Boolean(previous?.host),
      helper: Boolean(options.withHelper) || Boolean(previous?.helper),
      platform_tools: Boolean(platformTools) || Boolean(previous?.platform_tools),
      host_sha256: integrity.host_sha256 || previous?.host_sha256 || null,
      host_bytes: integrity.host_bytes || previous?.host_bytes || null,
      helper_sha256: integrity.helper_sha256 || previous?.helper_sha256 || null,
      helper_signer_sha256: integrity.helper_signer_sha256 || previous?.helper_signer_sha256 || null,
      platform_tools_revision: integrity.platform_tools_revision || previous?.platform_tools_revision || null,
      platform_tools_sha256: integrity.platform_tools_sha256 || previous?.platform_tools_sha256 || null,
      skill: skill?.path || previous?.skill || null,
      skill_hashes: skill?.hashes || previous?.skill_hashes || null,
      manifest_authenticated: release.authenticated || false,
      manifest_key_id: release.keyId || null,
    };
    await writeJsonAtomic(p.current, current);
    testCrashAt(options, "after-current");
    if (manifest) await writeJsonAtomic(p.manifest, manifest);
    testCrashAt(options, "after-manifest");
    await rm(p.transaction, { force: true });
    transactionStarted = false;
    let pathUpdated = false;
    let pathState = current.path_state || null;
    if (options.addPath && !options.noPath && !options.skillOnly) {
      pathState = await updateUserPath(p.bin);
      await writeJsonAtomic(p.current, { ...current, path_state: pathState });
      pathUpdated = true;
    }
    let helperInstalled = false;
    if (options.installHelper) {
      const executable = join(p.bin, spec.executableName);
      const apk = join(versionDir, "dev.codex.aubridge.apk");
      await execFileAsync(executable, ["app", "install", apk], { windowsHide: true, timeout: DEFAULT_TIMEOUT_MS, maxBuffer: 256 * 1024 });
      helperInstalled = true;
    }
    return { version: VERSION, platform: `${spec.osName}-${spec.cpuName}`, root, executable: options.skillOnly ? null : join(p.bin, spec.executableName), skill: skill?.path || null, helper: options.withHelper ? join(versionDir, "dev.codex.aubridge.apk") : null, helper_installed: helperInstalled, path_updated: pathUpdated };
  } catch (error) {
    if (transactionStarted) await recoverInstallTransaction(root);
    throw error;
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

async function setup(options) {
  if (options.agent) options = { ...options, agent: resolveAgent(options.agent) };
  const root = installRoot(options);
  const executable = executablePath(options);
  const helperPath = join(root, "versions", VERSION, "dev.codex.aubridge.apk");
  let installed = null;
  if (!existsSync(executable) || !existsSync(helperPath)) {
    installed = await install({
      ...options,
      withHelper: true,
      installHelper: false,
      hostOnly: false,
      skillOnly: false,
      addPath: !options.noPath,
    });
  }

  if (options.dryRun) {
    return {
      ...(installed || {}),
      version: VERSION,
      root,
      executable,
      helper: helperPath,
      install_required: Boolean(installed),
      path_update: !options.noPath,
      helper_install_requested: options.installHelper !== false,
      dry_run: true,
    };
  }

  let pathUpdated = Boolean(installed?.path_updated);
  if (!options.noPath && !pathUpdated) {
    await updateUserPath(dirname(executable));
    pathUpdated = true;
  }

  const setupArgs = () => [
    ...(options.repair ? ["--repair"] : []),
    ...(options.wait ? ["--wait"] : []),
  ];
  let first = await hostCommand(options, "setup", setupArgs());
  let helperInstalled = false;
  const helperRequested = options.installHelper !== false;
  if (helperRequested && existsSync(helperPath)) {
    let status = null;
    try { status = await hostCommand(options, "st"); } catch { /* setup will return the waiting state */ }
    const statusData = status?.data?.data || status?.data;
    const connected = statusData?.state === "device";
    if (connected) {
      const result = await invokeHost(first.executable, ["app", "install", helperPath], options);
      if (!result.ok) throw new InstallerError("E_HELPER", "helper installation failed", { result: result.data });
      helperInstalled = true;
      first = await hostCommand(options, "setup", setupArgs());
    }
  }
  if (options.agent) {
    const configured = await invokeHost(first.executable, ["agent", "configure", options.agent, "-j"], options);
    if (!configured.ok) throw new InstallerError("E_AGENT", "agent adapter configuration failed", { result: configured.data });
  }
  const ready = await hostCommand(options, "ready");
  return {
    ...(installed || {}),
    setup: first.data,
    ready: ready.data,
    helper_installed: helperInstalled,
    path_updated: pathUpdated,
    resumed: Boolean(installed),
    root,
  };
}

async function doctor(options) {
  const root = installRoot(options);
  const p = paths(root);
  const current = await readJson(p.current, null);
  const spec = platformSpec(options);
  const executable = join(p.bin, spec.executableName);
  const platformToolsExecutable = join(p.platformTools, spec.adbName);
  const skill = skillRoot(options.agent || "codex", options);
  const skillHashes = current?.skill_hashes;
  let skillDrift = null;
  if (current?.skill && skillHashes && existsSync(skill)) {
    const actual = await hashTree(skill);
    skillDrift = JSON.stringify(actual) !== JSON.stringify(skillHashes);
  }
  let hostReady = null;
  if (existsSync(executable)) {
    try {
      hostReady = (await hostCommand(options, "ready")).data;
    } catch (error) {
      hostReady = { ready: false, code: error.code || "E_SETUP", message: error.message };
    }
  }
  const result = { root, current, executable, executable_exists: existsSync(executable), platform_tools: p.platformTools, platform_tools_exists: existsSync(platformToolsExecutable), host_ready: hostReady, skill, skill_exists: existsSync(skill), skill_drift: skillDrift };
  return result;
}

async function rollback(options) {
  const root = installRoot(options);
  const p = paths(root);
  const history = await readJson(p.history, null);
  const previous = history?.previous?.version;
  const spec = platformSpec(options);
  if (!previous || !existsSync(join(p.versions, previous, spec.executableName))) throw new InstallerError("E_ROLLBACK", "no verified previous version is available");
  const integrity = await verifyVersionIntegrity(join(p.versions, previous), spec);
  if (!options.dryRun) {
    const current = await readJson(p.current, {});
    let helperRolledBack = false;
    if (current.helper) {
      if (!integrity.helper_sha256) {
        throw new InstallerError("E_ROLLBACK", "previous version has no verified helper APK; refusing a split host/helper rollback");
      }
      const activeHost = join(p.bin, spec.executableName);
      const previousHelper = join(p.versions, previous, "dev.codex.aubridge.apk");
      const installed = await invokeHost(activeHost, ["app", "install", previousHelper], options);
      if (!installed.ok) {
        throw new InstallerError("E_ROLLBACK", "previous helper APK could not be restored; host rollback was not activated", { result: installed.data });
      }
      helperRolledBack = true;
    }
    await copyBinaryAtomically(join(p.versions, previous, spec.executableName), join(p.bin, spec.executableName), spec.runtimePlatform !== "win32");
    await writeJsonAtomic(p.current, {
      ...current,
      product: PRODUCT,
      version: previous,
      host: true,
      host_sha256: integrity.host_sha256,
      host_bytes: integrity.host_bytes,
      helper_sha256: integrity.helper_sha256 || null,
      helper_rolled_back: helperRolledBack,
      rollback: true,
    });
  }
  return { version: previous, root, dry_run: Boolean(options.dryRun) };
}

async function uninstall(options) {
  const root = installRoot(options);
  const p = paths(root);
  if (options.purge && !options.yes) throw new InstallerError("E_CONFIRM", "--purge is destructive; repeat with --yes");
  if (!options.dryRun) {
    const spec = platformSpec(options);
    const current = await readJson(p.current, null);
    const ownedSkill = current?.skill ? await assertOwnedSkillPath(current.skill, options) : null;
    const executable = join(p.bin, spec.executableName);
    if (existsSync(executable)) await invokeHost(executable, ["x", "stop", "-j"], options).catch(() => null);
    const pathRemoved = await removeManagedPath(root, current?.path_state);
    await rm(executable, { force: true });
    await rm(p.staging, { recursive: true, force: true });
    let skill = "preserved";
    if (ownedSkill && existsSync(ownedSkill)) {
      const expected = current.skill_hashes;
      const actual = expected ? await hashTree(ownedSkill) : null;
      if (options.purge || (expected && JSON.stringify(actual) === JSON.stringify(expected))) {
        await rm(ownedSkill, { recursive: true, force: true });
        skill = "removed";
      }
    }
    await rm(p.current, { force: true });
    if (options.purge) {
      await rm(p.versions, { recursive: true, force: true });
      await rm(p.platformTools, { recursive: true, force: true });
      await rm(p.history, { force: true });
      await rm(p.manifest, { force: true });
      await rm(p.transaction, { force: true });
      await rm(p.bin, { recursive: true, force: true });
      await rm(join(root, "agents"), { recursive: true, force: true });
    }
    return { root, purged: Boolean(options.purge), skill, path_removed: pathRemoved, preserved: options.purge ? ["config.json", "state", "artifacts"] : ["versions", "platform-tools", "history.json", "release-manifest.json", "config.json", "state", "artifacts"] };
  }
  return { root, purged: Boolean(options.purge), skill: "dry-run", preserved: ["config.json", "state", "artifacts"] };
}

function help() {
  return "android-use installer: install|update|setup|ready|doctor|rollback|uninstall|print-path|version; use --json, --dry-run, --with-helper, --install-helper, --skill-only, --host-only, --agent, --repair, --add-path, --no-path, or --purge --yes";
}

async function main() {
  const parsed = parseArgs(process.argv.slice(2));
  const { command, options } = parsed;
  if (command === "version" || options.version) return output(true, { version: VERSION }, options);
  if (options.help || command === "help") {
    process.stdout.write(`${help()}\n`);
    return;
  }
  const dispatch = async () => {
    let result;
    if (command === "install" || command === "update") result = await install(options);
    else if (command === "setup") result = await setup(options);
    else if (command === "ready") result = (await hostCommand(options, "ready")).data;
    else if (command === "doctor") result = await doctor(options);
    else if (command === "rollback") result = await rollback(options);
    else if (command === "uninstall") result = await uninstall(options);
    else if (command === "print-path") result = { path: executablePath(options) };
    else throw new InstallerError("E_ARGS", `unknown command ${command}`);
    return result;
  };
  const mutating = ["install", "update", "setup", "rollback", "uninstall"].includes(command) && !options.dryRun;
  let result;
  if (mutating) {
    const root = installRoot(options);
    result = await withInstallLock(root, async () => {
      await recoverInstallTransaction(root);
      await recoverAtomicBackups(root);
      return dispatch();
    });
  } else {
    result = await dispatch();
  }
  output(true, result, options);
}

try {
  await main();
} catch (error) {
  let options = { json: false };
  try { options = parseArgs(process.argv.slice(2)).options; } catch { /* preserve a compact error for malformed arguments */ }
  output(false, fail(error), options);
  process.exitCode = 2;
}
