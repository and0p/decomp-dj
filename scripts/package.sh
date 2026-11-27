#!/usr/bin/env bash
# Assembles decomp-dj's portable folder and ZIP for this system from release builds:
#   dist/decomp-dj-<version>-<os>-<arch>/   and   dist/decomp-dj-<version>-<os>-<arch>.zip
# Nothing to install: unzip and run. Used by CI (.github/workflows) and locally.
#
# Usage: scripts/package.sh [--build] [--cores]
#   --build   cargo build --release first
#   --cores   build the cores that go in the ZIP and aren't in build/cores/ yet. Without it,
#             the ZIP carries the ones already built there, and says which are missing
# Needs bash (Git Bash on Windows), Python 3.11 or later for the list of cores, and zip, 7z
# or PowerShell.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
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

build_cores=""
for arg in "$@"; do
  case "$arg" in
    --build) cargo build --release --locked ;;
    --cores) build_cores="--build" ;;
    *) echo "unknown option $arg"; exit 1 ;;
  esac
done
python="$(command -v python3 || command -v python)"

name="decomp-dj-$version-$os-$arch"
out="dist/$name"
rm -rf "$out" "dist/$name.zip"
mkdir -p "$out"

# The programs. On macOS the GUI is an app bundle (its icon in Finder and the Dock).
cp "target/release/dj$exe" "$out/"
if [ "$os" = macos ]; then
  crates/dj-gui/macos/bundle.sh --no-build >/dev/null
  cp -R target/release/decomp-dj.app "$out/"
else
  cp "target/release/dj-gui$exe" "$out/"
fi

# Decoder plugins (docs/decoders.md): the reference plugin as an example to load. A bare
# [[decoders]] path is looked for here, next to dj.
mkdir -p "$out/decoders"
cp "target/release/$lib" "$out/decoders/"

# Cores: the ones connectors/cores.toml lists for bundling, built from the connectors below.
# dj looks for a core named without a folder here, next to itself (docs/config.md). Each
# has its licence text beside it, and its source is a tarball on the same release
# (scripts/cores.py sources).
mkdir -p "$out/cores"
cat > "$out/cores/README.txt" <<NOTE
libretro cores built with decomp-dj's connectors (../connectors/), which make a core report
the code it runs. dj finds a core named without a folder here: \`mesen_libretro\` in a
project's config.toml, or \`dj init --core mesen_libretro\`.

These are separate programs from decomp-dj, each under its own licence: the file
<core>.LICENSE.txt beside it. Each core's complete source, as built, is on the release this
ZIP came from, in the file named below; ../connectors/<name>/build.sh builds it from the
same commit.

NOTE
"$python" scripts/cores.py bundle $build_cores | while read -r cname clib cid; do
  for f in "build/cores/$clib".so "build/cores/$clib".dylib "build/cores/$clib".dll; do
    [ -f "$f" ] && cp "$f" "$out/cores/"
  done
  cp "connectors/$cname/LICENSE.txt" "$out/cores/$clib.LICENSE.txt"
  printf '%s\n  build   %s\n  source  %s-source.tar.gz\n' "$clib" "$cid" "$(echo "$cid" | sed 's/@/-/; s/+/-/')" >> "$out/cores/README.txt"
done

# Connectors: the patches and build scripts that make a core report coverage, for building
# a core yourself; connectors/README.md says how.
mkdir -p "$out/connectors"
for d in connectors/*/; do
  mkdir -p "$out/$d"
  cp "$d"*.patch "$d"*.sh "$d"README.md "$d"LICENSE.txt "$out/$d" 2>/dev/null || true
  # The connector's own files (MIT), which build.sh copies into the core.
  if [ -d "${d}src" ]; then cp -R "${d}src" "$out/$d"; fi
done
cp connectors/README.md connectors/build-core.sh "$out/connectors/"
# The C headers: decoder plugins, connectors and comparison cores.
mkdir -p "$out/include"
cp include/*.h "$out/include/"

# Linux: the desktop entry, so launchers and Wayland desktops show the icon.
if [ "$os" = linux ]; then
  mkdir -p "$out/linux"
  cp crates/dj-gui/linux/decomp-dj.desktop crates/dj-gui/linux/install-desktop-entry.sh "$out/linux/"
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
