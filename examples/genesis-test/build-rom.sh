#!/usr/bin/env bash
# Builds the Genesis test ROM and its symbol map into examples/genesis-test/build/.
# Needs only Python 3: tools/asm68k.py is a small 68000 assembler (no Mega Drive toolchain).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
build="$here/build"
mkdir -p "$build"
python3 -I "$here/tools/asm68k.py" "$here/src/main.s" "$build/genesis-test.bin" "$build/genesis-test.csv"
echo "built $build/genesis-test.bin"
