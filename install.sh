#!/bin/sh
# Build Targz Launchpad and install it as a menu-bar app.
#
# By hand:      ./install.sh                 (needs Rust; updates later come from git)
# On GitHub:    the workflow runs it with UNIVERSAL=1 NO_INSTALL=1 and publishes
#               the zipped .app as a release; installed copies update from there.
#
# DEST overrides where the .app goes (the git updater passes the running copy).
set -e
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

NAME="Targz Launchpad Rust"
DEST="${DEST:-/Applications/$NAME.app}"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

if [ "$UNIVERSAL" = "1" ]; then
  # Apple Silicon + Intel in one binary
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/release
  lipo -create -output target/release/targz-launchpad \
    target/aarch64-apple-darwin/release/targz-launchpad \
    target/x86_64-apple-darwin/release/targz-launchpad
else
  cargo build --release
fi

BUNDLE="target/$NAME.app"
rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp target/release/targz-launchpad "$BUNDLE/Contents/MacOS/targz-launchpad"
cp icon.icns "$BUNDLE/Contents/Resources/icon.icns"
cat > "$BUNDLE/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>com.targz.launchpad.rust</string>
  <key>CFBundleExecutable</key><string>targz-launchpad</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>${LAUNCHPAD_BUILD:-$VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
# ad-hoc signature: Apple Silicon refuses to run unsigned code
codesign --force --deep -s - "$BUNDLE"

if [ "$NO_INSTALL" = "1" ]; then
  echo "Built $BUNDLE (v$VERSION)"
  exit 0
fi
rm -rf "$DEST"
mkdir -p "$(dirname "$DEST")"
cp -R "$BUNDLE" "$DEST"
echo "Installed $DEST (v$VERSION)"
