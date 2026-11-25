#!/usr/bin/env bash
# Builds the SNES test ROM and its symbol file into examples/snes-test/build/: snes-test.sfc
# (LoROM, the one the project plays) and snes-test-hirom.sfc (the same program as HiROM, for
# checking the connector's HiROM mapping), each with its ca65 .dbg.
# Needs git, make, a C compiler and Python 3; builds cc65 here if it isn't built yet.
set -euo pipefail

CC65_COMMIT="77b6484615427746e55fda2755c9cbc320a1fb51"   # ca65 V2.19, as examples/thwaite

here="$(cd "$(dirname "$0")" && pwd)"
build="$here/build"
mkdir -p "$build"

if [ ! -x "$build/cc65/bin/ca65" ]; then
  [ -d "$build/cc65/.git" ] || git clone https://github.com/cc65/cc65.git "$build/cc65"
  git -C "$build/cc65" checkout -q "$CC65_COMMIT"
  make -C "$build/cc65" -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" bin
fi
bin="$build/cc65/bin"

# The header's checksum: the sum of every byte, with its complement beside it.
fix_checksum() {
  python3 -I - "$1" "$2" <<'PY'
import sys
path, at = sys.argv[1], int(sys.argv[2], 16)
rom = bytearray(open(path, "rb").read())
rom[at:at + 4] = b"\xff\xff\x00\x00"
s = sum(rom) & 0xFFFF
rom[at:at + 4] = (s ^ 0xFFFF).to_bytes(2, "little") + s.to_bytes(2, "little")
open(path, "wb").write(rom)
PY
}

# Relative paths, so the .dbg files name no folder of this machine.
cd "$here"
"$bin/ca65" --cpu 65816 -g -o build/main.o src/main.s
"$bin/ld65" -C src/lorom.cfg --dbgfile build/snes-test.dbg -o build/snes-test.sfc build/main.o
fix_checksum build/snes-test.sfc 7FDC

"$bin/ca65" --cpu 65816 -g -D HIROM -o build/main-hirom.o src/main.s
"$bin/ld65" -C src/hirom.cfg --dbgfile build/snes-test-hirom.dbg -o build/snes-test-hirom.sfc build/main-hirom.o
fix_checksum build/snes-test-hirom.sfc FFDC

for f in snes-test.sfc snes-test-hirom.sfc; do
  echo "built $build/$f"
  if command -v sha256sum >/dev/null; then sha256sum "$build/$f"; else shasum -a 256 "$build/$f"; fi
done
