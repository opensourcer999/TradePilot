#!/bin/bash
# Builds TradePilot for Linux (x86_64) and packs it as a tar.gz in dist/: program, launcher (.desktop), icon, short guide.
#
# Usage (on Linux):
#   ./package-linux.sh              # build the release and pack it
#   ./package-linux.sh --no-build   # use the existing release binary
#
# The package contains only the program, never the user's data (~/.local/share/TradeWorkspace).
set -euo pipefail
cd "$(dirname "$0")"

build=1
for arg in "$@"; do
    case "$arg" in
        --no-build) build=0 ;;
        *) echo "Unknown option: $arg" >&2; exit 2 ;;
    esac
done

target_root="${CARGO_TARGET_DIR:-$PWD/target}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' trade_pilot/Cargo.toml | head -1)"
[ -n "$version" ] || { echo "Version not found." >&2; exit 1; }

[ "$build" = 1 ] && cargo build --release
binary="$target_root/release/trade_pilot"
[ -f "$binary" ] || { echo "Binary missing: $binary" >&2; exit 1; }

name="TradePilot-$version-linux-x86_64"
dist="$PWD/dist"
staging="$dist/staging/$name"
rm -rf "$dist/staging"
mkdir -p "$staging"
cp "$binary" "$staging/TradePilot"
strip "$staging/TradePilot" || true
chmod +x "$staging/TradePilot"
cp trade_pilot/assets/icon.png "$staging/TradePilot.png"
cp installer/README-Linux.txt "$staging/README.txt"
cat > "$staging/TradePilot.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=TradePilot
Comment=Options trading with TWS / IB Gateway
Exec=TradePilot
Icon=TradePilot
Terminal=false
Categories=Office;Finance;
EOF

rm -f "$dist/$name.tar.gz"
tar -C "$dist/staging" -czf "$dist/$name.tar.gz" "$name"
rm -rf "$dist/staging"
echo "Done: $dist/$name.tar.gz"
