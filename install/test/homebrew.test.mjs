import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseSums, renderFormula } from "../render-formula.mjs";

const platforms = JSON.parse(readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "platforms.json"), "utf8"));
const sums = parseSums(
  platforms.map((p, i) => `${String(i).repeat(64)}  android-use-v9.8.7-${p.asset}.${p.archive}`).join("\n"),
);

test("the formula declares the AndroidUse class and the version", () => {
  const formula = renderFormula("v9.8.7", sums);
  assert.match(formula, /^class AndroidUse < Formula$/m);
  assert.match(formula, /^ {2}version "9\.8\.7"$/m);
});

test("every macOS and Linux platform has a url and its own checksum", () => {
  const formula = renderFormula("v9.8.7", sums);
  for (const asset of ["macos-aarch64", "macos-x86_64", "linux-x86_64", "linux-aarch64"]) assert.match(formula, new RegExp(`${asset}\\.tar\\.gz`));
  assert.equal((formula.match(/sha256 "[0-9a-f]{64}"/g) ?? []).length, 4);
  assert.match(formula, /on_macos do\n {4}on_arm do/);
  assert.match(formula, /on_linux do\n {4}on_intel do/);
});

test("a missing checksum fails instead of rendering a broken formula", () => {
  const partial = new Map([...sums].filter(([name]) => !name.includes("linux-aarch64")));
  assert.throws(() => renderFormula("v9.8.7", partial), /linux-aarch64/);
});
