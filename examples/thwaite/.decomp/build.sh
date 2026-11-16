#!/usr/bin/env bash
# Builds the Thwaite decomp's libretro core into .decomp/build/.
# Needs CMake and a C compiler. The data tables are committed; tools/ regenerates them.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cmake -S "$here" -B "$here/build" -DCMAKE_BUILD_TYPE="${BUILD_TYPE:-Release}"
cmake --build "$here/build" --parallel
ls "$here"/build/thwaite_decomp_libretro.*
