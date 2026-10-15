import test from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { chmod, mkdtemp, mkdir, writeFile, copyFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { writeManifest } from "../manifest.mjs";
const run = promisify(execFile), cli = join(dirname(fileURLToPath(import.meta.url)), "..", "cli.mjs");
const commands = ["setup", "repair", "doctor", "status", "update", "uninstall", "install", "serve", "read", "act", "devices", "enroll", "version"];
const posix = { skip: process.platform === "win32" && "uses a POSIX stub script" };

// A stub au that prints its argv as one JSON line, so the test sees exactly what the launcher passed.
async function argvThrough(t, args) {
  const dir = await mkdtemp(join(tmpdir(), "android-use-stub-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const bin = join(dir, "au-stub");
  await writeFile(bin, '#!/usr/bin/env node\nprocess.stdout.write(JSON.stringify(process.argv.slice(2)) + "\\n");\n');
  await chmod(bin, 0o755);
  const r = await run(process.execPath, [cli, ...args], { env: { ...process.env, AU_BIN: bin } });
  return JSON.parse(r.stdout);
}

test("reports the package version without network access",async()=>{const r=await run(process.execPath,[cli,"--version"]);assert.equal(r.stdout.trim(),"1.3.0")});
test("explains the platform binary and lists every au command",async()=>{const r=await run(process.execPath,[cli,"--help"]);assert.match(r.stdout,/platform binary/);for(const c of commands)assert.match(r.stdout,new RegExp(`\\b${c}\\b`))});
test("rejects commands outside the au surface",async()=>{await assert.rejects(run(process.execPath,[cli,"bogus"]),e=>e.code===2)});
test("fails closed when a platform binary is absent",async()=>{await assert.rejects(run(process.execPath,[cli,"doctor"],{env:{...process.env,AU_BIN:""}}),e=>/binary is missing/.test(e.stderr))});
test("fails closed when a bundled binary is tampered",async()=>{const root=await mkdtemp(join(tmpdir(),"android-use-test-"));try{const platform=`${process.platform}-${process.arch}`,name=process.platform==="win32"?"au.exe":"au",bin=join(root,"bin",platform,name),key=`bin/${platform}/${name}`;await mkdir(join(root,"bin",platform),{recursive:true});await copyFile(cli,join(root,"cli.mjs"));await copyFile(join(dirname(cli),"manifest.mjs"),join(root,"manifest.mjs"));await writeFile(bin,"tampered");await writeFile(join(root,"manifest.json"),JSON.stringify({files:{[key]:createHash("sha256").update("trusted").digest("hex")}}));await assert.rejects(run(process.execPath,[join(root,"cli.mjs"),"doctor"]),e=>/integrity check/.test(e.stderr))}finally{await rm(root,{recursive:true,force:true})}});
test("defaults to setup when no command is given",posix,async(t)=>{assert.deepEqual(await argvThrough(t,[]),["setup"])});
test("passes serve --mcp through verbatim with no launcher stdout",posix,async(t)=>{assert.deepEqual(await argvThrough(t,["serve","--mcp"]),["serve","--mcp"])});
test("passes read screen through verbatim",posix,async(t)=>{assert.deepEqual(await argvThrough(t,["read","screen"]),["read","screen"])});
test("passes act 'tap \"A\"' through as one argument",posix,async(t)=>{assert.deepEqual(await argvThrough(t,["act",'tap "A"']),["act",'tap "A"'])});
test("falls back to the optional platform package when AU_BIN is unset",posix,async(t)=>{
  const root=await mkdtemp(join(tmpdir(),"android-use-optional-"));t.after(()=>rm(root,{recursive:true,force:true}));
  const key=`${process.platform}-${process.arch}`,pkg=join(root,"node_modules","@androiduse",key),bin=join(pkg,"bin","au");
  await mkdir(join(pkg,"bin"),{recursive:true});
  await writeFile(join(pkg,"package.json"),JSON.stringify({name:`@androiduse/${key}`,version:"1.0.0"}));
  await writeFile(bin,'#!/usr/bin/env node\nprocess.stdout.write(JSON.stringify(process.argv.slice(2)) + "\\n");\n');
  await chmod(bin,0o755);writeManifest(pkg,["bin/au"]);await copyFile(cli,join(root,"cli.mjs"));await copyFile(join(dirname(cli),"manifest.mjs"),join(root,"manifest.mjs"));
  const r=await run(process.execPath,[join(root,"cli.mjs"),"serve","--mcp"],{env:{...process.env,AU_BIN:""}});
  assert.deepEqual(JSON.parse(r.stdout),["serve","--mcp"]);
});
