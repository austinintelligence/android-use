import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
const root=join(fileURLToPath(new URL("../..",import.meta.url))),prompt=(text,heading)=>text.split(`${heading}\n`,2)[1].match(/```text\r?\n([\s\S]*?)\r?\n```/)[1].replaceAll("\r\n","\n");
test("README and recovery guide keep the agent installer prompt aligned",async()=>{const [readme,guide]=await Promise.all([readFile(join(root,"README.md"),"utf8"),readFile(join(root,"docs","agents","install.md"),"utf8")]),p=prompt(readme,"## Use Android Use in your agent"),g=prompt(guide,"## Copy-paste prompt");assert.equal(p,g);for(const required of ["npx skills add","doctor --json","setup --json","serve --mcp","SHA256SUMS","next action"])assert.match(p,new RegExp(required));assert.match(g,/Build number seven times/);assert.match(g,/Settings → Accessibility → Android Use/);});
