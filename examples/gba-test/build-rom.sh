#!/usr/bin/env bash
# Builds the GBA test ROM and its ELF (the symbol map) into examples/gba-test/build/.
# Needs LLVM's clang and llvm-objcopy (Homebrew's llvm, or any LLVM with the ARM target)
# and an ELF linker for ARM: ld.lld, or the rust-lld that comes with Rust.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
build="$here/build"
mkdir -p "$build"

find_tool() {
  for t in "$@"; do
    if command -v "$t" >/dev/null 2>&1; then command -v "$t"; return; fi
  done
  return 1
}
llvm_bin="$(brew --prefix llvm 2>/dev/null || true)/bin"
clang="$(find_tool "$llvm_bin/clang" clang)" || { echo "needs clang"; exit 1; }
objcopy="$(find_tool "$llvm_bin/llvm-objcopy" llvm-objcopy)" || { echo "needs llvm-objcopy"; exit 1; }
if lld="$(find_tool "$llvm_bin/ld.lld" ld.lld)"; then
  link=("$lld")
else
  # rust-lld ships with Rust: in the active toolchain, or any rustup toolchain.
  rust_lld=""
  for c in "$(rustc --print sysroot 2>/dev/null)"/lib/rustlib/*/bin/rust-lld "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/rust-lld; do
    if [ -x "$c" ]; then rust_lld="$c"; break; fi
  done
  [ -n "$rust_lld" ] || { echo "needs ld.lld or rust-lld"; exit 1; }
  link=("$rust_lld" -flavor gnu)
fi

asm_flags=(--target=armv4t-none-eabi -mcpu=arm7tdmi -g)
flags=("${asm_flags[@]}" -ffreestanding -fno-builtin -nostdlib -O2)
"$clang" "${asm_flags[@]}" -c "$here/src/crt0.s" -o "$build/crt0.o"
"$clang" "${asm_flags[@]}" -c "$here/src/iwram.s" -o "$build/iwram.o"
"$clang" "${flags[@]}" -mthumb -c "$here/src/main.c" -o "$build/main.o"
"${link[@]}" -T "$here/src/gba.ld" "$build/crt0.o" "$build/main.o" "$build/iwram.o" -o "$build/gba-test.elf"
"$objcopy" -O binary "$build/gba-test.elf" "$build/gba-test.gba"
echo "built $build/gba-test.gba"
