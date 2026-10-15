#!/bin/sh
# Runs install.sh against a fake curl (and gh) on PATH serving a local fixture, so no network is used.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPT="$HERE/../install.sh"
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
FAILS=0
check() { if [ "$2" = ok ]; then echo "ok - $1"; else echo "not ok - $1"; FAILS=$((FAILS + 1)); fi; }

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) PLAT=macos-aarch64 ;; Darwin-x86_64) PLAT=macos-x86_64 ;;
  Linux-x86_64) PLAT=linux-x86_64 ;; Linux-aarch64 | Linux-arm64) PLAT=linux-aarch64 ;;
  *) echo "skip - unsupported platform"; exit 0 ;;
esac
ASSET="android-use-v1.2.3-$PLAT.tar.gz"
mkdir -p "$W/fix" "$W/stage" "$W/tools" "$W/gh"
printf '#!/bin/sh\necho new\n' > "$W/stage/au"
tar -czf "$W/fix/$ASSET" -C "$W/stage" au
if command -v sha256sum >/dev/null 2>&1; then (cd "$W/fix" && sha256sum "$ASSET" > SHA256SUMS); else (cd "$W/fix" && shasum -a 256 "$ASSET" > SHA256SUMS); fi

# Fake curl: logs its arguments, answers the latest-release probe, and copies fixture files.
cat > "$W/tools/curl" <<'CURL'
#!/bin/sh
echo "$*" >> "$FAKE_LOG"
OUT=""; PREV=""; URL=""
for a in "$@"; do
  [ "$PREV" = -o ] && OUT=$a
  case "$a" in http*) URL=$a ;; esac
  PREV=$a
done
case "$*" in *url_effective*) printf '%s' "$FAKE_LATEST"; exit 0 ;; esac
cp "$FAKE_FIX/${URL##*/}" "$OUT"
CURL
printf '#!/bin/sh\necho "$*" >> "$FAKE_LOG"\n[ "$1" = auth ] && exit "${FAKE_GH_AUTH:-0}"\nexit "${FAKE_GH_EXIT:-0}"\n' > "$W/gh/gh"
chmod +x "$W/tools/curl" "$W/gh/gh"
# A PATH holding only what install.sh needs, so a real gh or cosign on this machine cannot leak in.
for t in gzip sh awk grep tar mktemp uname sha256sum shasum mv rm chmod mkdir cat cp dirname printf; do
  p=$(command -v "$t" 2>/dev/null) && [ -x "$p" ] && ln -sf "$p" "$W/tools/$t"
done

run() { # run NAME EXTRA_PATH [VAR=value ...]; sets OUT and RC, with a previous au already installed
  EXTRA=$2; shift 2
  rm -rf "$W/dest"; mkdir "$W/dest"; echo old > "$W/dest/au"; : > "$W/log"
  OUT=$(env -i HOME="$W" PATH="$EXTRA$W/tools" AU_INSTALL_DIR="$W/dest" FAKE_LOG="$W/log" FAKE_FIX="$W/fix" \
    FAKE_LATEST="https://github.com/austinintelligence/android-use/releases/tag/v1.2.3" "$@" sh "$SCRIPT" 2>&1)
  RC=$?
}
refused() { if [ "$RC" -ne 0 ] && [ "$(cat "$W/dest/au")" = old ] && [ -z "$(ls -A "$W/dest" | grep -v '^au$')" ]; then check "$1" ok; else check "$1 (rc=$RC: $OUT)" no; fi; }

run bad "$W/gh:" ANDROID_USE_VERSION=1.2
refused "a short version is refused"
run bad "$W/gh:" 'ANDROID_USE_VERSION=1.2.3/../x'
refused "a version with path characters is refused"
run bad "$W/gh:" ANDROID_USE_VERSION=1.2.3-rc1
refused "a pre-release suffix is refused"

run http "$W/gh:" FAKE_LATEST=http://github.com/austinintelligence/android-use/releases/tag/v1.2.3
refused "an http release URL is refused"
run host "$W/gh:" FAKE_LATEST=https://evil.example/austinintelligence/android-use/releases/tag/v1.2.3
refused "a redirect to another host is refused"

printf '%s  %s\n' 0000000000000000000000000000000000000000000000000000000000000000 "$ASSET" > "$W/fix/SHA256SUMS.bad"
cp "$W/fix/SHA256SUMS" "$W/fix/SHA256SUMS.good"; cp "$W/fix/SHA256SUMS.bad" "$W/fix/SHA256SUMS"
run sum "$W/gh:"
refused "a bad checksum is refused"
case "$OUT" in *"SHA256 mismatch"*) check "the checksum failure is named" ok ;; *) check "the checksum failure is named: $OUT" no ;; esac
cp "$W/fix/SHA256SUMS.good" "$W/fix/SHA256SUMS"

run req "" AU_REQUIRE_ATTESTATION=1
refused "AU_REQUIRE_ATTESTATION=1 without gh is refused"
run fail "$W/gh:" FAKE_GH_EXIT=1
refused "a failed attestation is refused"

run login "$W/gh:" AU_REQUIRE_ATTESTATION=1 FAKE_GH_AUTH=1
refused "AU_REQUIRE_ATTESTATION=1 with gh not logged in is refused"
run login2 "$W/gh:" FAKE_GH_AUTH=1
if [ "$RC" -eq 0 ] && [ "$(cat "$W/dest/au")" != old ] && ! grep -q '^attestation' "$W/log"; then check "gh not logged in counts as no gh" ok; else check "gh not logged in: $OUT" no; fi

run none ""
if [ "$RC" -eq 0 ] && [ "$(cat "$W/dest/au")" != old ]; then check "no gh and no requirement installs after the checksum" ok; else check "no gh install: $OUT" no; fi

run good "$W/gh:" AU_REQUIRE_ATTESTATION=1
if [ "$RC" -eq 0 ] && [ -x "$W/dest/au" ] && [ "$(cat "$W/dest/au")" != old ] && [ -z "$(ls -A "$W/dest" | grep -v '^au$')" ]; then check "a verified archive installs in one move and leaves no staging" ok; else check "good install: $OUT" no; fi
grep -q "^attestation verify .*$ASSET --repo austinintelligence/android-use" "$W/log" && check "gh verifies the archive against this repo" ok || check "gh attestation call" no
if grep 'https://' "$W/log" | grep -v -e "--proto =https" -e "--tlsv1.2" >/dev/null; then check "every curl call is https-only TLS 1.2+" no; else check "every curl call is https-only TLS 1.2+" ok; fi

[ "$FAILS" -eq 0 ]
