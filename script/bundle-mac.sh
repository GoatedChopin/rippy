#!/bin/sh
# Builds dist/Rippy.app with its icon. Signed ad hoc, so it is for this Mac, not for sharing.
set -eu
cd "$(dirname "$0")/.."

cargo build --release
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=dist/Rippy.app

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/rippy "$app/Contents/MacOS/rippy"
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
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
echo "Built $app"
