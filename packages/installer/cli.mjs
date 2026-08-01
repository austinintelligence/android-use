#!/usr/bin/env node

import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { createReadStream, existsSync } from "node:fs";
import { mkdir, open, readFile, readdir, rename, rm, stat, writeFile, copyFile } from "node:fs/promises";
import { homedir, platform, arch } from "node:os";
import { dirname, basename, join, resolve, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const packageRoot = dirname(fileURLToPath(import.meta.url));
const packageJson = JSON.parse(await readFile(join(packageRoot, "package.json"), "utf8"));
const PRODUCT = "android-use";
const VERSION = packageJson.version;
const REPOSITORY = process.env.AU_REPOSITORY || "drperky20/android-use";
const DEFAULT_TIMEOUT_MS = 30_000;
const MAX_MANIFEST_BYTES = 256 * 1024;
const MAX_ASSET_BYTES = 100 * 1024 * 1024;

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
    else if (token === "--version") options.version = true;
    else if (token === "--help" || token === "-h") options.help = true;
    else if (token === "--") positional.push(...argv.slice(index));
    else if (token.startsWith("-")) throw new InstallerError("E_ARGS", `unknown option ${token}`);
    else positional.push(token);
  }
  return { command, options, positional };
}

function installRoot(options) {
  const local = process.env.LOCALAPPDATA || join(homedir(), "AppData", "Local");
  return resolve(options.installRoot || process.env.AU_INSTALL_ROOT || join(local, "Codex", PRODUCT));
}

function paths(root) {
  return {
    root,
    versions: join(root, "versions"),
    staging: join(root, "staging"),
    bin: join(root, "bin"),
    current: join(root, "current.json"),
    history: join(root, "history.json"),
    manifest: join(root, "release-manifest.json"),
  };
}

function skillRoot(agent) {
  if (agent && agent !== "codex") {
    throw new InstallerError("E_AGENT", `unsupported agent ${agent}; this release targets codex`);
  }
  const home = process.env.CODEX_HOME || join(homedir(), ".codex");
  return resolve(join(home, "skills", PRODUCT));
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
  if (!/^https:\/\//i.test(source) && !options.manifestFile) {
    throw new InstallerError("E_URL", "release manifest must use HTTPS unless --manifest names a local fixture");
  }
  const raw = await readBytes(source, MAX_MANIFEST_BYTES, Boolean(options.manifestFile));
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
  if (!manifest.assets || typeof manifest.assets !== "object") throw new InstallerError("E_MANIFEST", "manifest has no assets");
  for (const [name, asset] of Object.entries(manifest.assets)) {
    if (!asset || typeof asset.url !== "string" || !/^[a-f0-9]{64}$/i.test(asset.sha256)) {
      throw new InstallerError("E_MANIFEST", `invalid asset record ${name}`);
    }
    if (!/^https:\/\//i.test(asset.url) && !(options.manifestFile && /^file:\/\//i.test(asset.url))) {
      throw new InstallerError("E_URL", `release asset URL must use HTTPS: ${asset.url}`);
    }
    if (asset.bytes !== undefined && (!Number.isSafeInteger(asset.bytes) || asset.bytes < 0)) {
      throw new InstallerError("E_MANIFEST", `invalid byte count for ${name}`);
    }
  }
  return { manifest, source };
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

async function copyBinaryAtomically(source, destination) {
  await mkdir(dirname(destination), { recursive: true });
  const temporary = `${destination}.${process.pid}.${Date.now()}.tmp`;
  await copyFile(source, temporary);
  await replaceFileAtomic(temporary, destination);
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

async function installSkill(destination, options) {
  const source = join(packageRoot, "skill");
  if (!existsSync(source)) throw new InstallerError("E_SKILL", "package does not contain the android-use skill payload");
  if (options.dryRun) return { path: destination, changed: false };
  await mkdir(dirname(destination), { recursive: true });
  const backup = `${destination}.backup-${Date.now()}`;
  if (existsSync(destination)) await rename(destination, backup);
  try {
    await copyTree(source, destination);
  } catch (error) {
    await rm(destination, { recursive: true, force: true });
    if (existsSync(backup)) await rename(backup, destination);
    throw error;
  }
  return { path: destination, backup: existsSync(backup) ? backup : null, changed: true, hashes: await hashTree(source) };
}

async function updateUserPath(binDirectory) {
  if (platform() !== "win32") throw new InstallerError("E_PLATFORM", "--add-path is supported only on Windows");
  const query = await execFileAsync("reg.exe", ["QUERY", "HKCU\\Environment", "/v", "Path"], { windowsHide: true });
  const line = query.stdout.split(/\r?\n/).find((value) => /\sPath\s+REG_/i.test(value));
  const existing = line ? line.replace(/^.*?REG_(?:EXPAND_)?SZ\s+/i, "").trim() : "";
  const values = existing.split(";").map((value) => value.trim()).filter(Boolean);
  if (!values.some((value) => value.toLowerCase() === binDirectory.toLowerCase())) values.push(binDirectory);
  await execFileAsync("reg.exe", ["ADD", "HKCU\\Environment", "/v", "Path", "/t", "REG_EXPAND_SZ", "/d", values.join(";"), "/f"], { windowsHide: true });
  return values;
}

async function install(options) {
  if (platform() !== (process.env.AU_PLATFORM || "win32") || arch() !== (process.env.AU_ARCH || "x64")) {
    throw new InstallerError("E_PLATFORM", "android-use currently supports Windows x64 only");
  }
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
  const host = manifest?.assets?.host_windows_x64;
  if (!host && !options.skillOnly) throw new InstallerError("E_MANIFEST", "manifest lacks host_windows_x64");
  const helper = manifest?.assets?.helper_apk;
  if (options.withHelper && !helper) throw new InstallerError("E_MANIFEST", "manifest lacks helper_apk");
  const skillDestination = skillRoot(options.agent || "codex");
  if (options.dryRun) {
    return { version: VERSION, root, manifest: source, skill: !options.hostOnly, host: !options.skillOnly, helper: Boolean(options.withHelper), dry_run: true };
  }
  await mkdir(p.staging, { recursive: true });
  const staging = join(p.staging, `${VERSION}-${process.pid}-${Date.now()}`);
  await mkdir(staging, { recursive: true });
  try {
    const versionDir = join(p.versions, VERSION);
    const versionStage = join(staging, "version");
    if (!options.skillOnly) await mkdir(versionStage, { recursive: true });
    if (!options.skillOnly) {
      const hostPath = join(staging, "au.exe");
      const hostResult = await downloadToFile(host.url, hostPath, MAX_ASSET_BYTES);
      if (hostResult.sha256 !== host.sha256.toLowerCase() || (host.bytes !== undefined && hostResult.bytes !== host.bytes)) {
        throw new InstallerError("E_HASH", "host asset integrity verification failed");
      }
      await copyFile(hostPath, join(versionStage, "au.exe"));
      if (options.withHelper) {
        const apkPath = join(staging, "dev.codex.aubridge.apk");
        const apkResult = await downloadToFile(helper.url, apkPath, MAX_ASSET_BYTES);
        if (apkResult.sha256 !== helper.sha256.toLowerCase() || (helper.bytes !== undefined && apkResult.bytes !== helper.bytes)) {
          throw new InstallerError("E_HASH", "helper APK integrity verification failed");
        }
        await copyFile(apkPath, join(versionStage, "dev.codex.aubridge.apk"));
      }
      await replaceDirectoryAtomic(versionStage, versionDir);
      await copyBinaryAtomically(join(versionDir, "au.exe"), join(p.bin, "au.exe"));
    }
    let skill = null;
    if (!options.hostOnly) skill = await installSkill(skillDestination, options);
    const previous = await readJson(p.current, null);
    await writeJsonAtomic(p.history, { previous, installed_at: new Date().toISOString(), version: VERSION });
    const current = {
      ...(previous || {}),
      product: PRODUCT,
      version: VERSION,
      host: !options.skillOnly || Boolean(previous?.host),
      helper: Boolean(options.withHelper) || Boolean(previous?.helper),
      skill: skill?.path || previous?.skill || null,
      skill_hashes: skill?.hashes || previous?.skill_hashes || null,
    };
    await writeJsonAtomic(p.current, current);
    if (manifest) await writeJsonAtomic(p.manifest, manifest);
    let pathUpdated = false;
    if (options.addPath && !options.noPath && !options.skillOnly) {
      await updateUserPath(p.bin);
      pathUpdated = true;
    }
    let helperInstalled = false;
    if (options.installHelper) {
      const executable = join(p.bin, "au.exe");
      const apk = join(versionDir, "dev.codex.aubridge.apk");
      await execFileAsync(executable, ["app", "install", apk], { windowsHide: true, timeout: DEFAULT_TIMEOUT_MS, maxBuffer: 256 * 1024 });
      helperInstalled = true;
    }
    return { version: VERSION, root, executable: options.skillOnly ? null : join(p.bin, "au.exe"), skill: skill?.path || null, helper: options.withHelper ? join(versionDir, "dev.codex.aubridge.apk") : null, helper_installed: helperInstalled, path_updated: pathUpdated };
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

async function doctor(options) {
  const root = installRoot(options);
  const p = paths(root);
  const current = await readJson(p.current, null);
  const executable = join(p.bin, "au.exe");
  const skill = skillRoot(options.agent || "codex");
  const skillHashes = current?.skill_hashes;
  let skillDrift = null;
  if (current?.skill && skillHashes && existsSync(skill)) {
    const actual = await hashTree(skill);
    skillDrift = JSON.stringify(actual) !== JSON.stringify(skillHashes);
  }
  const result = { root, current, executable, executable_exists: existsSync(executable), skill, skill_exists: existsSync(skill), skill_drift: skillDrift };
  return result;
}

async function rollback(options) {
  const root = installRoot(options);
  const p = paths(root);
  const history = await readJson(p.history, null);
  const previous = history?.previous?.version;
  if (!previous || !existsSync(join(p.versions, previous, "au.exe"))) throw new InstallerError("E_ROLLBACK", "no verified previous version is available");
  if (!options.dryRun) {
    await copyBinaryAtomically(join(p.versions, previous, "au.exe"), join(p.bin, "au.exe"));
    const current = await readJson(p.current, {});
    await writeJsonAtomic(p.current, { ...current, product: PRODUCT, version: previous, host: true, rollback: true });
  }
  return { version: previous, root, dry_run: Boolean(options.dryRun) };
}

async function uninstall(options) {
  const root = installRoot(options);
  const p = paths(root);
  if (options.purge && !options.yes) throw new InstallerError("E_CONFIRM", "--purge is destructive; repeat with --yes");
  if (!options.dryRun) {
    await rm(join(p.bin, "au.exe"), { force: true });
    await rm(p.staging, { recursive: true, force: true });
    const current = await readJson(p.current, null);
    let skill = "preserved";
    if (current?.skill && existsSync(current.skill)) {
      const expected = current.skill_hashes;
      const actual = expected ? await hashTree(current.skill) : null;
      if (options.purge || (expected && JSON.stringify(actual) === JSON.stringify(expected))) {
        await rm(current.skill, { recursive: true, force: true });
        skill = "removed";
      }
    }
    await rm(p.current, { force: true });
    if (options.purge) await rm(p.versions, { recursive: true, force: true });
    return { root, purged: Boolean(options.purge), skill, preserved: ["config.json", "state", "artifacts"] };
  }
  return { root, purged: Boolean(options.purge), skill: "dry-run", preserved: ["config.json", "state", "artifacts"] };
}

function help() {
  return "android-use installer: install|update|doctor|rollback|uninstall|print-path|version; use --json, --dry-run, --with-helper, --install-helper, --skill-only, --host-only, --add-path, --no-path, or --purge --yes";
}

async function main() {
  const parsed = parseArgs(process.argv.slice(2));
  const { command, options } = parsed;
  if (command === "version" || options.version) return output(true, { version: VERSION }, options);
  if (options.help || command === "help") {
    process.stdout.write(`${help()}\n`);
    return;
  }
  let result;
  if (command === "install" || command === "update") result = await install(options);
  else if (command === "doctor") result = await doctor(options);
  else if (command === "rollback") result = await rollback(options);
  else if (command === "uninstall") result = await uninstall(options);
  else if (command === "print-path") result = { path: join(installRoot(options), "bin", "au.exe") };
  else throw new InstallerError("E_ARGS", `unknown command ${command}`);
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
