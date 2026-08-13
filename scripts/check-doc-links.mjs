import { readdir, readFile, stat } from "node:fs/promises";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const ignored = /(?:[\\/]target[\\/]|[\\/]build[\\/]|[\\/]\.gradle[\\/]|[\\/]node_modules[\\/]|[\\/]artifacts[\\/])/;
const markdown = [];

async function walk(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (ignored.test(path)) continue;
    if (entry.isDirectory()) await walk(path);
    else if (entry.isFile() && entry.name.toLowerCase().endsWith(".md")) markdown.push(path);
  }
}

function targetFromLink(source, raw) {
  const link = raw.trim().split(/\s+/u)[0];
  if (!link || link.startsWith("#") || /^[a-z][a-z0-9+.-]*:/iu.test(link)) return null;
  const withoutAnchor = link.split("#", 1)[0].split("?", 1)[0];
  if (!withoutAnchor) return null;
  return resolve(source, "..", decodeURIComponent(withoutAnchor));
}

await walk(root);
const failures = [];
for (const source of markdown) {
  const body = await readFile(source, "utf8");
  for (const match of body.matchAll(/\[[^\]]*\]\(([^)]+)\)/gu)) {
    const target = targetFromLink(source, match[1]);
    if (!target) continue;
    try {
      await stat(target);
    } catch {
      failures.push(`${relative(root, source)} -> ${match[1]}`);
    }
  }
}
if (failures.length) {
  console.error(failures.join("\n"));
  process.exitCode = 1;
} else {
  console.log(`documentation links passed files=${markdown.length}`);
}
