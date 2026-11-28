# Genesis Plus GX connector

Patches [libretro's Genesis Plus GX core](https://github.com/libretro/Genesis-Plus-GX)
(Sega Mega Drive / Genesis) so it reports which code ran, for decomp-dj.

## Why this core

Picked the way Mesen was picked for the NES: the best balance of accuracy and speed, with
deterministic save states and a place to hook execution.

| | Genesis Plus GX | BlastEm | PicoDrive |
| --- | --- | --- | --- |
| Accuracy | High: the reference among libretro Mega Drive cores | Highest (cycle-accurate VDP) | Lower: built for speed on weak hardware |
| 68000 | Musashi-derived interpreter, the same C everywhere | Translated to x86 machine code; other hosts need another CPU core | Cyclone (ARM assembly) or FAME, by platform |
| Save states | Synchronous (`retro_serialize` calls `state_save` at the frame boundary), no threads | Supported, less used through libretro | Supported |
| A place to hook | One loop (`m68k_run`) and one instruction-stream fetch (`m68ki_read_imm_16`); a debug hook already sits there | Generated code: per-instruction hooks mean changing the recompiler | Assembly CPU cores on ARM |
| Licence | Non-commercial (below) | GPL-3 | Non-commercial (MAME-style) |

Genesis Plus GX is the one that runs the same C on every machine, saves synchronously and has
an obvious hook point. PicoDrive has the same kind of licence and is less accurate; BlastEm
has the better licence but runs its 68000 as generated x86 code, which leaves no single
place to see each instruction, and no one CPU core on x86 and ARM machines alike.

**The licence, and the choice made.** Releases include the built core for now (decided 2026-11-27), with this licence text beside it and its source on the release. Genesis Plus GX's own licence
([LICENSE.txt](LICENSE.txt)) allows redistribution and derivative works only if they are
not sold or used in a commercial product or activity, and a modified build must come with
its complete source. It is not the GPL and not compatible with it. What that means here:

- `dj-genesis-plus-gx.patch` changes the core's files, so it is under that licence, and so
  is any core built from it. The connector's own files ([src/](src/)) are MIT
  ([src/LICENSE.txt](src/LICENSE.txt)), like the rest of decomp-dj, which stays a separate
  program that loads the core at run time.
- Shipping the built core in a release (as decided for the other cores) is allowed for free,
  non-commercial distribution, with the source: the pinned upstream commit, this patch and
  `src/`. A paid or commercial distribution of decomp-dj could not include it.

## Build

- **Pinned upstream:** `51348f65738b0c75f4b8b4488fbc67645e3769af` (2026-11-25).
- **Build:** `connectors/genesis-plus-gx/build.sh` clones that commit into
  `build/genesis-plus-gx-src`, applies `dj-genesis-plus-gx.patch`, copies in the files under
  `src/`, and builds `genesis_plus_gx_libretro.{so,dylib}` into `build/cores/` with the
  core's `Makefile.libretro`. The Mega CD's CHD disc images aren't built (`HAVE_CHD=0`); the
  core's own zlib is used, so nothing depends on what the machine has installed. Needs git,
  make and a C compiler.

## What the patch does

- `core/m68k/m68kcpu.c` calls a hook (`dj_m68k_exec`) in `m68k_run` before each instruction
  of the main 68000 is fetched, with its address. `core/m68k/m68kcpu.h` calls a second one
  (`dj_m68k_fetch`) from `m68ki_read_imm_16` and `_32`, which fetch the opcode word and
  every extension word, and a third (`dj_m68k_access`) from the data reads and writes, for
  traces. All three are NULL, and cost a test each, until dj asks for the API version (or
  starts a trace). They are compiled into the main CPU only: the Mega CD's second 68000 is
  the same code built again without them.
- `libretro/dj_connector.c` (new) maps an address to a region and offset through the 68000's
  own memory map (so bank-switching boards are followed), sets the flags, calls dj at a
  breakpoint, records traces and holds the `dj_*` functions; `libretro.c` registers
  `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK` for them and tells the connector when a game
  is loaded.
- Nothing about emulation changes, and no options are forced. Save states are the core's
  own.

Regions, in index order (connector API version 3), both `cpu` `m68k`, for Mega Drive /
Genesis cartridges:

| Region | Addresses | Size | `data` | `file_offset` |
| --- | --- | --- | --- | --- |
| `ROM` | where the 68000's memory map points into the cartridge (`0x000000`-`0x3FFFFF` on a plain cartridge; mirrors folded together) | the ROM's size | the ROM, big-endian as in the file | 0 for a plain dump (`.bin`, `.md`, `.gen`); -1 when the core changed the file while loading it (an `.smd` file's header and interleaving) |
| `RAM` | `0xFF0000`-`0xFFFFFF`, mirrored from `0xE00000` | 64 KB | none | -1 |

Flags: `0x01` on an instruction's first byte, `0x02` on the second byte of its opcode word
and on every extension word the CPU fetched. The 68000 has no mode bits. A `Bcc.W` or `DBcc`
that doesn't branch skips its displacement word without fetching it; the connector flags
that word from the opcode, so every instruction that ran reads as long as it is.
A breakpoint can be at an instruction's first byte in either region, and dj counts logic
ticks at one. `dj_read_registers` hands out D0 to D7, A0 to A7 (A7 the stack pointer in
use), the PC and the SR, 18 values of 32 bits in the host's byte order; at a breakpoint the
PC is the instruction about to run.

**Traces** (`dj_trace_start`): every instruction the main 68000 runs, and every byte it
reads or writes.

- **The address** is `addr_high << 16 | addr`, the 68000's 24-bit address; `offset` is in
  region `ROM` or `RAM`, or -1 (I/O, the VDP, the Z80's area).
- **An execute** carries the opcode word's first byte in `value` and up to three more bytes
  of the instruction in `operands`, in the CPU's order. The registers are 0;
  `dj_read_registers` gives them at a breakpoint.
- **A read or write** is one event per byte; a word access is two. Operands read
  PC-relative (`move.w table(pc),d0`) and the instruction stream itself aren't data reads,
  and aren't in the trace.
- **`cycle`** counts instructions since the trace started.
- **Not traced:** the Z80, and DMA (the VDP copying from ROM or RAM).

The decoder for `m68k` is built into dj (`crates/dj-decoder/src/isa/m68k.rs`): lengths,
branches, calls, returns and PC-relative literals.

## What isn't covered

- **The Z80** (the sound CPU), which runs code the 68000 copies into its 8 KB of RAM: no
  region, no flags, no breakpoints, not in traces. Its code is the sound driver, rarely what
  a decomp targets.
- **The Mega CD / Sega CD**: no regions are reported for a disc (its code runs from RAM
  loaded from the disc), and its second 68000 isn't hooked.
- **The 32X** isn't emulated by Genesis Plus GX at all (PicoDrive does).
- **Master System, Game Gear, SG-1000 and Pico** games load as usual but report no regions:
  their CPU is the Z80 (the Pico's 68000 isn't hooked either).
- **Mappers.** Bank-switching boards (the SSF2 mapper and others) are followed: an address
  maps through the 68000's memory map as it is when the instruction runs. Code on a board's
  own chip (the SVP in Virtua Racing) isn't 68000 code and isn't covered. A cartridge whose
  ROM is read through a handler with no memory behind it (some protection schemes) shows as
  outside the regions.
- **Data in RAM** isn't handed out (`data` is NULL for `RAM`): the core keeps it in 16-bit
  words of the host's byte order. Code that runs from RAM is flagged there, but coverage
  rolls up only `ROM`.

## For a project

- **Core name:** the core calls itself `Genesis Plus GX`; dj's `[core] name` is
  `genesis plus gx`. `dj init` knows it: it detects a Mega Drive ROM by "SEGA" at `0x100`
  of its header (or by a `.md` or `.gen` extension), suggests this core, hashes `RAM` (the
  68000's 64 KB) and `SRAM` when the game has save RAM, and names the pad's buttons
  ([config.md](../../docs/config.md#console-defaults)).
- **Buttons:** the core reads the RetroPad's Y, B and A as the pad's A, B and C; L, X and R
  as a 6-button pad's X, Y and Z; Select as Mode. A port's `JOYPAD` device is the core's
  "Joypad Auto": a 6-button pad when the game's header lists one (`6` in its I/O support
  field), else a 3-button pad.
- **Hashes:** `RAM` is the core's `work_ram`, in 16-bit words of the host's byte order. On
  every little-endian machine (x86, Apple silicon, ARM Linux) that is each pair of bytes
  swapped: the 68000's byte at `0xFF0000 + A` is at `A ^ 1`. The hashes are the same on all
  of them, but a `[WRITE]` or `[hash] exclude` offset in `RAM` is flipped the same way.
- **Symbols:** on a plain cartridge the CPU address is the ROM offset. A csv map
  (`name,ROM,offset,size,cpu_addr`), an ELF from the game's build (`format = "elf"`,
  `region = "ROM"`), or an ld map works. `jmp table(pc,d0.w)` jump tables of `bra.w`
  instructions decode as code; tables of data inside a function need `[[data]]` in
  annotations.toml unless the code reads them through `(d16,PC)`.
- **Snapshots:** a save state is about 1 MB (`0xFD000` bytes), which the default
  `small` cache preset stores whole, only at markers and segment ends.

## Checked

2026-11-25, on the Genesis test ROM ([examples/genesis-test](../../examples/genesis-test/)),
on macOS (Apple silicon):

- `dj connector-check`: every flag agrees with the decoder (152 instructions run inside the
  symbol map's functions: branches taken and not, `dbra`, a `bra.w` jump table, `movem`,
  PC-relative and absolute-long operands).
- `dj check --from-power-on --strict`: every segment's hashes match.
- The same ending hashes with and without the patch: every segment, and a 20,000-frame one
  holding B and Right, from power-on, on the patched core and on the same commit built
  unpatched.
- Save states: a root segment from a state taken at frame 30 of "Boot and idle" ends with
  that segment's hash, and one from frame 16 of "Press A four times" (A held there, the
  rest of its presses after) ends with that segment's hash.
- Traces: the MCP server's `trace` tool on `on_a` gives each instruction with its
  disassembly and the RAM it read and wrote.
- Speed: about 2,600 frames per second against 3,400 for the unpatched core (Apple silicon,
  `dj check --no-db` on the 20,000-frame segment), about 25% slower, from the calls per
  instruction and per extension word.

Not checked: a Linux build (the Makefile's `unix` target, which the script runs there), a
commercial game, and a cartridge with a mapper.
