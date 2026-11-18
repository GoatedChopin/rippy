#!/bin/bash
# Builds Rippy for Mac (one app for Apple silicon and Intel, signed ad hoc) and uploads
# Rippy-<version>-macos.zip to the GitHub release.
#   ./script/release-mac.sh             build, zip, upload
#   ./script/release-mac.sh --dry-run   build and zip only
# The app isn't notarized, so the first time someone opens it, macOS asks them to allow it in
# System Settings > Privacy & Security. Updates after that install without asking.
set -euo pipefail
cd "$(dirname "$0")/.."
. script/release-common.sh

dry=0
[ "${1:-}" = --dry-run ] && dry=1
[ $dry = 1 ] || require_pushed

./script/bundle-mac.sh --release
app=dist/Rippy.app
archs=$(lipo -archs "$app/Contents/MacOS/rippy")
[[ "$archs" == *arm64* && "$archs" == *x86_64* ]] || fail "the app isn't universal ($archs)"

zip="Rippy-$version-macos.zip"
rm -f "dist/$zip"
# ditto keeps the bundle's symlinks and signature, which the updater checks.
ditto -c -k --keepParent "$app" "dist/$zip"
(cd dist && shasum -a 256 "$zip" > "$zip.sha256")
echo "Built dist/$zip ($(cut -d' ' -f1 "dist/$zip.sha256"))"

if [ $dry = 1 ]; then
  echo "Dry run: nothing uploaded."
  exit 0
fi
upload "dist/$zip" "dist/$zip.sha256"
