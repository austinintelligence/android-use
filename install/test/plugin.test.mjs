import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const json = async (file) => JSON.parse(await readFile(join(root, file), "utf8"));

test("plugin, marketplace, and npm package agree on the version", async () => {
  const version = (await readFile(join(root, "VERSION"), "utf8")).trim();
  const plugin = await json(".claude-plugin/plugin.json");
  const marketplace = await json(".claude-plugin/marketplace.json");
  const installPkg = await json("install/package.json");
  assert.equal(plugin.version, version);
  assert.equal(installPkg.version, version);
  assert.equal(marketplace.plugins[0].version, version);
  assert.equal(marketplace.plugins[0].name, plugin.name);
});

test("MCP config launches the npm server pinned to VERSION", async () => {
  const version = (await readFile(join(root, "VERSION"), "utf8")).trim();
  const mcp = await json(".mcp.json");
  assert.deepEqual(mcp.mcpServers["android-use"], {
    command: "npx",
    args: ["-y", `android-use@${version}`, "serve", "--mcp"],
  });
});

test("the plugin ships the android-use skill", async () => {
  const skill = await readFile(join(root, "skills", "android-use", "SKILL.md"), "utf8");
  assert.match(skill, /^name: android-use$/m);
});
