#!/usr/bin/env node
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const version = "1.1.0";
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

// npm installs ship the binary in an optional platform package, e.g. @android-use/darwin-arm64.
const optionalDir = (() => {
  try {
    return dirname(createRequire(import.meta.url).resolve(`@android-use/${platform}/package.json`));
  } catch {
    return null;
  }
})();
const candidates = [
  process.env.AU_BIN,
  join(here, "bin", platform, name),
  optionalDir && join(optionalDir, "bin", name),
  join(here, name),
].filter(Boolean);
const binary = candidates.find(existsSync);
if (!binary) fail(`The ${platform} au binary is missing from this package. Install the platform package or set AU_BIN.`);

// Check the binary and helper APK against the manifest of the package they came from.
const pkgDir = optionalDir && binary.startsWith(optionalDir + sep) ? optionalDir : here;
const manifest = join(pkgDir, "manifest.json");
if (existsSync(manifest)) {
  try {
    const files = JSON.parse(readFileSync(manifest, "utf8")).files ?? {};
    const verify = (file) => {
      const key = relative(pkgDir, file).replaceAll("\\", "/");
      if (!files[key]) return;
      if (!existsSync(file)) fail("The Android Use package is incomplete. Reinstall it from a trusted source.");
      const actual = createHash("sha256").update(readFileSync(file)).digest("hex");
      if (actual !== files[key]) fail("The Android Use package failed its integrity check. Reinstall it from a trusted source.");
    };
    verify(binary);
    verify(join(dirname(binary), "aubridge.apk"));
  } catch {
    fail("The Android Use package manifest could not be verified.");
  }
}

// stdout is the MCP channel for "serve", so nothing above writes to stdout on that path.
const child = spawn(binary, [command, ...process.argv.slice(3)], { stdio: "inherit", windowsHide: true });
child.on("error", (e) => fail(e.message));
child.on("exit", (code, signal) => process.exit(signal ? 1 : code ?? 1));
