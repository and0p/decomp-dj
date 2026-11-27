#!/usr/bin/env bash
# Builds mGBA's libretro core with the decomp-dj connector.
# Usage: connectors/mgba/build.sh [output-dir]
# Needs git, cmake, make and a C compiler. Optional libraries are all turned off, so the
# core is the same whatever is installed.
set -euo pipefail

REPO="https://github.com/libretro/mgba.git"
COMMIT="7a12d6d4b9acb14c0ae62c9166b6a2f3d08007f6"
CLEAN="src include"
ID_HEADER="src/platform/libretro/dj_build_id.h"
BUILT="build-dj/mgba_libretro"

compile() {
  cmake -S . -B build-dj -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_LIBRETRO=ON -DSKIP_LIBRARY=ON -DBUILD_QT=OFF -DBUILD_SDL=OFF -DBUILD_GL=OFF \
    -DBUILD_GLES2=OFF -DBUILD_GLES3=OFF -DUSE_EPOXY=OFF -DBUILD_SUITE=OFF -DBUILD_CINEMA=OFF \
    -DENABLE_DEBUGGERS=OFF -DENABLE_GDB_STUB=OFF -DUSE_EDITLINE=OFF -DENABLE_SCRIPTING=OFF \
    -DUSE_FFMPEG=OFF -DUSE_ZLIB=OFF -DUSE_MINIZIP=OFF -DUSE_PNG=OFF -DUSE_LIBZIP=OFF \
    -DUSE_SQLITE3=OFF -DUSE_ELF=OFF -DUSE_LUA=OFF -DUSE_JSON_C=OFF -DUSE_FREETYPE=OFF \
    -DUSE_LZMA=OFF -DUSE_DISCORD_RPC=OFF >/dev/null
  cmake --build build-dj --target mgba_libretro -j"$jobs"
}

. "$(dirname "$0")/../build-core.sh"
