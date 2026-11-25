# Mesen-S connector

Patches the [libretro Mesen-S core](https://github.com/libretro/Mesen-S) (Super Nintendo /
Super Famicom) so it reports which code ran, for decomp-dj.

## Why Mesen-S

The SNES core was picked the way Mesen was for the NES: the best balance of accuracy and
speed, with save states that are deterministic and a clean place to hook execution.

- **Accurate and fast enough.** Mesen-S is one of the most accurate SNES emulators, and
  it emulates nearly every coprocessor commercial games used (SA-1, Super FX, Cx4, the
  DSPs, S-DD1, SPC7110 and others). Under dj it plays a few hundred frames per second on
  one core, several times real time. snes9x is faster but less accurate; bsnes is about as
  accurate, and its save states are the problem below.
- **Save states are taken between frames.** Mesen-S has no cooperative threads: `retro_run`
  runs one whole frame and `retro_serialize` writes the state it stopped in, without
  running anything. bsnes and bsnes-mercury serialize at a synchronisation point (higan's
  `runToSave`), which runs the emulation on a little before saving, so the same frame can
  hash two ways ([.agents/_archive/CONNECTOR_PLAN.md](../../.agents/_archive/CONNECTOR_PLAN.md)).
- **One place sees every fetch, and the mapping is already there.** Every 65816 bus access
  goes through `Cpu::Read` with its kind (opcode, operand, data), and every memory handler
  turns a CPU address into "this byte of ROM / work RAM / save RAM" (`GetAbsoluteAddress`),
  the key coverage needs. So LoROM, HiROM, ExHiROM and the coprocessor boards' own mappings
  all work without the connector knowing about them. The same author's NES core is dj's
  Mesen connector, built the same way.
- **Maintained.** libretro/Mesen-S still gets build fixes (2026). Mesen2, its successor,
  has no libretro core.

## The build

- **Pinned upstream:** `9e4fdeb9b336470bc96beb8765b2e79c86a2da1e` (2026-08-23).
- **Build:** `connectors/mesen-s/build.sh` clones that commit into `build/mesen-s-src`,
  applies `dj-mesen-s.patch`, copies in the files under `src/`, and builds
  `mesen-s_libretro.{so,dylib,dll}` into `build/cores/`. Needs git, make and a C++11
  compiler. About 25 seconds on an Apple silicon Mac.
- **License:** two parts. The connector's own files (`Core/DjConnector.cpp`, `.h`, in
  [src/](src/)) are decomp-dj's and MIT ([src/LICENSE.txt](src/LICENSE.txt)).
  `dj-mesen-s.patch` changes Mesen-S's own files to hook them in, so it is GPL-3.0 like
  Mesen-S ([LICENSE.txt](LICENSE.txt)), and so is the core built from them. dj itself (MIT)
  is a separate program that loads the core at run time, like any libretro frontend.

## What the patch does

- `Cpu::Read` (the main CPU's, `Core/Cpu.cpp`) calls `DjConnector::OnExec` for every opcode
  and operand fetch, before the byte is read, once dj has asked for the API version. It maps
  the address to a region and offset with Mesen-S's own memory mappings and sets the flags.
  At an opcode it also tests the breakpoints, and calls dj there, before the instruction
  runs. `Cpu::Read` and `Cpu::Write` record trace events while tracing is on.
- `retro_set_environment` registers `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK`;
  `retro_load_game` sizes the regions for the game, and `retro_unload_game` drops them.
- `EmuSettings` seeds its random numbers with 0 instead of the clock, so anything a game
  asks to be random (Mesen-S's per-game RAM fill for one title) is the same on every run.
- It forces one core option (`dj_forced_options`): `mesen-s_ramstate = All 0s`. Mesen-S
  fills RAM with random bytes at power-on by default, which would make every run hash
  differently. Nothing else about emulation changes: the patched and unpatched cores reach
  the same hashes.

Regions, in index order (connector API version 3), all `cpu` `65816`:

| Region | What | Size | `data` | `file_offset` |
| --- | --- | --- | --- | --- |
| `ROM` | The cartridge's ROM, as the file has it after any copier header, however the board maps it | the ROM file's, less the header | the ROM | 0, or 512 with a copier header |
| `WRAM` | The console's work RAM, `$7E0000`-`$7FFFFF` (and its mirror at `$0000`-`$1FFF` of the low banks) | 128 KB | yes | -1 |
| `SRAM` | The cartridge's save RAM, when it has some | the game's | yes | -1 |

Flags: `0x01` on an opcode, `0x02` on its operand bytes, and on the opcode the register
widths it ran with: `0x04` 8-bit accumulator, `0x08` 16-bit, `0x10` 8-bit index registers,
`0x20` 16-bit (emulation mode counts as 8-bit). A breakpoint can be at an opcode in any
region, and dj counts logic ticks at one. `dj_read_registers` gives 16 bytes: A, X, Y, S, D
and PC as 16-bit values, low byte first, then the program bank, the data bank, P, and 1 in
emulation mode. At a breakpoint PC is the instruction's own address.

Traces (`dj_trace_start` and the rest): one event per instruction, with its operand bytes,
and one per byte the CPU reads or writes. The address is 24 bits: `addr` holds the low 16,
`addr_high` the bank. The registers in an event are the low bytes of A, X, Y and S, and P;
`dj_read_registers` gives the whole ones at a breakpoint. `cycle` counts CPU cycles since
power-on. DMA is not traced. dj disassembles 65816 traces itself (`trace` in the MCP
server).

A Game Boy game, which Mesen-S also plays, reports no regions: only the SNES's CPU is
hooked. A Super Game Boy cartridge keeps them (its BIOS runs on the 65816).

## What isn't covered

- **Coprocessors.** Only the main 65816 is hooked. Code run by the SA-1 (a second 65816),
  the Super FX (GSU), the Cx4, the DSP-1 to DSP-4, the ST010/ST011 and the SPC700 (the
  sound CPU, from audio RAM) isn't flagged. On an SA-1 game the main CPU's code is still
  flagged in `ROM`, but the SA-1's isn't.
- **Banks a coprocessor switches.** The main CPU's fetches are mapped as the board maps them
  at that moment, so SA-1, SPC7110 and S-DD1 ROM banking is followed; but nothing of those
  boards has been tried.
- **Code in other memory.** Code run from anywhere but ROM, WRAM and SRAM (the SA-1's
  I-RAM or BW-RAM, a BS-X memory pack) isn't flagged.
- **Mode tracking in the decoder.** dj's 65816 decoder starts each function in the widths
  its first instruction ran with and follows `REP` and `SEP` from there. Code that changes
  widths some other way (`PLP`, a callee that returns with other widths) can be decoded
  with the wrong immediate lengths after that point.

## For a project

- **Hashes:** `RAM` (the 128 KB of WRAM) and `SRAM`. Mesen-S publishes no memory maps for
  the SNES, so `BUS:` ranges and VRAM aren't available.
- **Symbols:** a ca65 `.dbg` (`format = "ca65-dbg"`, `region = "ROM"`) places code by its
  file offset; `dj init --symbols` sets the region. CPU addresses are 24 bits (`$018000`),
  and jumps and calls within a bank resolve to that bank. Other assemblers' formats (WLA-DX
  `.sym`, asar, bass) aren't read yet; `csv` works for any of them.
- **Snapshots:** a state is about 260 KB, handed out padded to 526 KB
  (`retro_serialize_size` is twice the first state's size), and the default `[cache]`
  preset, `nes-sparse`, suits it. A preset tuned on real games is for later.
- **Ticks:** `[tick] exec = "ROM:0x…"` at an opcode that runs once per logic step.

## Checked

2026-11-25, on the SNES test ROM ([examples/snes-test](../../examples/snes-test/)), Apple
silicon, macOS:

- `dj connector-check`: every flag agrees with the decoder (105 instructions, 8- and 16-bit
  immediates, `MVN`, `JSL` across banks, operand flags, no opcode run in both widths). The
  same on a HiROM build of the same program (`build/snes-test-hirom.sfc`).
- `dj check --from-power-on --strict` and `dj check --strict` (children from stored states)
  pass, also on a 20,000-frame segment with a 600-frame child.
- The same ending hashes with the unpatched core.
- Speed on that 20,600-frame run: 237-260 frames per second patched, 253-280 unpatched
  (within the noise of a busy machine; under 10%). The test ROM is idle most of each frame.
- `[tick]` breakpoints: one tick per frame at the main loop's `JSR`.
- MCP `trace`: instructions, operands, registers, reads and writes, with 65816 disassembly.
- Not checked: Linux and Windows builds, commercial games, any coprocessor board.
