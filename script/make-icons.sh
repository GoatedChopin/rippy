#!/bin/sh
# Renders assets/icon/*.svg into rippy.icns (macOS) and rippy.ico (Windows).
# Needs rsvg-convert and ImageMagick (brew install librsvg imagemagick); iconutil ships with macOS.
set -eu
cd "$(dirname "$0")/../assets/icon"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir "$work/rippy.iconset"
for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" rippy.svg -o "$work/rippy.iconset/icon_${size}x${size}.png"
    rsvg-convert -w $((size * 2)) -h $((size * 2)) rippy.svg -o "$work/rippy.iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$work/rippy.iconset" -o rippy.icns

for size in 16 20 24 32 40 48 64 256; do
    rsvg-convert -w "$size" -h "$size" rippy-windows.svg -o "$work/win-$size.png"
done
magick "$work"/win-16.png "$work"/win-20.png "$work"/win-24.png "$work"/win-32.png \
    "$work"/win-40.png "$work"/win-48.png "$work"/win-64.png "$work"/win-256.png rippy.ico

rsvg-convert -w 1024 -h 1024 rippy.svg -o rippy.png
echo "Wrote rippy.icns, rippy.ico and rippy.png"
