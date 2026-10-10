#!/usr/bin/env node
// Stage a publishable @androiduse/<key> platform package.
// Usage: node install/stage-platform.mjs <key> <path-to-au-binary> <out-dir>
import { createHash } from "node:crypto";
import { chmod, copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const OS = ["darwin", "linux", "win32"];
const CPU = ["arm64", "x64"];

const [key, source, outDir] = process.argv.slice(2);
const [os, cpu] = (key ?? "").split("-");
if (!OS.includes(os) || !CPU.includes(cpu) || !source || !outDir) {
  console.error("Usage: node install/stage-platform.mjs <darwin|linux|win32>-<arm64|x64> <path-to-au-binary> <out-dir>");
  process.exit(2);
}

const install = JSON.parse(await readFile(join(here, "package.json"), "utf8"));
const version = (await readFile(join(here, "..", "VERSION"), "utf8")).trim();
const file = `bin/${os === "win32" ? "au.exe" : "au"}`;
const target = join(outDir, file);

await mkdir(join(outDir, "bin"), { recursive: true });
await copyFile(source, target);
await chmod(target, 0o755);
const sha256 = createHash("sha256").update(await readFile(target)).digest("hex");

const pkg = {
  name: `@androiduse/${key}`,
  version,
  description: `Android Use ${os} ${cpu} binary`,
  os: [os],
  cpu: [cpu],
  files: ["bin"],
  license: "MIT",
  repository: install.repository,
  publishConfig: { access: "public" },
};
await writeFile(join(outDir, "package.json"), `${JSON.stringify(pkg, null, 2)}\n`);
await writeFile(join(outDir, "manifest.json"), `${JSON.stringify({ files: { [file]: sha256 } }, null, 2)}\n`);
