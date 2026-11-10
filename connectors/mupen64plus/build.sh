#!/usr/bin/env bash
# Builds the mupen64plus-libretro-nx (Mupen64Plus-Next) core with the decomp-dj connector.
# Usage: connectors/mupen64plus/build.sh [output-dir]
# Needs git, make, a C/C++ compiler, pkg-config, zlib and libpng (Homebrew's on macOS).
set -euo pipefail

REPO="https://github.com/libretro/mupen64plus-libretro-nx.git"
COMMIT="15bada85906593376e8fd697f1b0b3af3a20f512"

here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/../../build/cores}"
work="$here/../../build/mupen64plus-src"

mkdir -p "$out"
if [ ! -d "$work/.git" ]; then
  git clone "$REPO" "$work"
fi
cd "$work"
git fetch --depth 1 origin "$COMMIT" 2>/dev/null || true
git checkout -q --force "$COMMIT"
git clean -qfd libretro mupen64plus-core
git apply "$here/dj-mupen64plus.patch"

# The build id the connector reports (dj_build_id): the upstream commit and this patch, line
# endings aside, so the same source gives the same id on every platform.
patch_id="$(tr -d '\r' < "$here/dj-mupen64plus.patch" | { shasum -a 256 2>/dev/null || sha256sum; } | cut -c1-8)"
printf '#define DJ_BUILD_ID "mupen64plus@%s+dj.%s"\n' "${COMMIT:0:8}" "$patch_id" > libretro/dj_build_id.h

# The bundled zlib and libpng take macOS for classic Mac OS (fp.h, fdopen), so build
# against the system's copies.
jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
make -j"$jobs" SYSTEM_ZLIB=1 SYSTEM_LIBPNG=1

for f in mupen64plus_next_libretro.so mupen64plus_next_libretro.dylib mupen64plus_next_libretro.dll; do
  if [ -f "$f" ]; then
    # Copy to a new file and move it into place (macOS kills processes that load a library
    # overwritten in place, see connectors/mesen/build.sh).
    cp "$f" "$out/.$(basename "$f").new"
    mv -f "$out/.$(basename "$f").new" "$out/$(basename "$f")"
    echo "built $out/$(basename "$f")"
  fi
done
