#!/bin/sh
# Install the au CLI from a GitHub release (the latest, or ANDROID_USE_VERSION). The archive is verified
# against SHA256SUMS, and against its build attestation when gh is installed (always, with
# AU_REQUIRE_ATTESTATION=1), before anything is extracted. The install is one rename; a failure leaves the
# previous au untouched.
set -eu

REPO="https://github.com/austinintelligence/android-use"
SLUG="austinintelligence/android-use"
INSTALL_DIR="${AU_INSTALL_DIR:-$HOME/.local/bin}"

fetch() { curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL "$@"; }
plain_version() { printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; }

# The newest release unless ANDROID_USE_VERSION pins one (for example 1.2.0 or v1.2.0).
if [ -n "${ANDROID_USE_VERSION:-}" ]; then
  VERSION="${ANDROID_USE_VERSION#v}"
  if ! plain_version "$VERSION"; then
    echo "ANDROID_USE_VERSION must look like 1.2.0 or v1.2.0. Aborting." >&2
    exit 1
  fi
else
  LATEST=$(fetch -I -o /dev/null -w '%{url_effective}' "$REPO/releases/latest") || LATEST=""
  VERSION="${LATEST##*/v}"
  case "$LATEST" in
    "$REPO"/releases/tag/v*) plain_version "$VERSION" || VERSION="" ;;
    *) VERSION="" ;;
  esac
  if [ -z "$VERSION" ]; then
    echo "Could not find the latest release over https. Set ANDROID_USE_VERSION." >&2
    exit 1
  fi
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

# Stage inside the install directory so the final move is a rename on one filesystem.
mkdir -p "$INSTALL_DIR"
TMP=$(mktemp -d "$INSTALL_DIR/.au-install.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
trap 'exit 1' INT TERM HUP
fetch -o "$TMP/$ASSET" "$BASE/$ASSET"
fetch -o "$TMP/SHA256SUMS" "$BASE/SHA256SUMS"

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

# The attestation proves the archive was built by this repository's release workflow. Only a logged-in gh can
# check it; gh without a login counts as no gh.
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  if ! gh attestation verify "$TMP/$ASSET" --repo "$SLUG" --signer-workflow "$SLUG/.github/workflows/release.yml" >/dev/null; then
    echo "Attestation check failed for $ASSET. Aborting." >&2
    exit 1
  fi
elif [ "${AU_REQUIRE_ATTESTATION:-}" = 1 ]; then
  echo "AU_REQUIRE_ATTESTATION=1 needs the GitHub CLI (gh), logged in with gh auth login, to verify the attestation (cosign alone cannot). Aborting." >&2
  exit 1
else
  echo "Skipping the attestation check: install gh and run gh auth login to enable it (SHA256SUMS was verified)." >&2
fi

tar -xzf "$TMP/$ASSET" -C "$TMP" au
chmod +x "$TMP/au"
mv -f "$TMP/au" "$INSTALL_DIR/au"
echo "Installed au to $INSTALL_DIR. Next: au setup"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "Add $INSTALL_DIR to PATH to run au from anywhere." ;;
esac
