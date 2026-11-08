# Plan: a C decompilation of Tetris (NES)

Draft for review, 2026-11-07. Decisions that change the plan are in
[QUESTIONS.md](QUESTIONS.md); where this plan assumes an answer, it says so and uses the
recommended default.

## 1. Goal

A C program that plays Tetris (USA) exactly as the ROM does, proven by decomp-dj: every
segment in `examples/tetris` plays on the decomp to the same state hashes (all 2 KB of
RAM) as it does on Mesen.

- **Behavioural, not matching** (Q1). A matching decomp rebuilds the ROM byte for byte
  from C. Tetris was written in 6502 assembly, and no C compiler for the 6502 (cc65,
  llvm-mos) produces code shaped like hand-written assembly, so a matching C decomp isn't
  realistic. The reference disassembly already rebuilds the ROM byte for byte; the C
  version's job is to be readable and to behave the same.
- **Proof is RAM parity.** The game's whole state lives in 2 KB of RAM (plus VRAM, which
  the game only writes). If the C version keeps every variable at its original address,
  decomp-dj's existing hashes become the test suite, with no new comparison format.
- **First target:** Tetris (U) [!], the dump in `build/tetris_usa.nes`. The PAL and
  Nintendo World Championships builds the disassembly also supports are out of scope at
  first (Q9).

## 2. The reference

[CelestialAmber/TetrisNESDisasm](https://github.com/CelestialAmber/TetrisNESDisasm),
commit `94581c0306fa0009fe11fde48222875549d15b8b` (2024-03-27), with its cc65 submodule at
`16bf472e`. It came out of ejona86's [taus](https://github.com/ejona86/taus) info file.

What I checked:

- It builds as is (`git submodule update --init && make`, needs Pillow). The output
  matches the repo's own `tetris.sha1` (`77747840…`).
- **Its PRG and CHR are identical to your dump.** The 49,152 bytes after the header hash
  the same (`sha1 fd9079cb…`). Only the 16-byte header differs: your dump has a NES 2.0
  header, the build an iNES 1.0 one. So the debug file's offsets apply to your ROM with
  `rom_header = 16`, and `[rom] sha256` in `config.toml` stays as it is.
- The repo has no LICENSE file. That matters if the C source copies its names and
  comments and is ever published (Q5).

### The game in numbers

- MMC1, one 32 KB PRG bank that never switches, two 8 KB CHR banks switched between the
  menu, ending and game tilesets.
- About **10.2 KB of code in 245 routines**, by the count in [functions.csv](functions.csv)
  (labels that are code and are called, jumped to or listed in a jump table; local
  `@labels` and branch-only `Lxxxx` labels fold into their routine). Median routine 20
  bytes; 23 are over 100 bytes, the largest `render_mode_play_and_demo` at 444.
- The other ~22 KB of PRG is data: nametables, palettes, sprite tables, the music and
  sound data, demo input, and about 3 KB the disassembly marks unreferenced.

### 6502 habits the C has to deal with

These are the places where a line-by-line translation doesn't work, found reading
`main.asm`:

| Habit | Where | In C |
| --- | --- | --- |
| Jump tables after a `JSR`: `switch_s_plus_2a` pops its return address and jumps through the `.addr` list after the call | 20 sites (game mode, play state, render mode, endings) | A `switch` |
| Inline arguments after a `JSR`: `bulkCopyToPpu` reads a pointer from after its own call and bumps the return address | 22 sites | A pointer argument |
| Results in registers: the main loop compares A after `branchOnGameMode` against `gameModeState` to decide whether to wait for a frame | main loop | Return values, kept exact |
| Shared tails and fall-through into the next routine (`_updatePpuMask`, `_updatePpuCtrl`) | many | Tail calls |
| Waiting for vblank deep inside call chains (`updateAudioAndWaitForNmi`, `sleep_for_a_vblanks`); the NMI does the rendering, steps the RNG and polls the controller | everywhere | A yield (see 4.3) |
| Writing to the stack page as data: `initRam` stores `$C0 $80 _ $35 $AC` at `$0100` | boot | Kept, since `$0100-$01FF` is in the hashed RAM |
| Two-player code paths (`player2_*`, `makePlayer2Active`) | gameplay | `numberOfPlayers` is only ever set to 1, so they can't run |

## 3. Coverage today, by routine

I made a `csv` symbol map from the reference build ([tools/dbg2csv.py](tools/dbg2csv.py),
written to `examples/tetris/build/ref/tetris_symbols.csv`) and combined it with the PRG
coverage the 23 clean segments have in `.local/cache.db`. Counts are bytes run (opcode or
operand) inside each routine.

| Area | Routines | Bytes | Run | Routines never run |
| --- | --- | --- | --- | --- |
| gameplay | 57 | 2668 | 81% | 7 |
| audio | 99 | 2209 | 77% | 21 |
| ending | 23 | 1506 | 1% | 21 |
| system (NMI, boot, PPU copy, RNG, input, MMC1) | 39 | 1346 | 83% | 2 |
| menus and demo | 11 | 999 | 92% | 0 |
| play rendering | 7 | 789 | 73% | 0 |
| high scores | 9 | 721 | 28% | 5 |
| **all** | **245** | **10238** | **65%** | **56** |

So two thirds of the code already runs in some segment, and everything a decomp would do
first (the frame loop, menus, moving and locking pieces, line clears, scoring, the demo)
can already be checked. What's missing is mostly the endings (A-type rockets, B-type
concert and cathedral), entering a high score, some sound effects and the B-type goal
check. The two-player routines will never run.

**A decomp-dj finding on the way:** the `ca65-dbg` reader finds no functions in this
disassembly's `.dbg`. Its code segments are named `PRG_chunk1..3`, not `CODE`, and it uses
no `.proc` scopes, so neither rule in [coverage.md](../../../docs/coverage.md#symbol-maps)
applies. The `.dbg` also can't tell code labels from data labels, which is why
`dbg2csv.py` reads the `.asm` files to classify them. See Q6.

## 4. Architecture

Recommended shape, with the alternatives in QUESTIONS.md.

### 4.1 The decomp is a libretro core

`tetris_decomp_libretro` (`.dylib`/`.so`), C99, built with the host compiler (Q3). It
loads the same ROM file as content, so `[rom] sha256` still matches and decomp-dj can play
the project on it like any other core:

```sh
dj run examples/tetris --core build/decomp/tetris_decomp_libretro.dylib --no-db
```

Every segment whose end hash and checkpoints come out the same has parity. (`dj` doesn't
check the core's name, and `[core] sha256` is empty, so this works today. The setup id
differs, so `--no-db` keeps those runs out of the Mesen database; Q8 covers a proper
comparison core.)

Loading the ROM also means **the repository never holds ROM bytes**. Tilesets, nametables,
palettes, music data, sprite tables and demo input are read from the loaded ROM by the
offsets in the symbol map, as other decomps extract assets from a user's ROM at build
time (Q5).

### 4.2 RAM at its original addresses

One `uint8_t ram[0x800]`, exposed as `RETRO_MEMORY_SYSTEM_RAM`. Every variable in
`tetris-ram.asm` is a name for its original address (`#define playState ram[0x48]`, or
generated accessors for multi-byte and array variables; `ram.h` can be generated from
`tetris-ram.asm`). Rules that follow:

- No C local or global may stand in for a RAM variable. Locals only replace registers.
- Scratch bytes the original uses (`tmp1`, `tmp2`, `generalCounter*`) are written exactly
  as the original writes them, because their leftover values are hashed.
- The stack page is RAM too (4.4, Q4).

### 4.3 The frame loop as a coroutine

The original runs its logic in a main loop that blocks waiting for vblank, sometimes deep
in a call chain, while the NMI renders, steps the RNG and reads the controller. In C:

- The game's main thread is a coroutine (a small library such as minicoro, or
  `ucontext`). `wait_for_nmi()` yields.
- `retro_run` = run the NMI handler, then resume the main thread until it yields again.
- This matches the original only if the main loop never overruns a frame (lag). If it
  does, the real NMI fires in the middle of game logic. Phase 0 measures this with
  `[tick]` on the main loop: ticks equal to frames everywhere means no lag.
- Where Mesen's `retro_run` ends relative to the NMI (before or after it in the frame)
  decides which order makes RAM match at frame boundaries. Phase 1 calibrates it.

### 4.4 Hybrid while in progress (recommended, Q2)

So the game always runs and every routine can be checked the moment it's written, the
core also carries a small 6502 interpreter that runs the original code from the ROM:

- Each routine has an entry in a table: run the ROM's code, or its C version.
- When the interpreter reaches the entry of a routine that has a C version, it calls the
  C function and returns as `RTS` would. When C calls a routine that is still assembly,
  the interpreter runs it until its `RTS`.
- **Start:** every routine in assembly. The core then has to match Mesen on every segment
  with no C at all, which proves the platform layer (PPU, input, timing) before any game
  code is translated.
- **Finish:** every routine in C, the interpreter compiled out, and the core is the
  decomp.
- **Stack:** each `JSR` leaves its return address in the stack page, and those leftover
  bytes are hashed. C calls go through a macro that writes the same bytes the original
  `JSR` would (`CALL(0x8A3C, rotate_tetrimino)`), which the hybrid needs anyway to hand
  control between C and assembly, and which keeps each call site's original address in
  the source. The alternative is to leave `$0100-$01FF` out of parity (Q4).

### 4.5 Platform layer

Only what Tetris uses:

- **PPU:** registers, 2 KB nametable RAM, palette, OAM and OAM DMA, CHR bank switching
  through MMC1 writes, and a renderer that draws a whole frame from VRAM and OAM after the
  frame's writes. Tetris never splits the screen mid-frame: it reads `PPUSTATUS` only to
  wait for vblank and to reset the address latch, and never polls sprite 0.
- **Controller:** the `$4016` shift register, so `pollController_actualRead` sees the same
  bits.
- **APU:** register writes are captured from the start; turning them into sound can wait
  (Q11). Audio isn't hashed, but the music driver's state at `$0680-$06FF` is, so the
  driver itself must be exact from the start.

### 4.6 The decomp reports coverage of the original

The core also exports the coverage connector API ([connectors.md](../../../docs/connectors.md#the-api))
with a `PRG` region, but the flags are set by the C code: a `BLOCK(0x9A6B)` marker at the
top of each C basic block that corresponds to a block in the original marks those original
bytes as run (and in hybrid mode the interpreter marks what it runs, as Mesen does).

That gives three things with no new decomp-dj features:

- Coverage maps for the decomp core use the same symbol map as for Mesen.
- A segment that runs a block on Mesen but not on the decomp shows a control-flow
  divergence even before RAM differs.
- Every C block names the original address it came from, which makes review against the
  disassembly quick. `[tick]` also works, since the main loop's block is marked.

## 5. How decomp-dj is used

| Need | Tool |
| --- | --- |
| Pick what to translate next | [functions.csv](functions.csv), `dj gaps`, `dj function`: leaves first inside well-covered areas |
| Check a routine | `dj run --core <decomp>`: every segment that runs it must keep its hashes |
| Find where it goes wrong | A per-frame RAM diff of two cores over one segment chain: the first frame and addresses that differ. `player/examples/ramtrace.rs` prints chosen bytes per frame on one core; this needs all 2 KB on two cores. A small tool in `.decomp/tools/` first (Q8) |
| Then which routine | MCP `trace` on Mesen around that frame, beside the decomp's `BLOCK` coverage |
| Unit tests per routine | MCP `calls(addr)` already returns, per call, the registers on entry and return, the memory read before written (inputs) and the memory written (outputs). Saved as fixtures in `.decomp/tests/`, they test a C routine on its own, without playing anything |
| Reach code nothing runs | New segments, written by hand or by an agent through `dj mcp` (`seek_goal`, `fuzz`, drafts). A-type endings need high scores, B-type endings need cleared B-type games (the larger ones at level 9 and greater heights); `[WRITE]` pokes are fine where play would take hours, and are counted apart |
| Mark what can't run | `examples/tetris/unreachable.txt`: the two-player paths and the unreferenced code the disassembly names (`unreferenced_func1`, `unreferenced_code2`, `unreferenced_soundRng`) |

## 6. Phases

Each phase ends when every clean segment still has parity on the decomp core.

| Phase | Work | Done when |
| --- | --- | --- |
| **0. Groundwork** | A script that fetches the reference at the pinned commits into `build/ref/`, builds it and checks the PRG+CHR against your ROM. Generate the symbol map; add `[[symbols]]`, `unreachable.txt` and a `[tick]` on the main loop to `examples/tetris` (left for you to commit). Record baseline coverage | `dj summary` names functions; lag measured |
| **1. Shell core** | Platform layer, 6502 interpreter, libretro glue, connector API, no C game code | All 23 segments keep their hashes on the shell core; screenshots match Mesen's |
| **2. Parity tools** | Two-core RAM diff; call capture into fixtures; the `CALL`/`BLOCK` macros; `ram.h` generated from `tetris-ram.asm` | A deliberately broken routine is found by the diff and its fixture |
| **3. System routines** | RNG, controller polling, memset, MMC1, PPU copies (`bulkCopyToPpu`, `copyToPpu`), the jump-table dispatchers as `switch`, NMI, boot | Those routines in C, parity holds |
| **4. Gameplay** | Shifting, rotating, dropping, `isPositionValid`, locking, completed rows, scoring, lines and level, spawning and picking the next piece, B-type garbage and goal check | 81% covered already; parity on the line-clear and lose-game segments |
| **5. Menus, demo and rendering** | Legal and title screens, game type and level menus, demo playback, `render_mode_play_and_demo`, line-clear animation, palettes | The demo segment (6000 frames, no input) has parity |
| **6. Audio driver** | The music bytecode interpreter and sound effects (99 small routines) | Music state RAM has parity on every segment |
| **7. Endings and high scores** | New segments first (Phase 7 is blocked on coverage), then translation | Those areas covered and with parity |
| **8. Pure C** | Compile out the interpreter; a standalone frontend (SDL) if wanted; APU synthesis | The decomp core has parity with no 6502 code in it |

Phases 3 to 6 can overlap once Phase 2 is done; the order above follows coverage and
dependencies (gameplay calls system routines, rendering reads gameplay state).

## 7. Layout

Proposed, assuming the source lives in this folder (Q7):

```
examples/tetris/.decomp/
  README.md  PLAN.md  QUESTIONS.md  functions.csv
  Makefile                 builds into ../build/decomp/
  include/ram.h            generated from tetris-ram.asm
  src/game/                one file per area, routine names from the disassembly
  src/platform/            PPU, controller, APU capture, libretro glue, connector API
  src/hybrid/              6502 interpreter and the asm/C routine table (removed in Phase 8)
  tests/                   fixtures captured with `calls`
  tools/                   dbg2csv.py, fetch-reference.sh, ramdiff
examples/tetris/build/ref/     the reference checkout and build (gitignored)
examples/tetris/build/decomp/  the decomp core (gitignored)
```

## 8. Risks

- **Lag frames.** If the main loop ever overruns a frame, the NMI runs in the middle of
  game logic and the coroutine model has to interrupt C at the same point. Measured in
  Phase 0 before anything depends on it.
- **Frame boundary.** RAM is compared where Mesen's `retro_run` returns; the shell core has
  to stop at the same point in the frame. Phase 1 exists to settle this and the boot
  frames (two vblank waits after reset) before any game code is written.
- **Stack bytes.** Exact stack contents need the `CALL` discipline everywhere; one missed
  call changes the hash without changing behaviour. The fallback is Q4.
- **Coverage-driven blind spots.** A third of the code isn't run by any segment yet; those
  routines can't be checked until segments reach them (Phase 7).
- **decomp-dj changes underneath.** A connector rebuild changes the setup id of the Mesen
  project ([roadmap](../../../.agents/ROADMAP.md), versioning). Parity runs should pin the
  Mesen build they compare against.
- **Licensing.** No ROM bytes go in the repository, but the disassembly has no licence, so
  copying its names and comments into published C needs a decision (Q5).

## 9. decomp-dj improvements this surfaced

For the roadmap, not part of this plan's work unless you say so:

- `ca65-dbg` should read disassemblies whose code isn't in a `CODE` segment, for example a
  `code_segments` key, and needs a way to tell code labels from data.
- Comparison cores (roadmap, "Comparison and parity"): a second core in `config.toml`,
  `dj parity`, and a first-differing-frame report. The RAM diff in Phase 2 is a prototype.
- Masks for parity, such as leaving the stack page out, without changing the project's
  own `[hash]` regions.
- Exporting `calls` results as test fixtures.
