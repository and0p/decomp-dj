#!/usr/bin/env bash
# Builds SameBoy's libretro core (Game Boy, Game Boy Color) with the decomp-dj connector.
# Usage: connectors/sameboy/build.sh [output-dir]
# Needs git, make, hexdump and a C compiler. The boot ROMs come prebuilt with the pinned
# release (SameBoy's own, from its sources), so RGBDS isn't needed.
set -euo pipefail

SAMEBOY_REPO="https://github.com/LIJI32/SameBoy.git"
# The v1.0.5-libretro tag: the v1.0.5 release plus its prebuilt boot ROMs, which is what
# libretro's own builds use.
SAMEBOY_COMMIT="90e2899574b7b63c59dce8a7276783426d52a068"

here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/../../build/cores}"
work="$here/../../build/sameboy-src"

mkdir -p "$out"
if [ ! -d "$work/.git" ]; then
  git clone "$SAMEBOY_REPO" "$work"
fi
cd "$work"
git fetch --depth 1 origin "$SAMEBOY_COMMIT" 2>/dev/null || true
git checkout -q --force "$SAMEBOY_COMMIT"
git clean -qfdx Core libretro build
git apply "$here/dj-sameboy.patch"
# decomp-dj's own files (MIT, src/LICENSE.txt), laid out as in the core's tree. The patch
# above only hooks them into the core's files.
own="$(cd "$here/src" && find . -type f ! -name LICENSE.txt | LC_ALL=C sort)"
for f in $own; do
  mkdir -p "$(dirname "$f")"
  cp "$here/src/$f" "$f"
done

# The build id the connector reports (dj_build_id): the upstream commit, then the patch and
# the files under src/ in that order, line endings aside, so the same source gives the same
# id on every platform.
patch_id="$({ cat "$here/dj-sameboy.patch"; for f in $own; do cat "$here/src/$f"; done; } | tr -d '\r' | { shasum -a 256 2>/dev/null || sha256sum; } | cut -c1-8)"
printf '#define DJ_BUILD_ID "sameboy@%s+dj.%s"\n' "${SAMEBOY_COMMIT:0:8}" "$patch_id" > libretro/dj_build_id.h

jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
make -C libretro -j"$jobs" BOOTROMS_DIR="$work/BootROMs/prebuilt" >/dev/null

for f in sameboy_libretro.so sameboy_libretro.dylib sameboy_libretro.dll; do
  if [ -f "$f" ]; then
    # A new file moved into place: macOS kills a process that loads a library whose file
    # was overwritten in place (Code Signature Invalid).
    cp "$f" "$out/.$f.new"
    mv -f "$out/.$f.new" "$out/$f"
    echo "built $out/$f"
  fi
done
