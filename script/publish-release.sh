#!/bin/bash
# Signs and publishes a release once its downloads are up: run it on the Mac after
# release-mac.sh, release-linux.sh and, on the PC, release-windows.ps1. It downloads each zip
# from the release, checks it, writes rippy-update.json signed with the update key from the
# login Keychain, uploads it, and makes the release the latest, which is when Rippy sees it.
#   ./script/publish-release.sh             sign, upload, publish
#   ./script/publish-release.sh --dry-run   check and sign into dist/, upload nothing
set -euo pipefail
cd "$(dirname "$0")/.."
. script/release-common.sh

dry=0
[ "${1:-}" = --dry-run ] && dry=1
[ $dry = 1 ] || require_pushed
# The feed has to be checked against the key that real builds carry, not a test key.
[ -z "${RIPPY_UPDATE_PUBLIC_KEY:-}" ] || fail "unset RIPPY_UPDATE_PUBLIC_KEY first"
grep -q 'None => "[A-Za-z0-9+/=]\{44\}"' src/feed.rs || fail "src/feed.rs has no update key yet"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
gh release download "$tag" --repo "$REPO" --dir "$work" --pattern 'Rippy-*.zip*'

# Each target, the zip it comes in, and the file the updater takes out of it.
required="macos windows-x64 linux-x64"
pairs=()
for target in macos windows-x64 windows-arm64 linux-x64 linux-arm64; do
  zip="Rippy-$version-$target.zip"
  case "$target" in
    macos) inside="Rippy.app/Contents/MacOS/rippy" ;;
    windows-*) inside="Rippy.exe" ;;
    linux-*) inside="Rippy/rippy" ;;
  esac
  if [ ! -f "$work/$zip" ]; then
    [[ " $required " == *" $target "* ]] && fail "the release has no $zip yet"
    continue
  fi
  [ -f "$work/$zip.sha256" ] || fail "the release has no $zip.sha256"
  (cd "$work" && shasum -a 256 -c "$zip.sha256" >/dev/null) || fail "$zip doesn't match its .sha256"
  # Windows' Compress-Archive writes backslashes, so only the file name is certain there.
  unzip -Z1 "$work/$zip" 2>/dev/null | tr '\\' '/' | grep -q "$inside\$" || fail "$zip has no $inside"
  pairs+=("$target=$work/$zip")
  echo "Checked $zip"
done

cargo build --release --locked --example update_feed
feed_tool=target/release/examples/update_feed
feed="$work/rippy-update.json"
security find-generic-password -s "Rippy update key" -a rippy -w \
  | "$feed_tool" sign "$version" "https://github.com/$REPO/releases/download/$tag" "${pairs[@]}" > "$feed"
"$feed_tool" verify "$feed" "${pairs[@]}"

if [ $dry = 1 ]; then
  cp "$feed" dist/rippy-update.json
  echo "Dry run: signed dist/rippy-update.json, uploaded nothing."
  exit 0
fi
gh release upload "$tag" "$feed" --clobber --repo "$REPO"
gh release edit "$tag" --repo "$REPO" --prerelease=false --latest
echo "Published https://github.com/$REPO/releases/tag/$tag"

# What copies of Rippy will read. A private repo answers 404 to them.
latest="https://github.com/$REPO/releases/latest/download/rippy-update.json"
if curl -fsSL "$latest" -o "$work/latest.json" 2>/dev/null; then
  "$feed_tool" verify "$work/latest.json" | head -1
else
  echo "warning: $latest isn't reachable without signing in, so copies of Rippy can't update from it" >&2
fi
