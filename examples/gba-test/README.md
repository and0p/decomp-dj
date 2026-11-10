# GBA test ROM

A tiny Game Boy Advance program, built from source here, for trying the mGBA connector and
the ARM decoder end to end. It draws nothing (the screen is one colour that changes); each
button runs different code:

| Code | Where | Runs on |
| --- | --- | --- |
| `_start` (start-up, with the cartridge header as data) | ROM, ARM | power-on |
| `main` | ROM, Thumb | every frame |
| `on_a`, with a jump table in the middle of the code | ROM, Thumb | A |
| `on_b`, with conditional instructions | ROM, ARM, reached through a linker thunk | B |
| `on_start` | ROM, Thumb | Start (no segment presses it: it stays uncovered) |
| `on_direction` | ROM, Thumb | Left, Right |
| `iwram_mix` | IWRAM, ARM, copied there at start-up | Left, Right |

## Building

```sh
./build-rom.sh                  # build/gba-test.gba and build/gba-test.elf
../../connectors/mgba/build.sh  # the core, into build/cores/ at the repository root
```

`build-rom.sh` needs LLVM's `clang` and `llvm-objcopy` (Homebrew's `llvm`) and an ELF linker
for ARM: `ld.lld`, or the `rust-lld` that comes with Rust. No GBA toolchain is needed.

Then set the ROM and the core in `.local/local.toml` (relative to this folder):

```toml
core = "../../build/cores/mgba_libretro"
rom = "build/gba-test.gba"
```

and play it: `dj run examples/gba-test`, `dj summary examples/gba-test`.

## The project

- **Segments:** boot and idle for 60 frames, then hold A, tap B twice, press Right.
- **Hashes:** IWRAM and EWRAM through the memory maps, and SRAM (see
  [connectors/mgba](../../connectors/mgba/README.md#for-a-project)).
- **Symbols:** `symbols/gba-test.elf`, with `rom_header = 0x08000000`.
- **Coverage:** 113 of 140 instructions (80.7%). Not covered: `on_start` (Start isn't
  pressed), the `b` the linker puts after `bx pc` in each thunk (never runs), the loop after
  `main` returns (never does), and `iwram_mix`, which runs from IWRAM: coverage rolls up only
  ROM, so it shows as never entered.

The ROM isn't committed (`config.toml` pins its sha256); the ELF is, as `symbols/gba-test.elf`.
Rebuilding gives the same bytes with the same LLVM (checked with LLVM 23.1.5). Another LLVM
version can give another ROM: rebuild, copy the new ELF into `symbols/`, update `[rom]
sha256`, and re-record with `dj clean --accept`.
