#!/usr/bin/env bash
# Builds SameBoy's libretro core (Game Boy, Game Boy Color) with the decomp-dj connector.
# Usage: connectors/sameboy/build.sh [output-dir]
# Needs git, make, hexdump and a C compiler. The boot ROMs come prebuilt with the pinned
# release (SameBoy's own, from its sources), so RGBDS isn't needed.
set -euo pipefail

REPO="https://github.com/LIJI32/SameBoy.git"
# The v1.0.5-libretro tag: the v1.0.5 release plus its prebuilt boot ROMs, which is what
# libretro's own builds use.
COMMIT="90e2899574b7b63c59dce8a7276783426d52a068"
CLEAN="Core libretro build"
CLEAN_IGNORED=1
ID_HEADER="libretro/dj_build_id.h"
BUILT="sameboy_libretro"

compile() {
  make -C libretro -j"$jobs" BOOTROMS_DIR="$PWD/BootROMs/prebuilt" >/dev/null
}

. "$(dirname "$0")/../build-core.sh"
