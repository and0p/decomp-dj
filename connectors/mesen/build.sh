#!/usr/bin/env bash
# Builds the Mesen libretro core with the decomp-dj connector.
# Usage: connectors/mesen/build.sh [output-dir]
# Needs git, make and a C++11 compiler. On Windows, run it from MSYS2.
set -euo pipefail

REPO="https://github.com/libretro/Mesen.git"
COMMIT="0102910c39ad1a62bc3f784466f3f67ca9eae335"
CLEAN="Core Libretro"
ID_HEADER="Core/DjBuildId.h"
BUILT="Libretro/mesen_libretro"

compile() {
  make -C Libretro -j"$jobs"
}

. "$(dirname "$0")/../build-core.sh"
