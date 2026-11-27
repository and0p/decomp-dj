#!/usr/bin/env bash
# Builds the mupen64plus-libretro-nx (Mupen64Plus-Next) core with the decomp-dj connector.
# Usage: connectors/mupen64plus/build.sh [output-dir]
# Needs git, make, a C/C++ compiler, pkg-config, zlib and libpng (Homebrew's on macOS).
set -euo pipefail

REPO="https://github.com/libretro/mupen64plus-libretro-nx.git"
COMMIT="15bada85906593376e8fd697f1b0b3af3a20f512"
CLEAN="libretro mupen64plus-core"
ID_HEADER="libretro/dj_build_id.h"
BUILT="mupen64plus_next_libretro"

compile() {
  # The bundled zlib and libpng take macOS for classic Mac OS (fp.h, fdopen), so build
  # against the system's copies.
  if [ "$(uname -s)" = Darwin ]; then
    # macOS has zlib but no libpng, and Homebrew's is not on every Mac: link its static
    # library in, so the core loads anywhere. pkg-config is pointed at a libpng.pc that
    # names the .a file, which is what the core's makefile asks.
    mkdir -p dj-pkgconfig
    printf 'Name: libpng\nDescription: libpng, linked statically\nVersion: %s\nCflags: %s\nLibs: %s/libpng16.a\n' \
      "$(pkg-config --modversion libpng)" "$(pkg-config --cflags libpng)" "$(pkg-config --variable=libdir libpng)" > dj-pkgconfig/libpng.pc
    export PKG_CONFIG_PATH="$PWD/dj-pkgconfig"
    rm -f mupen64plus_next_libretro.dylib   # link again: make doesn't see changed flags
  fi
  make -j"$jobs" SYSTEM_ZLIB=1 SYSTEM_LIBPNG=1
}

. "$(dirname "$0")/../build-core.sh"
