import { generateKeyPairSync, createHash } from "node:crypto";
import { mkdir, open, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const values = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  const value = process.argv[index + 1];
  if (!key?.startsWith("--") || !value) throw new Error(`invalid argument ${key || ""}`);
  values.set(key.slice(2), value);
}

const privateOutput = resolve(values.get("private-output") || "");
const publicOutput = resolve(values.get("public-output") || "");
if (!values.get("private-output") || !values.get("public-output")) {
  throw new Error("--private-output and --public-output are required");
}

for (const destination of [privateOutput, publicOutput]) {
  await mkdir(dirname(destination), { recursive: true });
  const handle = await open(destination, "wx", 0o600).catch((error) => {
    if (error.code === "EEXIST") throw new Error(`refusing to replace existing key material: ${destination}`);
    throw error;
  });
  await handle.close();
}

try {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const privatePem = privateKey.export({ format: "pem", type: "pkcs8" });
  const publicPem = publicKey.export({ format: "pem", type: "spki" });
  await writeFile(privateOutput, privatePem, { mode: 0o600 });
  await writeFile(publicOutput, publicPem, { mode: 0o644 });
  const publicDer = publicKey.export({ format: "der", type: "spki" });
  const keyId = createHash("sha256").update(publicDer).digest("hex");
  process.stdout.write(`${JSON.stringify({ algorithm: "ed25519", key_id: keyId, public_key: publicOutput })}\n`);
} catch (error) {
  await Promise.all([
    import("node:fs/promises").then(({ rm }) => rm(privateOutput, { force: true })),
    import("node:fs/promises").then(({ rm }) => rm(publicOutput, { force: true })),
  ]);
  throw error;
}
