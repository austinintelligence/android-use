#!/usr/bin/env node
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const here=dirname(fileURLToPath(import.meta.url));
const version="3.0.0";
const command=process.argv[2]||"setup";
if(command==="--version"||command==="version"){console.log(version);process.exit(0)}
if(command==="--help"||command==="help"){console.log("Android Use\n\n  npx android-use setup       Connect and prepare one Android device\n  npx android-use status      Show readiness\n  npx android-use doctor      Explain anything that needs attention\n  npx android-use update      Update the Android helper\n  npx android-use uninstall   Remove Android Use from the enrolled device\n\nThe platform binary owns device setup. Use --json for machine output.");process.exit(0)}
if(!["setup","repair","doctor","status","update","uninstall"].includes(command)){console.error("Expected setup, status, doctor, update, or uninstall.");process.exit(2)}
const platform=`${process.platform}-${process.arch}`;
const name=process.platform==="win32"?"au.exe":"au";
const candidates=[process.env.AU_BIN,join(here,"bin",platform,name),join(here,name)].filter(Boolean);
const binary=candidates.find(existsSync);
if(!binary){console.error(`The ${platform} au binary is missing from this package. Install the platform package or set AU_BIN.`);process.exit(1)}
const manifest=join(here,"manifest.json");
if(existsSync(manifest)){
  try{
    const files=JSON.parse(readFileSync(manifest,"utf8")).files??{};
    const verify=(file,key)=>{
      if(!files[key])return;
      if(!existsSync(file)){console.error("The Android Use package is incomplete. Reinstall it from a trusted source.");process.exit(1)}
      const actual=createHash("sha256").update(readFileSync(file)).digest("hex");
      if(actual!==files[key]){console.error("The Android Use package failed its integrity check. Reinstall it from a trusted source.");process.exit(1)}
    };
    verify(binary,relative(here,binary).replaceAll("\\","/"));
    const apk=join(dirname(binary),"aubridge.apk");
    verify(apk,relative(here,apk).replaceAll("\\","/"));
  }catch(error){console.error("The Android Use package manifest could not be verified.");process.exit(1)}
}
const child=spawn(binary,[command,...process.argv.slice(3)],{stdio:"inherit",windowsHide:true});
child.on("error",e=>{console.error(e.message);process.exit(1)});
child.on("exit",(code,signal)=>process.exit(signal?1:code??1));
