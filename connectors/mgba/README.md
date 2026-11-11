# mGBA connector

Patches [libretro's mGBA core](https://github.com/libretro/mgba) (Game Boy Advance) so it
reports which code ran, for decomp-dj.

- **Pinned upstream:** `7a12d6d4b9acb14c0ae62c9166b6a2f3d08007f6` (2026-09-16).
- **Build:** `connectors/mgba/build.sh` clones that commit into `build/mgba-src`, applies
  `dj-mgba.patch`, and builds `mgba_libretro.{so,dylib,dll}` into `build/cores/` with CMake.
  Every optional library (zlib, libpng, SQLite, Lua, FFmpeg, ...) and the debuggers are
  turned off, so the core is the same whatever the machine has installed. Needs git, CMake,
  make and a C compiler.
- **License:** mGBA is MPL-2.0. The new files (`dj_connector.c`, `dj_connector.h`) are
  decomp-dj's own and MIT (so they sit in an MPL-2.0 core without trouble); the few lines
  the patch changes in mGBA's files are under the MPL-2.0, as is the core built from it.
  The player (GPL-3.0) is a separate program that loads the core at run time, like any
  libretro frontend.
- **Game Boy games** load as usual but report no regions: only the GBA's CPU is hooked.

## What the patch does

- `src/arm/arm.c` calls a hook (`ARMDjExecHook`) for every instruction `ARMStep` and
  `ThumbStep` reach, with its address, opcode and state, before its condition is checked.
  The hook is NULL, and costs one test per instruction, until dj asks for the API version.
- `src/platform/libretro/dj_connector.c` (new) maps the address to a region and offset and
  sets the flags, counts ticks, and holds the `dj_*` functions; `libretro.c` registers
  `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK` for them and hands the connector the GBA
  on load.
- Nothing about emulation changes, and no options are forced. Save states are mGBA's own,
  which line up with frames.

Regions, in index order (connector API version 2), all `cpu` `armv4t`:

| Region | Addresses | Size | `data` | `file_offset` |
| --- | --- | --- | --- | --- |
| `ROM` | `0x08000000`-`0x0DFFFFFF`, the three wait-state mirrors folded together | the ROM file's size | the ROM | 0 |
| `IWRAM` | `0x03000000` | 32 KB | yes | -1 |
| `EWRAM` | `0x02000000` | 256 KB | yes | -1 |
| `BIOS` | `0x00000000` | 16 KB | yes | -1 |

Flags: `0x01` on an opcode's first byte, `0x02` on the rest, and the `armv4t` mode bits on
the first byte: `0x04` ran in Thumb state, `0x08` in ARM state. Thumb's `BL` is two
halfwords the CPU runs one after the other; the second gets operand flags, so the pair reads
as one 4-byte instruction, as dj's decoder reads it. Ticks are counted at an opcode's
address in any region.

The BIOS is mGBA's own (its HLE BIOS, real ARM code) unless a `gba_bios.bin` is in the
system directory; dj gives every core an empty one of its own, so it's always mGBA's and
runs are the same on every machine.

## For a project

- **Hashes:** mGBA's `RETRO_MEMORY_SYSTEM_RAM` (the `RAM` hash region) is only the first
  32 KB of EWRAM (the core reports the Game Boy's size). Hash the GBA's RAM through the
  memory maps instead, which mGBA publishes:
  `regions = ["BUS:0x03000000-0x03007FFF", "BUS:0x02000000-0x0203FFFF", "SRAM"]`.
  `dj init` doesn't pick these yet.
- **Symbols:** an ELF from the game's build (`format = "elf"`, `region = "ROM"`,
  `rom_header = 0x08000000`, the address of the ROM file's first byte). Thumb functions are
  read from the symbols' bit 0, and data inside code (literal pools, jump tables) from the
  ELF's `$d` mapping symbols.
- **Code in IWRAM or EWRAM** is flagged in those regions, but coverage rolls up only `ROM`,
  so a function the ELF links to run from IWRAM shows as never entered.

## Checked

2026-11-09, on the GBA test ROM ([examples/gba-test](../../examples/gba-test/)):

- `dj check-connector`: every flag agrees with the decoder (113 instructions, ARM and
  Thumb, operand flags, no opcode run in both states).
- The same ending hashes with and without the patch (a 20,000-frame segment, and every
  segment of the example from power-on).
- Speed: 2,300 frames per second against 3,700 for the unpatched core (Apple silicon), about
  37% slower, from the call per instruction. Mesen's connector costs about 5%; folding the
  flag write into `ARMStep` and `ThumbStep` without a call would close most of the gap.
