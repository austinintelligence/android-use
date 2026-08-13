import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { createReadStream } from "node:fs";
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rm, stat, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { promisify } from "node:util";
import { deflateRawSync, gzipSync } from "node:zlib";

const run = promisify(execFile);
const values = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  const value = process.argv[index + 1];
  if (!key?.startsWith("--") || !value) throw new Error(`invalid argument ${key || ""}`);
  values.set(key.slice(2), value);
}

const version = values.get("version");
const assets = resolve(values.get("assets") || "release/assets");
const output = resolve(values.get("output") || "release/packages");
const repository = resolve(values.get("repository") || ".");
const githubRepository = values.get("github-repository") || process.env.GITHUB_REPOSITORY || "austinintelligence/android-use";
const formats = values.get("formats") || "all";
const sourceDateEpoch = Number(values.get("source-date-epoch") || process.env.SOURCE_DATE_EPOCH || 0);
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version || "")) throw new Error("--version must be strict semver");
if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(githubRepository)) throw new Error("--github-repository must be owner/name");
if (!new Set(["all", "portable"]).has(formats)) throw new Error("--formats must be all or portable");
if (!Number.isSafeInteger(sourceDateEpoch) || sourceDateEpoch <= 0) throw new Error("--source-date-epoch must be a positive Unix timestamp");
await mkdir(output, { recursive: true });

const helper = join(assets, "dev.codex.aubridge.apk");
const common = [
  [join(repository, "LICENSE"), "LICENSE"],
  [join(repository, "THIRD_PARTY_NOTICES.md"), "THIRD_PARTY_NOTICES.md"],
];
await requireFile(helper);
for (const [source] of common) await requireFile(source);

const targets = [
  { os: "windows", arch: "x64", host: "au-windows-x64.exe", executable: "au.exe", archive: "zip", deb: null, rpm: null },
  { os: "windows", arch: "arm64", host: "au-windows-arm64.exe", executable: "au.exe", archive: "zip", deb: null, rpm: null },
  { os: "macos", arch: "x64", host: "au-macos-x64", executable: "au", archive: "tar.gz", deb: null, rpm: null },
  { os: "macos", arch: "arm64", host: "au-macos-arm64", executable: "au", archive: "tar.gz", deb: null, rpm: null },
  { os: "linux", arch: "x64", host: "au-linux-x64", executable: "au", archive: "tar.gz", deb: "amd64", rpm: "x86_64" },
  { os: "linux", arch: "arm64", host: "au-linux-arm64", executable: "au", archive: "tar.gz", deb: "arm64", rpm: "aarch64" },
];

const temporary = await mkdtemp(join(tmpdir(), "android-use-packages-"));
const generated = [];
try {
  for (const target of targets) {
    const host = join(assets, target.host);
    await requireFile(host);
    const rootName = `android-use-${version}`;
    const root = join(temporary, `${target.os}-${target.arch}`, rootName);
    const binary = join(root, "bin", target.executable);
    const packagedHelper = join(root, "share", "android-use", "dev.codex.aubridge.apk");
    await mkdir(dirname(binary), { recursive: true });
    await mkdir(dirname(packagedHelper), { recursive: true });
    await copyFile(host, binary);
    await copyFile(helper, packagedHelper);
    if (target.os !== "windows") await chmod(binary, 0o755);
    for (const [source, name] of common) await copyFile(source, join(root, name));
    const manifest = {
      schema: 1,
      product: "android-use",
      version,
      protocol_version: 1,
      os: target.os,
      arch: target.arch,
      binary: `bin/${target.executable}`,
      helper: "share/android-use/dev.codex.aubridge.apk",
      au_sha256: await sha256(binary),
      helper_sha256: await sha256(packagedHelper),
    };
    await writeFile(join(root, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
    await normalizeTree(join(temporary, `${target.os}-${target.arch}`), sourceDateEpoch);

    const archiveName = `android-use-${version}-${target.os}-${target.arch}.${target.archive}`;
    const archive = join(output, archiveName);
    const entries = await listFiles(root, rootName);
    if (target.archive === "zip") {
      await createZip(archive, dirname(root), entries, sourceDateEpoch);
    } else {
      await createTarGz(archive, dirname(root), entries, sourceDateEpoch);
    }
    generated.push(archive);

    if (formats === "all" && target.deb) generated.push(await buildDeb(target, root, temporary, output));
    if (formats === "all" && target.rpm) generated.push(await buildRpm(target, root, temporary, output));
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}

const macosArm = generated.find((path) => path.endsWith(`macos-arm64.tar.gz`));
const macosIntel = generated.find((path) => path.endsWith(`macos-x64.tar.gz`));
if (!macosArm || !macosIntel) throw new Error("macOS archives are required for the Homebrew formula");
const formulaTemplate = await readFile(join(repository, "packaging", "homebrew", "android-use.rb.in"), "utf8");
const formula = formulaTemplate
  .replaceAll("@VERSION@", version)
  .replaceAll("@REPOSITORY@", githubRepository)
  .replaceAll("@MACOS_ARM64_SHA256@", await sha256(macosArm))
  .replaceAll("@MACOS_X64_SHA256@", await sha256(macosIntel));
const formulaPath = join(output, "android-use.rb");
await writeFile(formulaPath, formula);
generated.push(formulaPath);

process.stdout.write(`${JSON.stringify({ version, generated: await Promise.all(generated.map(describe)) })}\n`);

async function buildDeb(target, root, work, destination) {
  const packageRoot = join(work, `deb-${target.arch}`);
  await mkdir(join(packageRoot, "DEBIAN"), { recursive: true });
  await installFile(join(root, "bin", "au"), join(packageRoot, "usr", "bin", "au"), 0o755);
  await installFile(join(root, "share", "android-use", "dev.codex.aubridge.apk"), join(packageRoot, "usr", "share", "android-use", "dev.codex.aubridge.apk"), 0o644);
  await installFile(join(root, "LICENSE"), join(packageRoot, "usr", "share", "doc", "android-use", "LICENSE"), 0o644);
  await installFile(join(root, "THIRD_PARTY_NOTICES.md"), join(packageRoot, "usr", "share", "doc", "android-use", "THIRD_PARTY_NOTICES.md"), 0o644);
  const control = [
    "Package: android-use",
    `Version: ${version}-1`,
    "Section: utils",
    "Priority: optional",
    `Architecture: ${target.deb}`,
    "Maintainer: Android Use maintainers <austinintelligence@users.noreply.github.com>",
    `Homepage: https://github.com/${githubRepository}`,
    "Description: Fast, bounded Android control for authorized AI agents",
    " A local-first Rust CLI for authorized Android device control.",
    "",
  ].join("\n");
  await writeFile(join(packageRoot, "DEBIAN", "control"), control);
  await normalizeTree(packageRoot, sourceDateEpoch);
  const result = join(destination, `android-use_${version}-1_${target.deb}.deb`);
  await run("dpkg-deb", ["--build", "--root-owner-group", packageRoot, result], {
    env: { ...process.env, SOURCE_DATE_EPOCH: String(sourceDateEpoch) },
    maxBuffer: 512 * 1024,
  });
  await run("dpkg-deb", ["--info", result], { maxBuffer: 512 * 1024 });
  await run("dpkg-deb", ["--contents", result], { maxBuffer: 512 * 1024 });
  return result;
}

async function buildRpm(target, root, work, destination) {
  const top = join(work, `rpm-${target.arch}`);
  for (const name of ["BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS"]) await mkdir(join(top, name), { recursive: true });
  await copyFile(join(root, "bin", "au"), join(top, "SOURCES", "au"));
  await copyFile(join(root, "share", "android-use", "dev.codex.aubridge.apk"), join(top, "SOURCES", "dev.codex.aubridge.apk"));
  await copyFile(join(root, "LICENSE"), join(top, "SOURCES", "LICENSE"));
  await copyFile(join(root, "THIRD_PARTY_NOTICES.md"), join(top, "SOURCES", "THIRD_PARTY_NOTICES.md"));
  const spec = `Name: android-use
Version: ${version.replace(/-.*/, "")}
Release: 1
Summary: Fast, bounded Android control for authorized AI agents
License: MIT
URL: https://github.com/${githubRepository}
Source0: au
Source1: dev.codex.aubridge.apk
Source2: LICENSE
Source3: THIRD_PARTY_NOTICES.md
BuildArch: ${target.rpm}
AutoReqProv: no

%description
A local-first Rust CLI for authorized Android device control.

%prep
%build

%install
install -Dpm0755 %{SOURCE0} %{buildroot}%{_bindir}/au
install -Dpm0644 %{SOURCE1} %{buildroot}%{_datadir}/android-use/dev.codex.aubridge.apk
install -Dpm0644 %{SOURCE2} %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dpm0644 %{SOURCE3} %{buildroot}%{_docdir}/%{name}/THIRD_PARTY_NOTICES.md

%files
%{_bindir}/au
%{_datadir}/android-use/dev.codex.aubridge.apk
%license %{_licensedir}/%{name}/LICENSE
%doc %{_docdir}/%{name}/THIRD_PARTY_NOTICES.md
`;
  const specPath = join(top, "SPECS", "android-use.spec");
  await writeFile(specPath, spec);
  await run("rpmbuild", [
    "--define", `_topdir ${top}`,
    "--define", `_source_date_epoch ${sourceDateEpoch}`,
    "--define", "_buildhost localhost",
    "--target", target.rpm,
    "-bb", specPath,
  ], { env: { ...process.env, SOURCE_DATE_EPOCH: String(sourceDateEpoch) }, maxBuffer: 1024 * 1024 });
  const built = (await findFiles(join(top, "RPMS"))).find((path) => path.endsWith(".rpm"));
  if (!built) throw new Error(`rpmbuild did not create a package for ${target.rpm}`);
  const result = join(destination, `android-use-${version}-1.${target.rpm}.rpm`);
  await copyFile(built, result);
  await run("rpm", ["-qip", result], { maxBuffer: 512 * 1024 });
  await run("rpm", ["-qpl", result], { maxBuffer: 512 * 1024 });
  await run("rpm", ["-qpR", result], { maxBuffer: 512 * 1024 });
  await run("rpm", ["-qp", "--scripts", result], { maxBuffer: 512 * 1024 });
  return result;
}

async function installFile(source, destination, mode) {
  await mkdir(dirname(destination), { recursive: true });
  await copyFile(source, destination);
  await chmod(destination, mode);
}

async function normalizeTree(root, epoch) {
  const time = new Date(epoch * 1000);
  for (const path of await findFiles(root, true)) await utimes(path, time, time);
  await utimes(root, time, time);
}

async function listFiles(root, prefix) {
  const entries = [];
  async function visit(directory, relative = "") {
    const children = (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name));
    for (const child of children) {
      const name = relative ? `${relative}/${child.name}` : child.name;
      if (child.isDirectory()) await visit(join(directory, child.name), name);
      else if (child.isFile()) entries.push(`${prefix}/${name}`);
      else throw new Error(`unsupported package entry ${name}`);
    }
  }
  await visit(root);
  return entries;
}

async function findFiles(root, includeDirectories = false) {
  const results = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (includeDirectories) results.push(path);
        await visit(path);
      } else if (entry.isFile()) results.push(path);
    }
  }
  await visit(root);
  return results;
}

async function createZip(destination, base, entries, epoch) {
  if (entries.length > 65_535) throw new Error("ZIP entry count exceeds the non-ZIP64 limit");
  const localRecords = [];
  const centralRecords = [];
  let offset = 0;
  const [dosTime, dosDate] = dosTimestamp(epoch);
  for (const entry of entries) {
    const name = Buffer.from(entry.replaceAll("\\", "/"), "utf8");
    const bytes = await readFile(join(base, entry));
    const compressed = deflateRawSync(bytes, { level: 9 });
    const crc = crc32(bytes);
    if (bytes.length > 0xffff_ffff || compressed.length > 0xffff_ffff || offset > 0xffff_ffff) {
      throw new Error("ZIP payload exceeds the non-ZIP64 limit");
    }
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6);
    local.writeUInt16LE(8, 8);
    local.writeUInt16LE(dosTime, 10);
    local.writeUInt16LE(dosDate, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(compressed.length, 18);
    local.writeUInt32LE(bytes.length, 22);
    local.writeUInt16LE(name.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(0x0314, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x0800, 8);
    central.writeUInt16LE(8, 10);
    central.writeUInt16LE(dosTime, 12);
    central.writeUInt16LE(dosDate, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(compressed.length, 20);
    central.writeUInt32LE(bytes.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt32LE((0o100644 << 16) >>> 0, 38);
    central.writeUInt32LE(offset, 42);
    localRecords.push(local, name, compressed);
    centralRecords.push(central, name);
    offset += local.length + name.length + compressed.length;
  }
  const centralSize = centralRecords.reduce((total, value) => total + value.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralSize, 12);
  end.writeUInt32LE(offset, 16);
  await writeFile(destination, Buffer.concat([...localRecords, ...centralRecords, end]));
}

async function createTarGz(destination, base, entries, epoch) {
  const records = [];
  for (const entry of entries) {
    const name = entry.replaceAll("\\", "/");
    if (Buffer.byteLength(name) > 100) throw new Error(`tar entry name exceeds ustar limit: ${name}`);
    const bytes = await readFile(join(base, entry));
    const header = Buffer.alloc(512);
    header.write(name, 0, 100, "utf8");
    writeTarOctal(header, 100, 8, name.endsWith("/bin/au") ? 0o755 : 0o644);
    writeTarOctal(header, 108, 8, 0);
    writeTarOctal(header, 116, 8, 0);
    writeTarOctal(header, 124, 12, bytes.length);
    writeTarOctal(header, 136, 12, epoch);
    header.fill(0x20, 148, 156);
    header[156] = 0x30;
    header.write("ustar\0", 257, 6, "ascii");
    header.write("00", 263, 2, "ascii");
    const checksum = header.reduce((sum, byte) => sum + byte, 0).toString(8).padStart(6, "0");
    header.write(checksum, 148, 6, "ascii");
    header[154] = 0;
    header[155] = 0x20;
    records.push(header, bytes);
    const remainder = bytes.length % 512;
    if (remainder) records.push(Buffer.alloc(512 - remainder));
  }
  records.push(Buffer.alloc(1024));
  await writeFile(destination, gzipSync(Buffer.concat(records), { level: 9, mtime: 0 }));
}

function writeTarOctal(buffer, offset, length, value) {
  const encoded = Math.trunc(value).toString(8).padStart(length - 1, "0");
  if (encoded.length >= length) throw new Error(`tar numeric field overflow: ${value}`);
  buffer.write(encoded, offset, length - 1, "ascii");
  buffer[offset + length - 1] = 0;
}

function dosTimestamp(epoch) {
  const date = new Date(epoch * 1000);
  const year = Math.min(2107, Math.max(1980, date.getUTCFullYear()));
  const time = (date.getUTCHours() << 11) | (date.getUTCMinutes() << 5) | Math.floor(date.getUTCSeconds() / 2);
  const day = (year === date.getUTCFullYear()) ? date.getUTCDate() : 1;
  const month = (year === date.getUTCFullYear()) ? date.getUTCMonth() + 1 : 1;
  return [time, ((year - 1980) << 9) | (month << 5) | day];
}

function crc32(bytes) {
  const table = crc32.table ??= Array.from({ length: 256 }, (_, index) => {
    let entry = index;
    for (let bit = 0; bit < 8; bit++) entry = (entry >>> 1) ^ ((entry & 1) ? 0xedb88320 : 0);
    return entry >>> 0;
  });
  let value = 0xffff_ffff;
  for (const byte of bytes) value = (value >>> 8) ^ table[(value ^ byte) & 0xff];
  return (value ^ 0xffff_ffff) >>> 0;
}

async function requireFile(path) {
  if (!(await stat(path).catch(() => null))?.isFile()) throw new Error(`required release input is missing: ${path}`);
}

async function sha256(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

async function describe(path) {
  return { file: basename(path), bytes: (await stat(path)).size, sha256: await sha256(path) };
}
