#!/bin/sh
# Install the latest GitHub build on a Mac (no Rust needed):
#
#   curl -fsSL https://raw.githubusercontent.com/OWNER/REPO/main/install-latest.sh | sh -s OWNER/REPO
#
# Installed with curl (not a browser download), so macOS doesn't quarantine the
# unsigned app. Later updates come from the app itself.
set -e
REPO="${1:-$LAUNCHPAD_REPO}"
[ -n "$REPO" ] || { echo "Usage: sh install-latest.sh OWNER/REPO"; exit 1; }
DEST="/Applications/Targz Launchpad Rust.app"
TMP=$(mktemp -d)

URL=$(curl -fsSL -H "Accept: application/vnd.github+json" "https://api.github.com/repos/$REPO/releases/latest" \
  | grep -o '"browser_download_url": *"[^"]*\.zip"' | head -1 | sed 's/.*"\(http[^"]*\)"/\1/')
[ -n "$URL" ] || { echo "No release with a .zip found for $REPO"; exit 1; }

echo "Downloading $URL"
curl -fL -o "$TMP/app.zip" "$URL"
ditto -x -k "$TMP/app.zip" "$TMP"
osascript -e 'quit app "Targz Launchpad Rust"' 2>/dev/null || true
rm -rf "$DEST"
ditto "$TMP/Targz Launchpad Rust.app" "$DEST"
xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true
rm -rf "$TMP"
echo "Installed $DEST"
open "$DEST"
