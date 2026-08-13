import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { gunzipSync, inflateRawSync } from "node:zlib";

const run = promisify(execFile);
const root = resolve(import.meta.dirname, "..");
const temporary = await mkdtemp(join(tmpdir(), "android-use-package-test-"));
try {
  const inputs = join(temporary, "inputs");
  const first = join(temporary, "first");
  const second = join(temporary, "second");
  await mkdir(inputs, { recursive: true });
  for (const name of [
    "au-windows-x64.exe", "au-windows-arm64.exe", "au-macos-x64",
    "au-macos-arm64", "au-linux-x64", "au-linux-arm64",
  ]) await writeFile(join(inputs, name), `fixture:${name}\n`);
  await writeFile(join(inputs, "dev.codex.aubridge.apk"), "fixture:helper\n");

  const argumentsFor = (output) => [
    join(root, "scripts", "package-release.mjs"),
    "--version", "1.0.0",
    "--assets", inputs,
    "--output", output,
    "--repository", root,
    "--github-repository", "austinintelligence/android-use",
    "--source-date-epoch", "1785629325",
    "--formats", "portable",
  ];
  await run(process.execPath, argumentsFor(first));
  await run(process.execPath, argumentsFor(second));

  const names = (await readdir(first)).sort();
  const peerNames = (await readdir(second)).sort();
  if (JSON.stringify(names) !== JSON.stringify(peerNames) || names.length !== 7) {
    throw new Error(`unexpected portable package set: ${names.join(",")}`);
  }
  for (const name of names) {
    const left = await readFile(join(first, name));
    const right = await readFile(join(second, name));
    if (sha256(left) !== sha256(right)) throw new Error(`non-reproducible package: ${name}`);
  }

  const zipEntries = readZip(await readFile(join(first, "android-use-1.0.0-windows-x64.zip")));
  const tarEntries = readTar(gunzipSync(await readFile(join(first, "android-use-1.0.0-linux-x64.tar.gz"))));
  for (const entries of [zipEntries, tarEntries]) {
    if (entries.length !== 5 || !entries.some((entry) => /\/bin\/au(?:\.exe)?$/.test(entry))) {
      throw new Error(`unexpected archive entries: ${entries.join(",")}`);
    }
  }
  const formula = await readFile(join(first, "android-use.rb"), "utf8");
  if (formula.includes("@") || !formula.includes("austinintelligence/android-use")) {
    throw new Error("Homebrew formula was not fully rendered");
  }
  process.stdout.write(`${JSON.stringify({ packages: names.length, reproducible: true, zip_entries: zipEntries.length, tar_entries: tarEntries.length })}\n`);
} finally {
  await rm(temporary, { recursive: true, force: true });
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function readZip(bytes) {
  const entries = [];
  let offset = 0;
  while (offset + 4 <= bytes.length && bytes.readUInt32LE(offset) === 0x04034b50) {
    const method = bytes.readUInt16LE(offset + 8);
    const compressedLength = bytes.readUInt32LE(offset + 18);
    const uncompressedLength = bytes.readUInt32LE(offset + 22);
    const nameLength = bytes.readUInt16LE(offset + 26);
    const extraLength = bytes.readUInt16LE(offset + 28);
    const name = bytes.subarray(offset + 30, offset + 30 + nameLength).toString("utf8");
    const payloadStart = offset + 30 + nameLength + extraLength;
    const payload = bytes.subarray(payloadStart, payloadStart + compressedLength);
    const decoded = method === 8 ? inflateRawSync(payload) : payload;
    if (decoded.length !== uncompressedLength) throw new Error(`ZIP size mismatch: ${name}`);
    entries.push(name);
    offset = payloadStart + compressedLength;
  }
  return entries;
}

function readTar(bytes) {
  const entries = [];
  let offset = 0;
  while (offset + 512 <= bytes.length) {
    const header = bytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const end = header.indexOf(0);
    const name = header.subarray(0, end === -1 ? 100 : end).toString("utf8");
    const size = Number.parseInt(header.subarray(124, 136).toString("ascii").replace(/\0.*$/, "").trim() || "0", 8);
    entries.push(name);
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  return entries;
}
