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

test("README and recovery guide keep the agent installer contract aligned", async () => {
  const [readme, guide] = await Promise.all([
    readFile(join(root, "README.md"), "utf8"),
    readFile(join(root, "docs", "agents", "install.md"), "utf8"),
  ]);
  const prompts = [
    prompt(readme, "## Use Android Use in your agent"),
    prompt(guide, "## Copy-paste setup prompt"),
  ];
  for (const required of [
    "preserve unrelated files",
    "au setup",
    "au doctor",
    "serve --mcp",
    "android.read command status",
    "android.read command screen",
    "command-string tools",
  ]) {
    for (const value of prompts) {
      assert.ok(value.toLowerCase().includes(required.toLowerCase()), `missing shared setup contract: ${required}`);
    }
  }
  for (const value of prompts) {
    assert.match(value, /raw ADB/);
    assert.match(value, /partial or unknown mutation/);
    assert.doesNotMatch(value, /Austin|Network & internet|Airplane mode/);
  }
});
