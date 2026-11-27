#!/usr/bin/env bash
# Builds the Mesen-S libretro core (SNES) with the decomp-dj connector.
# Usage: connectors/mesen-s/build.sh [output-dir]
# Needs git, make and a C++11 compiler.
set -euo pipefail

REPO="https://github.com/libretro/Mesen-S.git"
COMMIT="9e4fdeb9b336470bc96beb8765b2e79c86a2da1e"
CLEAN="Core Libretro"
ID_HEADER="Core/DjBuildId.h"
BUILT="Libretro/mesen-s_libretro"

compile() {
  make -C Libretro -j"$jobs"
}

. "$(dirname "$0")/../build-core.sh"
