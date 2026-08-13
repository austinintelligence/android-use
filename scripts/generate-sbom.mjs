import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const output = resolve(process.argv[2] || join(root, "release", "assets"));
const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const npmCli = join(dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js");

function run(file, args) {
  return execFileSync(file, args, { cwd: root, encoding: "utf8", maxBuffer: 8 * 1024 * 1024 }).trim();
}

function packageId(pkg) {
  return `SPDXRef-${pkg.name.replace(/[^A-Za-z0-9.-]/g, "-")}-${pkg.version.replace(/[^A-Za-z0-9.-]/g, "-")}`;
}

function cargoSpdx(metadata, commit) {
  const byId = new Map(metadata.packages.map((pkg) => [pkg.id, packageId(pkg)]));
  const packages = metadata.packages.map((pkg) => {
    const source = pkg.source || "NOASSERTION";
    const refs = source.startsWith("registry+")
      ? [{ referenceCategory: "PACKAGE-MANAGER", referenceType: "purl", referenceLocator: `pkg:cargo/${pkg.name}@${pkg.version}` }]
      : [];
    return {
      SPDXID: byId.get(pkg.id),
      name: pkg.name,
      versionInfo: pkg.version,
      downloadLocation: source,
      licenseConcluded: "NOASSERTION",
      licenseDeclared: pkg.license || "NOASSERTION",
      copyrightText: "NOASSERTION",
      filesAnalyzed: false,
      externalRefs: refs,
    };
  });
  const relationships = [{
    spdxElementId: "SPDXRef-DOCUMENT",
    relationshipType: "DESCRIBES",
    relatedSpdxElement: packageId(metadata.packages.find((pkg) => pkg.name === "android-use") || metadata.packages[0]),
  }];
  for (const node of metadata.resolve?.nodes || []) {
    const from = byId.get(node.id);
    if (!from) continue;
    for (const dependency of node.dependencies || []) {
      const to = byId.get(dependency);
      if (to) relationships.push({ spdxElementId: from, relationshipType: "DEPENDS_ON", relatedSpdxElement: to });
    }
  }
  return {
    SPDXID: "SPDXRef-DOCUMENT",
    spdxVersion: "SPDX-2.3",
    creationInfo: {
      created: new Date().toISOString(),
      creators: ["Tool: android-use/scripts/generate-sbom.mjs"],
    },
    name: "android-use Rust host dependency inventory",
    documentNamespace: `https://github.com/austinintelligence/android-use/sbom/cargo/${commit}`,
    dataLicense: "CC0-1.0",
    packages,
    relationships,
  };
}

async function writeJson(path, value) {
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

const commit = run("git", ["rev-parse", "HEAD"]);
const metadata = JSON.parse(run(cargo, ["metadata", "--locked", "--format-version", "1"]));
const npmArgs = [
  "sbom",
  "--workspace",
  "packages/installer",
  "--package-lock-only",
  "--sbom-format",
  "spdx",
  "--sbom-type",
  "application",
];
const npmSbom = JSON.parse(process.platform === "win32"
  ? run(process.execPath, [npmCli, ...npmArgs])
  : run("npm", npmArgs));

await mkdir(output, { recursive: true });
const cargoPath = join(output, "android-use-cargo.spdx.json");
const npmPath = join(output, "android-use-npm.spdx.json");
await writeJson(cargoPath, cargoSpdx(metadata, commit));
await writeJson(npmPath, npmSbom);

const assets = [];
for (const path of [cargoPath, npmPath]) {
  const bytes = await readFile(path);
  assets.push({
    name: path.split(/[\\/]/).pop(),
    bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  });
}
await writeJson(join(output, "sbom-manifest.json"), {
  schema: 1,
  product: "android-use",
  source_commit: commit,
  assets,
});
console.log(JSON.stringify({ output, source_commit: commit, assets }));
