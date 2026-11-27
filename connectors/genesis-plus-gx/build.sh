#!/usr/bin/env bash
# Builds Genesis Plus GX's libretro core with the decomp-dj connector.
# Usage: connectors/genesis-plus-gx/build.sh [output-dir]
# Needs git, make and a C compiler.
set -euo pipefail

REPO="https://github.com/libretro/Genesis-Plus-GX.git"
COMMIT="51348f65738b0c75f4b8b4488fbc67645e3769af"
CLEAN="core libretro"
ID_HEADER="libretro/dj_build_id.h"
BUILT="genesis_plus_gx_libretro"

compile() {
  # The Mega CD's disc formats (CHD) aren't built: dj drives cartridges only. Nothing else
  # depends on what the machine has installed (zlib is the core's own copy).
  # Cleaned first: the makefile has no header dependencies, and the patch changes headers.
  make -f Makefile.libretro HAVE_CHD=0 clean >/dev/null
  make -f Makefile.libretro -j"$jobs" HAVE_CHD=0 >/dev/null
}

. "$(dirname "$0")/../build-core.sh"
