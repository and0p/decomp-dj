#!/usr/bin/env bash
# Builds Genesis Plus GX's libretro core with the decomp-dj connector.
# Usage: connectors/genesis-plus-gx/build.sh [output-dir]
# Needs git, make and a C compiler.
set -euo pipefail

GPGX_REPO="https://github.com/libretro/Genesis-Plus-GX.git"
GPGX_COMMIT="51348f65738b0c75f4b8b4488fbc67645e3769af"

here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/../../build/cores}"
work="$here/../../build/genesis-plus-gx-src"

mkdir -p "$out"
if [ ! -d "$work/.git" ]; then
  git clone "$GPGX_REPO" "$work"
fi
cd "$work"
git fetch --depth 1 origin "$GPGX_COMMIT" 2>/dev/null || true
git checkout -q --force "$GPGX_COMMIT"
git clean -qfd core libretro
git apply "$here/dj-genesis-plus-gx.patch"
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
patch_id="$({ cat "$here/dj-genesis-plus-gx.patch"; for f in $own; do cat "$here/src/$f"; done; } | tr -d '\r' | { shasum -a 256 2>/dev/null || sha256sum; } | cut -c1-8)"
printf '#define DJ_BUILD_ID "genesis-plus-gx@%s+dj.%s"\n' "${GPGX_COMMIT:0:8}" "$patch_id" > libretro/dj_build_id.h

jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
# The Mega CD's disc formats (CHD) aren't built: dj drives cartridges only. Nothing else
# depends on what the machine has installed (zlib is the core's own copy).
# Cleaned first: the makefile has no header dependencies, and the patch changes headers.
make -f Makefile.libretro HAVE_CHD=0 clean >/dev/null
make -f Makefile.libretro -j"$jobs" HAVE_CHD=0 >/dev/null

for f in genesis_plus_gx_libretro.so genesis_plus_gx_libretro.dylib genesis_plus_gx_libretro.dll; do
  if [ -f "$f" ]; then
    # A new file moved into place: macOS kills a process that loads a library whose file
    # was overwritten in place (Code Signature Invalid).
    cp "$f" "$out/.$f.new"
    mv -f "$out/.$f.new" "$out/$f"
    echo "built $out/$f"
  fi
done
