#!/usr/bin/env bash
# Builds the Mesen-S libretro core (SNES) with the decomp-dj connector.
# Usage: connectors/mesen-s/build.sh [output-dir]
# Needs git, make and a C++11 compiler.
set -euo pipefail

MESENS_REPO="https://github.com/libretro/Mesen-S.git"
MESENS_COMMIT="9e4fdeb9b336470bc96beb8765b2e79c86a2da1e"

here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/../../build/cores}"
work="$here/../../build/mesen-s-src"

mkdir -p "$out"
if [ ! -d "$work/.git" ]; then
  git clone "$MESENS_REPO" "$work"
fi
cd "$work"
git fetch --depth 1 origin "$MESENS_COMMIT" 2>/dev/null || true
git checkout -q --force "$MESENS_COMMIT"
git clean -qfd Core Libretro
git apply "$here/dj-mesen-s.patch"
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
patch_id="$({ cat "$here/dj-mesen-s.patch"; for f in $own; do cat "$here/src/$f"; done; } | tr -d '\r' | { shasum -a 256 2>/dev/null || sha256sum; } | cut -c1-8)"
printf '#define DJ_BUILD_ID "mesen-s@%s+dj.%s"\n' "${MESENS_COMMIT:0:8}" "$patch_id" > Core/DjBuildId.h

jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
make -C Libretro -j"$jobs"

for f in Libretro/mesen-s_libretro.so Libretro/mesen-s_libretro.dylib Libretro/mesen-s_libretro.dll; do
  if [ -f "$f" ]; then
    # Copy to a new file and move it into place. Overwriting the old file in place keeps
    # its inode, and macOS then kills any process that loads it (Code Signature Invalid),
    # because it cached the old file's signature.
    cp "$f" "$out/.$(basename "$f").new"
    mv -f "$out/.$(basename "$f").new" "$out/$(basename "$f")"
    echo "built $out/$(basename "$f")"
  fi
done
