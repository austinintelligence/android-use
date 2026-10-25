import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = join(fileURLToPath(new URL("../..", import.meta.url)));

function prompt(text, heading) {
  const section = text.split(`${heading}\n`, 2)[1];
  assert.ok(section, `missing documentation heading: ${heading}`);
  const match = section.match(/```text\r?\n([\s\S]*?)\r?\n```/);
  assert.ok(match, `missing copy-paste prompt under: ${heading}`);
  return match[1].replaceAll("\r\n", "\n");
}

test("README keeps the two-line prompt and the recovery guide keeps the full contract", async () => {
  const [readme, guide] = await Promise.all([
    readFile(join(root, "README.md"), "utf8"),
    readFile(join(root, "docs", "agents", "install.md"), "utf8"),
  ]);
  const version = (await readFile(join(root, "VERSION"), "utf8")).trim();
  const short = prompt(readme, "## Get started");
  assert.ok(short.length < 200, "the README prompt stays short");
  for (const required of [`npx -y android-use@${version} setup`, "android_read", "android_act"]) {
    assert.ok(short.includes(required), `README prompt is missing: ${required}`);
  }
  const full = prompt(guide, "## Copy-paste setup prompt");
  for (const required of ["preserve unrelated files", "au setup", "au doctor", "serve --mcp", "android_read command status", "android_read command screen", "command-string tools", "raw ADB", "partial or unknown mutation"]) {
    assert.ok(full.toLowerCase().includes(required.toLowerCase()), `recovery prompt is missing: ${required}`);
  }
  assert.doesNotMatch(full, /Austin|Network & internet|Airplane mode/);
});
