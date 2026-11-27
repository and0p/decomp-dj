# SameBoy connector

Patches [SameBoy's libretro core](https://github.com/LIJI32/SameBoy) (Game Boy, Game Boy
Color) so it reports which code ran, for decomp-dj.

- **Pinned upstream:** `90e2899574b7b63c59dce8a7276783426d52a068`, the `v1.0.5-libretro` tag
  (2026-11-02): the v1.0.5 release with its boot ROMs prebuilt, as libretro's own builds use it.
- **Build:** `connectors/sameboy/build.sh` clones that commit into `build/sameboy-src`, applies
  `dj-sameboy.patch`, copies in the files under `src/`, and builds `sameboy_libretro.{so,dylib,dll}`
  into `build/cores/` with make. Needs git, make, `hexdump` and a C compiler; no RGBDS, since
  the boot ROMs come prebuilt.
- **License:** MIT throughout. SameBoy is MIT (Expat, [LICENSE.txt](LICENSE.txt)), so the patch
  and the built core are too; the connector's own files (`dj_connector.c`, `.h`, in [src/](src/))
  are decomp-dj's and MIT ([src/LICENSE.txt](src/LICENSE.txt)).

## Why SameBoy

The Game Boy could have been covered by mGBA, which dj already has a connector for. SameBoy
was chosen instead, the way Mesen was for the NES:

- **Accuracy.** SameBoy is the most accurate Game Boy and Game Boy Color emulator with a
  libretro core: it passes nearly every timing test ROM (Mooneye, Blargg, the AGE suite) and
  emulates the CPU's quirks (the HALT bug, OAM corruption, the Color's speed switch). mGBA's
  Game Boy core is good but less exact, and Gambatte, the other accurate choice, is GPL-2 with
  no debugger hooks to build on.
- **A place to hook execution.** SameBoy's debugger API already has an execution callback
  (called after each opcode fetch, before the instruction runs) and memory read and write
  callbacks. The patch changes only the libretro front end, 12 lines, and none of the
  emulator: the same SameBoy runs with and without it.
- **The licence.** MIT, like dj, so there is no split licence to explain.
- **The GBA stays as it is.** Hooking mGBA's Game Boy CPU would have changed the mGBA patch,
  and with it the mGBA core's build id that every GBA project records. A separate core leaves
  the GBA example's setup untouched.
- **Speed and determinism.** See [Checked](#checked): fast enough to play thousands of frames a
  second, and every run, from power-on or from a save state, ends with the same hashes once
  the two sources of chance below are pinned.

What mGBA would have reused (the proc address plumbing and region bookkeeping, about 150
lines) is small next to what any Game Boy connector needs anyway: the bank mapping, the SM83
instruction lengths and the trace hook.

## What the patch does

- `libretro/libretro.c` registers `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK` for the
  connector's functions, and hands the connector the first Game Boy after a game loads.
  `libretro/Makefile.common` builds `dj_connector.c`.
- `libretro/dj_connector.c` (new) sets SameBoy's execution callback once dj asks for the API
  version, maps each instruction's address to a region and offset through the core's own bank
  registers, sets the flags, calls dj at a breakpoint, and holds the `dj_*` functions. The read
  and write callbacks are set only while a trace records, so they cost nothing otherwise.
- **What dj pins while it drives the core:**
  - `sameboy_rtc` is forced to `accurate` (`dj_forced_options`): the cartridge clock of MBC3,
    HuC3 and TPP1 games then follows emulated time. The core's default, "sync to system
    clock", reads the host's clock, so no two runs would match.
  - Power-on RAM: SameBoy fills WRAM, HRAM and OAM from a generator seeded with the time the
    core was loaded, as real hardware powers on with noise. While dj drives the core the
    generator is off, so RAM starts as the same pattern (mostly zeros) every time. A game that
    reads RAM before writing it behaves as on emulators that clear RAM.
- Save states are SameBoy's own; it saves and loads at once, with no instructions run.

Regions, in index order (connector API version 3), all `cpu` `sm83`:

| Region | Addresses | Size | `data` | `file_offset` |
| --- | --- | --- | --- | --- |
| `ROM` | `0x0000`-`0x7FFF` through the bank registers | the ROM file's size | the ROM | 0 |
| `WRAM` | `0xC000`-`0xDFFF` (and its echo), bank by bank: offset = bank × `0x1000` + (address & `0xFFF`) | 8 KB, 32 KB on a Color | yes | -1 |
| `HRAM` | `0xFF80`-`0xFFFE` | 127 bytes | yes | -1 |
| `BOOT` | the boot ROM while it is mapped (`0x0000`-`0x00FF`, and `0x0200`-`0x08FF` on a Color) | 256 bytes, 2,304 on a Color | yes | -1 |
| `SRAM` | `0xA000`-`0xBFFF`, bank by bank; only when the cartridge has RAM | the cartridge RAM's size | yes | -1 |

- **Banking.** A ROM address maps to `bank × 0x4000 + (address & 0x3FFF)`, wrapped to the
  ROM's size, with the bank the core has selected for that half (`0x0000`-`0x3FFF` can be
  banked too, on MBC1 multicarts and MMM01). That is the core's own bookkeeping, so every
  mapper SameBoy emulates is covered the same way: MBC1, MBC2, MBC3, MBC5, MBC7, MMM01, HuC1,
  HuC3, TPP1, the Pocket Camera and ROM-only cartridges. Checked by hand on MBC1, MBC3 and
  MBC5 ([Checked](#checked)).
- **Flags:** `0x01` on an opcode, `0x02` on its operand bytes, each mapped on its own (an
  instruction across a bank's end is flagged where its bytes are). No mode bits. `CB`-prefixed
  instructions are two bytes; `STOP` is two, as assemblers write it.
- **Breakpoints** can be at an opcode's offset in any region; dj counts logic ticks at one.
- **`dj_read_registers`** (12 bytes): A, F, B, C, D, E, H, L, then SP and PC with their low
  byte first. At a breakpoint PC is the address of the instruction about to run.
- **Traces** (`dj_trace_*`): one execute event per instruction, with its operand bytes, and one
  event per data byte read or written by the CPU. Fetches of the instruction's own bytes are
  part of its execute event, not reads. `a` holds A and `p` holds F before the instruction;
  `x`, `y` and `sp` are 0 (`dj_read_registers` has the rest). `cycle` counts instructions since
  the trace started. Reads and writes outside the regions (VRAM, OAM, I/O) have region -1. DMA
  (OAM DMA, the Color's HDMA) isn't the CPU and isn't traced.

## For a project

- **Hashes:** `RAM` is SameBoy's WRAM (all eight banks on a Color). HRAM is hashed beside it
  through the memory maps: `regions = ["RAM", "BUS:0x0000FF80-0x0000FFFE", "SRAM"]`. `dj init`
  picks these for a Game Boy game ([config.md](../../docs/config.md#console-defaults)). VRAM
  and OAM aren't hashed.
- **Battery saves:** `SRAM` is the cartridge's RAM when it has a battery; `dj sram` writes it.
  The clock of an RTC cartridge is SameBoy's `RTC` memory, not hashed.
- **Symbols:** a `csv` symbol map (`name,ROM,offset,size,cpu_addr`, offsets into the ROM file).
  RGBDS's `.sym` and `.map` files aren't read yet; `offset` is `bank × 0x4000 + (address & 0x3FFF)`
  for an RGBDS `BANK:ADDRESS` symbol.
- **The decoder:** `sm83` is built into dj ([decoders.md](../../docs/decoders.md)).

## What is not covered

- **Code run from WRAM, HRAM or cartridge RAM** is flagged in those regions, but coverage rolls
  up only `ROM`. Code in cartridge RAM is flagged only for ROM-only, MBC1, MBC3 and MBC5
  cartridges with RAM enabled.
- **VRAM and OAM:** code run from there (a few demos and exploits) isn't flagged.
- **Calls and jumps into `0x4000`-`0x7FFF`** name a CPU address, which every switchable bank
  shares: callers between banks can be attributed to the wrong function.
- **The Super Game Boy** runs, but its SNES side isn't seen, and **link cable play** (two Game
  Boys in one core) reports only the first.
- **Linux** hasn't been tried yet; the build needs nothing macOS-specific.

- **Log lines.** SameBoy logs `Could not open boot ROM` twice when it loads: it looks for
  one in the system directory first, finds none, and uses the one built into the core.

## Checked

2026-11-25, on macOS (Apple silicon), on the Game Boy test ROM
([examples/gb-test](../../examples/gb-test/)):

- `dj check --from-power-on --strict`: every segment ends with its recorded hashes, and the
  logic tick (a breakpoint at the main loop) counts one a frame.
- **Save states:** each child segment played alone, from its parent's stored end state, ends
  with the hash recorded from power-on.
- `dj connector-check`: every flag agrees with the decoder (107 instructions, operand flags,
  nothing disagreeing).
- **Mappers:** the same program built as MBC1, MBC3 and MBC5, and as a Game Boy (not Color)
  game: code in banks 1 and 2 is flagged at its file offset, and `dj connector-check` finds
  nothing disagreeing on any of them.
- `dj sram`: writes the 8 KB battery RAM, with the counter the program keeps there.
- **Traces** (the MCP server's `trace`): instructions with their bytes, A and F, and the reads
  and writes, including WRAM bank switches and the battery RAM.
- **The hook changes nothing:** with the execution callback left off, a 6,000-frame run ends
  with the same hash.
- **Speed:** 472 frames per second against 503 without the execution callback, about 6% slower
  (dj playing a 6,000-frame segment; SameBoy itself is the larger cost).
- Not compared with the unpatched core: without the connector, power-on RAM is random and the
  clock is the host's, so no two unpatched runs match each other either.
