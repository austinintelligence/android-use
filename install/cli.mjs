#!/usr/bin/env node
import { existsSync } from "node:fs";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { constants } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { readManifest, sha256 } from "./manifest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const version = "1.3.0";
const commands = ["setup", "repair", "doctor", "status", "update", "uninstall", "install", "serve", "read", "act", "devices", "enroll", "version"];
const command = process.argv[2] || "setup";
const help = `Android Use

  npx android-use <command> [args]   Run an au command: ${commands.join(", ")}
  npx android-use serve --mcp        Start the local MCP server (stdout carries MCP)
  npx android-use --version          Print the package version

The platform binary owns device behavior. Use --json for machine output.`;

if (command === "--version" || command === "version") {
  console.log(version);
  process.exit(0);
}
if (command === "--help" || command === "help") {
  console.log(help);
  process.exit(0);
}
if (!commands.includes(command)) {
  console.error(`Expected one of: ${commands.join(", ")}.`);
  process.exit(2);
}

const platform = `${process.platform}-${process.arch}`;
const name = process.platform === "win32" ? "au.exe" : "au";
const fail = (message) => {
  console.error(message);
  process.exit(1);
};

// npm installs ship the binary in an optional platform package, e.g. @androiduse/darwin-arm64.
const optionalDir = (() => {
  try {
    return dirname(createRequire(import.meta.url).resolve(`@androiduse/${platform}/package.json`));
  } catch {
    return null;
  }
})();
// AU_BIN is an explicit override and skips verification; every other binary must match its package manifest.
let binary = process.env.AU_BIN;
let pkgDir = here;
if (binary) {
  console.error(`warning: AU_BIN is set, so ${binary} runs without an integrity check.`);
} else {
  const local = join(here, "bin", platform, name);
  const inPlatformPackage = optionalDir && join(optionalDir, "bin", name);
  if (existsSync(local)) binary = local;
  else if (inPlatformPackage && existsSync(inPlatformPackage)) [binary, pkgDir] = [inPlatformPackage, optionalDir];
  else fail(`The ${platform} au binary is missing from this package. Install the platform package or set AU_BIN.`);

  let files;
  try {
    files = readManifest(pkgDir);
  } catch (e) {
    fail(`Cannot verify the Android Use package: ${e.message}. Reinstall it from a trusted source.`);
  }
  // The binary must be listed; the helper APK is checked when it is listed.
  const verify = (file, required) => {
    const key = relative(pkgDir, file).replaceAll("\\", "/");
    if (!files[key]) return required && fail(`Cannot verify the Android Use package: manifest.json has no entry for ${platform} (${key}). Reinstall it from a trusted source.`);
    if (!existsSync(file)) fail("The Android Use package is incomplete. Reinstall it from a trusted source.");
    if (sha256(file) !== files[key]) fail("The Android Use package failed its integrity check. Reinstall it from a trusted source.");
  };
  verify(binary, true);
  verify(join(dirname(binary), "aubridge.apk"), false);
}

// stdout is the MCP channel for "serve", so nothing above writes to stdout on that path.
// Pass termination signals to au, then exit the way au did. The handlers exist before au does, so a
// signal that lands while it starts is held and sent once it is running instead of orphaning it.
let child;
let early;
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.on(signal, () => (child ? child.kill(signal) : (early ??= signal)));
child = spawn(binary, [command, ...process.argv.slice(3)], { stdio: "inherit", windowsHide: true });
if (early) child.kill(early);
child.on("error", (e) => fail(e.message));
child.on("exit", (code, signal) => process.exit(signal ? 128 + (constants.signals[signal] ?? 1) : code ?? 1));
