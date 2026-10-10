#!/bin/sh
# Install the au CLI from a GitHub release (the latest, or ANDROID_USE_VERSION). The archive is verified
# against SHA256SUMS before anything is extracted.
set -eu

REPO="https://github.com/austinintelligence/android-use"
INSTALL_DIR="${AU_INSTALL_DIR:-$HOME/.local/bin}"

# The newest release unless ANDROID_USE_VERSION pins one (for example 1.2.0).
if [ -n "${ANDROID_USE_VERSION:-}" ]; then
  VERSION="${ANDROID_USE_VERSION#v}"
else
  LATEST=$(curl -fsSIL -o /dev/null -w '%{url_effective}' "$REPO/releases/latest")
  VERSION="${LATEST##*/v}"
  case "$VERSION" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "Could not find the latest release. Set ANDROID_USE_VERSION." >&2; exit 1 ;;
  esac
fi
BASE="$REPO/releases/download/v$VERSION"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) ASSET="android-use-v$VERSION-macos-aarch64.tar.gz" ;;
  Darwin-x86_64) ASSET="android-use-v$VERSION-macos-x86_64.tar.gz" ;;
  Linux-x86_64) ASSET="android-use-v$VERSION-linux-x86_64.tar.gz" ;;
  Linux-aarch64 | Linux-arm64) ASSET="android-use-v$VERSION-linux-aarch64.tar.gz" ;;
  *)
    echo "Unsupported platform: $(uname -s) $(uname -m). Supported: macOS arm64 and x86_64, Linux x86_64 and arm64 (Windows: install.ps1)." >&2
    exit 1
    ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | awk '{ print $1 }'; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | awk '{ print $1 }'; }
else
  echo "Need sha256sum or shasum to verify the download." >&2
  exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/$ASSET" "$BASE/$ASSET"
curl -fsSL -o "$TMP/SHA256SUMS" "$BASE/SHA256SUMS"

# Verify before extracting anything from the archive.
EXPECTED=$(awk -v f="$ASSET" '{ sub(/^\*/, "", $2) } $2 == f { print $1 }' "$TMP/SHA256SUMS")
if [ -z "$EXPECTED" ]; then
  echo "SHA256SUMS has no entry for $ASSET. Aborting." >&2
  exit 1
fi
ACTUAL=$(sha256 "$TMP/$ASSET")
if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "SHA256 mismatch for $ASSET (expected $EXPECTED, got $ACTUAL). Aborting." >&2
  exit 1
fi

tar -xzf "$TMP/$ASSET" -C "$TMP" au
mkdir -p "$INSTALL_DIR"
mv "$TMP/au" "$INSTALL_DIR/au"
chmod +x "$INSTALL_DIR/au"
echo "Installed au to $INSTALL_DIR. Next: au setup"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "Add $INSTALL_DIR to PATH to run au from anywhere." ;;
esac
