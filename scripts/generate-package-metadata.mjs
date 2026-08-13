import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const values = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  const value = process.argv[index + 1];
  if (!key?.startsWith("--") || !value) throw new Error(`invalid argument ${key || ""}`);
  values.set(key.slice(2), value);
}

const version = values.get("version");
const assets = resolve(values.get("assets") || "release/assets");
const outputDirectory = resolve(values.get("output-directory") || join(assets, "winget"));
const repository = values.get("repository") || process.env.GITHUB_REPOSITORY || "austinintelligence/android-use";
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version || "")) throw new Error("--version must be strict semver");
if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) throw new Error("--repository must be owner/name");

const msiX64 = join(assets, `android-use-${version}-windows-x64.msi`);
const msiArm64 = join(assets, `android-use-${version}-windows-arm64.msi`);
for (const path of [msiX64, msiArm64]) {
  if (!(await stat(path).catch(() => null))?.isFile()) throw new Error(`required MSI is missing: ${path}`);
}

const replacements = new Map([
  ["@VERSION@", version],
  ["@REPOSITORY@", repository],
  ["@WINDOWS_X64_MSI_SHA256@", await sha256(msiX64)],
  ["@WINDOWS_ARM64_MSI_SHA256@", await sha256(msiArm64)],
]);
const names = [
  "AndroidUse.AndroidUse.yaml",
  "AndroidUse.AndroidUse.locale.en-US.yaml",
  "AndroidUse.AndroidUse.installer.yaml",
];
await mkdir(outputDirectory, { recursive: true });
for (const name of names) {
  let rendered = await readFile(resolve("packaging", "winget", `${name}.in`), "utf8");
  for (const [token, value] of replacements) rendered = rendered.replaceAll(token, value);
  if (/@[A-Z0-9_]+@/.test(rendered)) throw new Error(`${name} contains an unresolved placeholder`);
  await writeFile(join(outputDirectory, name), rendered, "utf8");
}
process.stdout.write(`${JSON.stringify({ output_directory: outputDirectory, files: names, version, repository })}\n`);

async function sha256(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex").toUpperCase();
}
