#!/usr/bin/env bash
# Builds the Thwaite ROM and its symbol file from source into examples/thwaite/build/.
# Thwaite is a free (GPL-3.0) NES game by Damian Yerrick: https://github.com/pinobatch/thwaite-nes
# Needs git, make, a C compiler, Python 3 with Pillow, and cc65 (built here if missing).
set -euo pipefail

THWAITE_COMMIT="00e36745188bc165990f60eed6b093c3ce6ad0e3"
CC65_COMMIT="77b6484615427746e55fda2755c9cbc320a1fb51"   # ca65 V2.19

here="$(cd "$(dirname "$0")" && pwd)"
build="$here/build"
mkdir -p "$build"

if [ ! -x "$build/cc65/bin/ca65" ]; then
  [ -d "$build/cc65/.git" ] || git clone https://github.com/cc65/cc65.git "$build/cc65"
  git -C "$build/cc65" checkout -q "$CC65_COMMIT"
  make -C "$build/cc65" -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" bin
fi
export PATH="$build/cc65/bin:$PATH"

[ -d "$build/thwaite-nes/.git" ] || git clone https://github.com/pinobatch/thwaite-nes.git "$build/thwaite-nes"
git -C "$build/thwaite-nes" checkout -q "$THWAITE_COMMIT"
# Thwaite's makefile links its host helper (tools/dte) with -static, which macOS
# cannot do. Build it without -static first so make skips that step.
if [ "$(uname)" = "Darwin" ]; then
  cc -std=gnu99 -Os -o "$build/thwaite-nes/tools/dte" "$build/thwaite-nes/tools/dte.c"
fi
make -C "$build/thwaite-nes" thwaite.nes
cp "$build/thwaite-nes/thwaite.nes" "$build/thwaite-nes/thwaite.dbg" "$build/"

echo "built $build/thwaite.nes"
if command -v sha256sum >/dev/null; then sha256sum "$build/thwaite.nes"; else shasum -a 256 "$build/thwaite.nes"; fi
echo "config.toml expects ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173"
