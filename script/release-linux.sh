#!/bin/bash
# Builds Rippy for Linux in Docker and uploads Rippy-<version>-linux-<arch>.zip to the release.
#   ./script/release-linux.sh                  x64: build, zip, upload
#   ./script/release-linux.sh --dry-run        build and zip only
#   ./script/release-linux.sh --arch arm64     64-bit Arm instead
# On an Apple silicon Mac the x64 build runs under emulation, so it takes a while.
set -euo pipefail
cd "$(dirname "$0")/.."
. script/release-common.sh

dry=0
arch=x64
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry=1 ;;
    --arch) arch="${2:-}"; shift ;;
    *) fail "unknown option $1" ;;
  esac
  shift
done
case "$arch" in
  x64) platform=linux/amd64 ;;
  arm64) platform=linux/arm64 ;;
  *) fail "--arch is x64 or arm64" ;;
esac
[ $dry = 1 ] || require_pushed
command -v docker >/dev/null || fail "Docker isn't installed"

image="rippy-linux-build:$arch"
docker build --platform "$platform" -t "$image" script/linux

out="dist/linux-$arch"
rm -rf "$out"
mkdir -p "$out/Rippy"
# The source is mounted read-only; the build and Cargo's downloads live in Docker volumes.
docker run --rm --platform "$platform" \
  -v "$PWD":/src:ro -v "$PWD/$out":/out \
  -v "rippy-linux-target-$arch":/target -v "rippy-linux-cargo-$arch":/usr/local/cargo/registry \
  -w /src "$image" \
  sh -c 'cargo build --release --locked --target-dir /target \
    && cp /target/release/rippy /out/Rippy/rippy \
    && objdump -T /out/Rippy/rippy | grep -o "GLIBC_[0-9.]*" | sort -uV | tail -1 > /out/glibc'
echo "Needs $(cat "$out/glibc") or newer"

zip="Rippy-$version-linux-$arch.zip"
rm -f "dist/$zip"
(cd "$out" && zip -q -X -r "../$zip" Rippy)
(cd dist && shasum -a 256 "$zip" > "$zip.sha256")
echo "Built dist/$zip ($(cut -d' ' -f1 "dist/$zip.sha256"))"

if [ $dry = 1 ]; then
  echo "Dry run: nothing uploaded."
  exit 0
fi
upload "dist/$zip" "dist/$zip.sha256"
