import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";

const values = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  const value = process.argv[index + 1];
  if (!key?.startsWith("--") || !value) throw new Error(`invalid argument ${key || ""}`);
  values.set(key.slice(2), value);
}
const version = values.get("version");
const baseUrl = values.get("base-url")?.replace(/\/$/, "");
const directory = resolve(values.get("directory") || "release/assets");
const output = resolve(values.get("output") || join(directory, "release-manifest.json"));
const signatureOutput = resolve(values.get("signature-output") || `${output}.sig`);
const signingKeyPath = values.get("signing-key") && resolve(values.get("signing-key"));
const publicKeyPath = values.get("public-key") && resolve(values.get("public-key"));
const helperSigner = values.get("helper-signer")?.trim().toLowerCase();
if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version || "")) throw new Error("--version must be semver");
if (!/^https:\/\//.test(baseUrl || "")) throw new Error("--base-url must use HTTPS");
if (!signingKeyPath || !publicKeyPath) throw new Error("--signing-key and --public-key are required");
if (!/^[0-9a-f]{64}$/.test(helperSigner || "")) throw new Error("--helper-signer must be one SHA-256 certificate fingerprint");

const privateKey = createPrivateKey(await readFile(signingKeyPath));
const publicKey = createPublicKey(await readFile(publicKeyPath));
const derivedPublic = createPublicKey(privateKey).export({ format: "der", type: "spki" });
const pinnedPublic = publicKey.export({ format: "der", type: "spki" });
if (!Buffer.from(derivedPublic).equals(Buffer.from(pinnedPublic))) {
  throw new Error("release manifest private key does not match the pinned installer public key");
}
const keyId = createHash("sha256").update(pinnedPublic).digest("hex");

const mapping = {
  "au-windows-x64.exe": "host_windows_x64",
  "au-windows-arm64.exe": "host_windows_arm64",
  "au-macos-x64": "host_macos_x64",
  "au-macos-arm64": "host_macos_arm64",
  "au-linux-x64": "host_linux_x64",
  "au-linux-arm64": "host_linux_arm64",
  "dev.codex.aubridge.apk": "helper_apk",
};

function assetKey(name) {
  if (mapping[name]) return mapping[name];
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const patterns = [
    [new RegExp(`^android-use-${escaped}-windows-(x64|arm64)\\.zip$`), (match) => `portable_windows_${match[1]}`],
    [new RegExp(`^android-use-${escaped}-(macos|linux)-(x64|arm64)\\.tar\\.gz$`), (match) => `portable_${match[1]}_${match[2]}`],
    [new RegExp(`^android-use_${escaped}-1_(amd64|arm64)\\.deb$`), (match) => `package_deb_${match[1] === "amd64" ? "x64" : "arm64"}`],
    [new RegExp(`^android-use-${escaped}-1\\.(x86_64|aarch64)\\.rpm$`), (match) => `package_rpm_${match[1] === "x86_64" ? "x64" : "arm64"}`],
    [new RegExp(`^android-use-${escaped}-windows-(x64|arm64)\\.msi$`), (match) => `package_msi_${match[1]}`],
    [/^android-use\.rb$/, () => "homebrew_formula"],
    [/^AndroidUse\.AndroidUse\.yaml$/, () => "winget_version_manifest"],
    [/^AndroidUse\.AndroidUse\.locale\.en-US\.yaml$/, () => "winget_locale_manifest"],
    [/^AndroidUse\.AndroidUse\.installer\.yaml$/, () => "winget_installer_manifest"],
  ];
  for (const [pattern, key] of patterns) {
    const match = name.match(pattern);
    if (match) return key(match);
  }
  return null;
}

const assets = {};
for (const name of (await readdir(directory)).sort()) {
  const key = assetKey(name);
  if (!key) continue;
  const bytes = await readFile(join(directory, name));
  assets[key] = {
    url: `${baseUrl}/${encodeURIComponent(basename(name))}`,
    bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}
for (const key of [
  "host_windows_x64", "host_windows_arm64", "host_macos_x64",
  "host_macos_arm64", "host_linux_x64", "host_linux_arm64", "helper_apk",
  "portable_windows_x64", "portable_windows_arm64", "portable_macos_x64",
  "portable_macos_arm64", "portable_linux_x64", "portable_linux_arm64",
  "package_deb_x64", "package_deb_arm64", "package_rpm_x64", "package_rpm_arm64",
  "package_msi_x64", "package_msi_arm64", "homebrew_formula",
  "winget_version_manifest", "winget_locale_manifest", "winget_installer_manifest",
]) {
  if (!assets[key]) throw new Error(`release asset is missing for ${key}`);
}
const manifest = { schema: 1, product: "android-use", version, protocol_version: 1, signing_key_id: keyId, helper_signer_sha256: helperSigner, assets };
const encoded = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, "utf8");
const signature = sign(null, encoded, privateKey);
await writeFile(output, encoded);
await writeFile(signatureOutput, `${JSON.stringify({ schema: 1, algorithm: "ed25519", key_id: keyId, signature: signature.toString("base64") })}\n`, "utf8");
console.log(JSON.stringify({ output, signature: signatureOutput, key_id: keyId, assets: Object.keys(assets) }));
