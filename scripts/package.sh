#!/usr/bin/env bash
# Assembles decomp-dj's portable folder and ZIP for this system from release builds:
#   dist/decomp-dj-<version>-<os>-<arch>/   and   dist/decomp-dj-<version>-<os>-<arch>.zip
# Nothing to install: unzip and run. Used by CI (.github/workflows) and locally.
#
# Usage: scripts/package.sh [--build]     --build: cargo build --release first
# Needs bash (Git Bash on Windows) and zip, 7z or PowerShell.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' player/Cargo.toml | head -1)"
case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  MINGW* | MSYS* | CYGWIN* | Windows_NT) os=windows ;;
  *) echo "unknown system $(uname -s)"; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) arch="$(uname -m)" ;;
esac
exe=""; lib="libdj_reference_decoders.so"
[ "$os" = windows ] && { exe=".exe"; lib="dj_reference_decoders.dll"; }
[ "$os" = macos ] && lib="libdj_reference_decoders.dylib"

if [ "${1:-}" = "--build" ]; then
  (cd player && cargo build --release --locked)
  (cd gui && cargo build --release --locked)
  (cd decoders/reference && cargo build --release --locked)
fi

name="decomp-dj-$version-$os-$arch"
out="dist/$name"
rm -rf "$out" "dist/$name.zip"
mkdir -p "$out"

# The programs. On macOS the GUI is an app bundle (its icon in Finder and the Dock).
cp "player/target/release/dj$exe" "$out/"
if [ "$os" = macos ]; then
  gui/macos/bundle.sh --no-build >/dev/null
  cp -R gui/target/release/decomp-dj.app "$out/"
else
  cp "gui/target/release/dj-gui$exe" "$out/"
fi

# Decoder plugins (docs/decoders.md): the C header, and the reference plugin as an example
# to load. A bare [[decoders]] path is looked for here, next to dj.
mkdir -p "$out/decoders"
cp decoders/include/dj_decoder.h "$out/decoders/"
cp "decoders/reference/target/release/$lib" "$out/decoders/"

# Connectors: the patches and build scripts that make a core report coverage. Cores aren't
# shipped; connectors/README.md says how to build one.
mkdir -p "$out/connectors"
for d in connectors/*/; do
  mkdir -p "$out/$d"
  cp "$d"*.patch "$d"*.sh "$d"README.md "$out/$d" 2>/dev/null || true
done
cp connectors/README.md "$out/connectors/"

# Linux: the desktop entry, so launchers and Wayland desktops show the icon.
if [ "$os" = linux ]; then
  mkdir -p "$out/linux"
  cp gui/linux/decomp-dj.desktop gui/linux/install-desktop-entry.sh "$out/linux/"
  cp assets/icon_gui.png "$out/linux/decomp-dj.png"
fi

cp -R docs "$out/"
cp LICENSE.txt "$out/LICENSE.txt"
cp README.md "$out/README.md"

# The ZIP, keeping executable bits and the app bundle's links where the tool can.
(
  cd dist
  if command -v zip >/dev/null; then
    zip -qry "$name.zip" "$name"
  elif command -v 7z >/dev/null; then
    7z a -tzip -bd "$name.zip" "$name" >/dev/null
  else
    powershell.exe -NoProfile -Command "Compress-Archive -Path '$name' -DestinationPath '$name.zip'"
  fi
)
sum="$( (shasum -a 256 "dist/$name.zip" 2>/dev/null || sha256sum "dist/$name.zip") | cut -d' ' -f1)"
echo "packaged dist/$name.zip ($(du -h "dist/$name.zip" | cut -f1), sha256 $sum)"
