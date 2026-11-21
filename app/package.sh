#!/bin/bash
# Builds TradePilot for macOS and packs it as TradePilot.app into a DMG and a ZIP (dist/).
#
# Usage (on a Mac):
#   ./package.sh              # build for the architecture of this Mac
#   ./package.sh universal    # universal binary (Apple Silicon + Intel)
#   ./package.sh --no-build   # use the existing release binary
#
# The package is not signed: on the first start, right-click TradePilot.app > Open.
# It contains only the program, never the user's data (~/Library/Application Support/TradeWorkspace).
set -euo pipefail
cd "$(dirname "$0")"

universal=0
build=1
for arg in "$@"; do
    case "$arg" in
        universal) universal=1 ;;
        --no-build) build=0 ;;
        *) echo "Unknown option: $arg" >&2; exit 2 ;;
    esac
done

target_root="${CARGO_TARGET_DIR:-$PWD/target}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' trade_pilot/Cargo.toml | head -1)"
[ -n "$version" ] || { echo "Version not found." >&2; exit 1; }

if [ "$universal" = 1 ]; then
    if [ "$build" = 1 ]; then
        rustup target add aarch64-apple-darwin x86_64-apple-darwin
        cargo build --release --target aarch64-apple-darwin
        cargo build --release --target x86_64-apple-darwin
    fi
    binary="$target_root/universal/TradePilot"
    mkdir -p "$target_root/universal"
    lipo -create -output "$binary" \
        "$target_root/aarch64-apple-darwin/release/trade_pilot" \
        "$target_root/x86_64-apple-darwin/release/trade_pilot"
    arch_label="universal"
else
    [ "$build" = 1 ] && cargo build --release
    binary="$target_root/release/trade_pilot"
    arch_label="$(uname -m)"
fi
[ -f "$binary" ] || { echo "Binary missing: $binary" >&2; exit 1; }

dist="$PWD/dist"
app="$dist/staging/TradePilot.app"
rm -rf "$dist/staging"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/TradePilot"
chmod +x "$app/Contents/MacOS/TradePilot"
sed "s/@VERSION@/$version/g" installer/Info.plist > "$app/Contents/Info.plist"

# App icon: an .icns built from the 256 px PNG.
iconset="$dist/staging/TradePilot.iconset"
mkdir -p "$iconset"
png="trade_pilot/assets/icon.png"
for size in 16 32 64 128 256; do
    sips -z "$size" "$size" "$png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
done
cp "$iconset/icon_32x32.png"   "$iconset/icon_16x16@2x.png"
cp "$iconset/icon_64x64.png"   "$iconset/icon_32x32@2x.png"
cp "$iconset/icon_256x256.png" "$iconset/icon_128x128@2x.png"
rm "$iconset/icon_64x64.png"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/TradePilot.icns"
rm -rf "$iconset"

# Ad-hoc signature: required for the binary to start on Apple Silicon at all.
codesign --force --deep --sign - "$app"

name="TradePilot-$version-macos-$arch_label"
cp installer/README-macOS.txt "$dist/staging/README.txt"
rm -f "$dist/$name.zip" "$dist/$name.dmg"
(cd "$dist/staging" && ditto -c -k --keepParent TradePilot.app "$dist/$name.zip")
hdiutil create -volname "TradePilot" -srcfolder "$dist/staging" -ov -format UDZO "$dist/$name.dmg" >/dev/null
rm -rf "$dist/staging"
echo "Done: $dist/$name.dmg and $dist/$name.zip"
