#!/usr/bin/env node
// Print the Homebrew formula for a release, with checksums from its SHA256SUMS, which must be attestation-verified.
// Usage: node install/render-formula.mjs vX.Y.Z                       (downloads SHA256SUMS and verifies it with gh)
//        node install/render-formula.mjs vX.Y.Z SHA256SUMS --verified  (you already ran `gh attestation verify` on it)
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { verifyArgs } from "./manifest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const platforms = JSON.parse(readFileSync(join(here, "platforms.json"), "utf8"));

// Homebrew block for each platform: [on_macos|on_linux, on_arm|on_intel].
const BLOCKS = {
  "macos-aarch64": ["on_macos", "on_arm"],
  "macos-x86_64": ["on_macos", "on_intel"],
  "linux-x86_64": ["on_linux", "on_intel"],
  "linux-aarch64": ["on_linux", "on_arm"],
};

export function renderFormula(tag, sums) {
  const version = tag.replace(/^v/, "");
  const entries = platforms.filter((p) => BLOCKS[p.asset]).map((p) => {
    const archive = `android-use-v#{version}-${p.asset}.tar.gz`;
    const sha = sums.get(`android-use-v${version}-${p.asset}.tar.gz`);
    if (!sha) throw new Error(`SHA256SUMS has no entry for ${p.asset}`);
    return { os: BLOCKS[p.asset][0], cpu: BLOCKS[p.asset][1], archive, sha };
  });
  const block = (os) => `  ${os} do\n${entries.filter((e) => e.os === os).map((e) => `    ${e.cpu} do
      url "https://github.com/austinintelligence/android-use/releases/download/v#{version}/${e.archive}"
      sha256 "${e.sha}"
    end`).join("\n")}\n  end`;
  return `class AndroidUse < Formula
  desc "Give AI agents fast, bounded access to one Android device"
  homepage "https://github.com/austinintelligence/android-use"
  version "${version}"
  license "MIT"

${block("on_macos")}

${block("on_linux")}

  def install
    bin.install "au"
    pkgshare.install "aubridge.apk"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/au version")
  end
end
`;
}

export function parseSums(text) {
  return new Map(text.trim().split("\n").map((line) => line.trim().split(/\s+/)).map(([sha, name]) => [name.replace(/^\*/, ""), sha]));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [tag, path, flag] = process.argv.slice(2);
  if (!/^v\d+\.\d+\.\d+$/.test(tag ?? "") || (path && flag !== "--verified")) {
    console.error("Usage: node install/render-formula.mjs vX.Y.Z [SHA256SUMS --verified]\nChecksums that were not attestation-verified are refused.");
    process.exit(2);
  }
  let file = path;
  if (!file) {
    const dir = mkdtempSync(join(tmpdir(), "au-formula-"));
    file = join(dir, "SHA256SUMS");
    execFileSync("gh", ["release", "download", tag, "-p", "SHA256SUMS", "--dir", dir], { stdio: "inherit" });
    execFileSync("gh", verifyArgs(file, process.env.GITHUB_REPOSITORY ?? "austinintelligence/android-use", tag), { stdio: "inherit" });
  }
  process.stdout.write(renderFormula(tag, parseSums(readFileSync(file, "utf8"))));
}
