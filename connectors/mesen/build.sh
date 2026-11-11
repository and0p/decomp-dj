#!/usr/bin/env bash
# Builds the Mesen libretro core with the decomp-dj connector.
# Usage: connectors/mesen/build.sh [output-dir]
# Needs git, make and a C++11 compiler. On Windows, run it from MSYS2.
set -euo pipefail

MESEN_REPO="https://github.com/libretro/Mesen.git"
MESEN_COMMIT="0102910c39ad1a62bc3f784466f3f67ca9eae335"

here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/../../build/cores}"
work="$here/../../build/mesen-src"

mkdir -p "$out"
if [ ! -d "$work/.git" ]; then
  git clone "$MESEN_REPO" "$work"
fi
cd "$work"
git fetch --depth 1 origin "$MESEN_COMMIT" 2>/dev/null || true
git checkout -q --force "$MESEN_COMMIT"
git clean -qfd Core Libretro
git apply "$here/dj-mesen.patch"

# The build id the connector reports (dj_build_id): the upstream commit and this patch, line
# endings aside, so the same source gives the same id on every platform.
patch_id="$(tr -d '\r' < "$here/dj-mesen.patch" | { shasum -a 256 2>/dev/null || sha256sum; } | cut -c1-8)"
printf '#define DJ_BUILD_ID "mesen@%s+dj.%s"\n' "${MESEN_COMMIT:0:8}" "$patch_id" > Core/DjBuildId.h

jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
make -C Libretro -j"$jobs"

for f in Libretro/mesen_libretro.so Libretro/mesen_libretro.dylib Libretro/mesen_libretro.dll; do
  if [ -f "$f" ]; then
    # Copy to a new file and move it into place. Overwriting the old file in place
    # keeps its inode, and macOS then kills any process that loads it (Code Signature
    # Invalid), because it cached the old file's signature.
    cp "$f" "$out/.$(basename "$f").new"
    mv -f "$out/.$(basename "$f").new" "$out/$(basename "$f")"
    echo "built $out/$(basename "$f")"
  fi
done
