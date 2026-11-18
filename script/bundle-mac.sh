#!/bin/sh
# Builds dist/Rippy.app with its icon.
#   ./script/bundle-mac.sh             this Mac's CPU, signed ad hoc: for trying Rippy on this Mac
#   ./script/bundle-mac.sh --release   one app for Apple silicon and Intel, for release-mac.sh
# Both are signed ad hoc. RIPPY_SIGN_IDENTITY="Developer ID Application" signs a --release build
# for notarizing instead.
set -eu
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=dist/Rippy.app
# Rippy runs on macOS 11 and later.
export MACOSX_DEPLOYMENT_TARGET=11.0

if [ "${1:-}" = --release ]; then
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked --target "$target"
  done
  mkdir -p dist
  binary=dist/rippy-universal
  lipo -create -output "$binary" \
    target/aarch64-apple-darwin/release/rippy target/x86_64-apple-darwin/release/rippy
  identity="${RIPPY_SIGN_IDENTITY:--}"
else
  cargo build --release
  binary=target/release/rippy
  identity=-
fi

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/rippy"
cp assets/icon/rippy.icns "$app/Contents/Resources/rippy.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Rippy</string>
  <key>CFBundleDisplayName</key><string>Rippy</string>
  <key>CFBundleIdentifier</key><string>biz.meline.Rippy</string>
  <key>CFBundleExecutable</key><string>rippy</string>
  <key>CFBundleIconFile</key><string>rippy</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$app/Contents/Info.plist"

if [ "$identity" = - ]; then
  codesign --force --sign - "$app"
else
  # Notarization needs the hardened runtime and a secure timestamp.
  codesign --force --timestamp --options runtime --sign "$identity" "$app"
fi
codesign --verify --deep --strict "$app"
echo "Built $app"
