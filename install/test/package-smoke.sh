#!/bin/sh
# Pack the real npm packages around an au binary, install them into a fresh prefix, and check the launcher
# runs, serves MCP, and fails closed. Usage: package-smoke.sh PATH_TO_AU
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
AU="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
VERSION=$(tr -d '\n' < "$ROOT/VERSION")
KEY=$(node -p 'process.platform + "-" + process.arch')
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
mkdir "$W/main" "$W/tarballs" "$W/prefix"

# The same two packages publish-npm.mjs publishes.
cd "$ROOT"
node install/stage-platform.mjs "$KEY" "$AU" "$W/platform"
for f in cli.mjs manifest.mjs LICENSE README.md package.json; do cp "$ROOT/install/$f" "$W/main/$f"; done
node --input-type=module -e '
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
const [main, key, version] = process.argv.slice(1);
const { writeManifest } = await import("./install/manifest.mjs");
const pj = JSON.parse(readFileSync(join(main, "package.json"), "utf8"));
pj.optionalDependencies = { [`@androiduse/${key}`]: version };
writeFileSync(join(main, "package.json"), JSON.stringify(pj, null, 2));
writeManifest(main, ["cli.mjs", "manifest.mjs"]);
' "$W/main" "$KEY" "$VERSION"
(cd "$W/main" && npm pack --silent --pack-destination "$W/tarballs")
(cd "$W/platform" && npm pack --silent --pack-destination "$W/tarballs")

cd "$W/prefix"
npm init -y > /dev/null
npm install --no-audit --no-fund --ignore-scripts "$W"/tarballs/*.tgz
LAUNCHER="$W/prefix/node_modules/android-use/cli.mjs"
PLATFORM_DIR="$W/prefix/node_modules/@androiduse/$KEY"
BIN=$(ls "$PLATFORM_DIR"/bin/au*)

echo "== android-use read help"
HELP=$(node "$LAUNCHER" read help)
case "$HELP" in *android_read*) ;; *) echo "read help did not list android_read" >&2; exit 1 ;; esac
echo "== MCP initialize through serve --mcp"
REPLY=$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"package-smoke","version":"0"}}}' | node "$LAUNCHER" serve --mcp)
case "$REPLY" in *'"result"'*) ;; *) echo "initialize got no result: $REPLY" >&2; exit 1 ;; esac

# Fail closed: each of these must make the launcher exit 1, not run the binary.
expect_exit_1() {
  rc=0
  node "$LAUNCHER" read help > /dev/null 2>&1 || rc=$?
  [ "$rc" -eq 1 ] || { echo "$1: expected exit 1, got $rc" >&2; exit 1; }
  echo "ok - $1"
}
cp "$BIN" "$W/au.orig"
node -e '
const fs = require("node:fs");
const b = fs.readFileSync(process.argv[1]);
b[b.length - 1] ^= 1;
fs.writeFileSync(process.argv[1], b);
' "$BIN"
expect_exit_1 "a binary with one byte flipped"
cp "$W/au.orig" "$BIN"
rm "$PLATFORM_DIR/manifest.json"
expect_exit_1 "a missing manifest"
echo "package smoke passed on $KEY"
