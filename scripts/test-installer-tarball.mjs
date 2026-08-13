import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";

const run = promisify(execFile);
const root = new URL("..", import.meta.url).pathname.replace(/^\/(?=[A-Za-z]:)/, "");
const packageRoot = join(root, "packages", "installer");
const node = process.execPath;
const npmCli = process.env.npm_execpath && existsSync(process.env.npm_execpath)
  ? process.env.npm_execpath
  : join(dirname(node), "node_modules", "npm", "bin", "npm-cli.js");
const runNpm = (args, options) => existsSync(npmCli)
  ? run(node, [npmCli, ...args], options)
  : run(process.platform === "win32" ? "npm.cmd" : "npm", args, options);
const temporary = await mkdtemp(join(tmpdir(), "android-use-npm-tarball-"));
const tarballRoot = join(temporary, "tarball");
const installRoot = join(temporary, "install");

try {
  await mkdir(tarballRoot, { recursive: true });
  const packed = await runNpm(["pack", "--json", "--pack-destination", tarballRoot], {
    cwd: packageRoot,
    windowsHide: true,
    maxBuffer: 512 * 1024,
  });
  const records = JSON.parse(packed.stdout);
  if (!Array.isArray(records) || records.length !== 1 || typeof records[0].filename !== "string") {
    throw new Error("npm pack did not return exactly one tarball");
  }
  const tarball = join(tarballRoot, records[0].filename);
  await runNpm(["install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--prefix", installRoot, tarball], {
    cwd: root,
    windowsHide: true,
    maxBuffer: 512 * 1024,
  });
  const entrypoint = join(installRoot, "node_modules", "android-use", "cli.mjs");
  const version = JSON.parse((await run(node, [entrypoint, "--version", "--json"], {
    cwd: root,
    windowsHide: true,
    maxBuffer: 64 * 1024,
  })).stdout);
  const packageJson = JSON.parse(await readFile(join(installRoot, "node_modules", "android-use", "package.json"), "utf8"));
  if (version.ok !== true || version.version !== packageJson.version) {
    throw new Error(`packed CLI version mismatch: ${JSON.stringify(version)}`);
  }
  process.stdout.write(`${JSON.stringify({ package: packageJson.name, version: packageJson.version, tarball: records[0].filename })}\n`);
} finally {
  await rm(temporary, { recursive: true, force: true });
}
