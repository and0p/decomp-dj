# Plan: a C reimplementation of Thwaite as a comparison core

Status: draft. Written 2026-11-11, rewritten 2026-11-15 for parity by breakpoints
([yield-parity/PLAN.md](yield-parity/PLAN.md)). Not started.

Decided by the developer:

- 2026-11-11: the C is written from the GPL source (a reimplementation, not clean room);
  generated data is committed as C tables; no sound.
- 2026-11-15: built on the breakpoint design; drawn with OpenGL (no software renderer).

This plan changes nothing in dj. It needs the breakpoint design built up to its step 6
(`dj_handle_event` in dj), and the nested OpenGL contexts of its step 4. Where this plan
finds a problem in that design, it says so under "For the breakpoint plan".

## Goal

`examples/thwaite/` gets a second core: Thwaite rewritten in C, drawn with OpenGL, built as
a libretro core that exports the comparison-core API. `dj parity` and the GUI check it
against the original ROM (Mesen) on the project's 12 segments. It becomes the example of
the "decomp" half of decomp-dj, and a second real decomp for the breakpoint design beside
Tetris: one whose lag frames are judged rather than skipped.

What it is:

- **Its own C code**, organised the way C wants (structs, enums, explicit state machines),
  not a line-by-line translation of the 6502. Each C function names the original routine
  it stands in for (`/* orig: moveCrosshairPlayerX */`) so failures point at both.
- **Its own memory layout.** One `struct game` holds all state. A generated table maps its
  fields to the original's RAM symbols, for copying the baseline, writes, comparing, and
  diffs.
- **The original graphics for now**: CHR, palettes, nametables, texts and level tables
  converted once from the GPL sources (`tilesets/*.png`, `texts.txt`, the `.s` tables) and
  committed as C tables. The converters stay in `tools/` to regenerate them; the core
  builds with only a C compiler and CMake, and never reads the ROM.
- **Written from the source,** in its own structure. A reimplementation, not clean room.
- **GPL-3.0**, like Thwaite (and the repo, once relicensed).

Not goals: sound (the core outputs silence), cycle accuracy, the PAL/Dendy speed paths (it
runs as NTSC), replacing the art, a standalone (non-libretro) build, a software renderer.

## Layout

```
examples/thwaite/.decomp/
  README.md               what it is, how to build, how it maps to the original
  CMakeLists.txt          builds thwaite_decomp_libretro.{dylib,so,dll}
  build.sh                runs cmake (CI and the README use it)
  tools/                  run by hand when the pinned Thwaite commit changes
    gen_assets.py         tilesets/*.png -> CHR; palettes, nametables, level/sine tables, texts
    gen_map.py            thwaite.dbg -> RAM symbols (name, address, size), the breakpoint
                          addresses (from source lines), and each wait loop's PC range
  src/
    data/                 their output, committed: chr.c, tables.c, texts.c, map.c
    game.h                struct game: every piece of game state
    main.c                the step loop and the screen state machine (orig: main.s)
    title.c practice.c cutscene.c levels.c missiles.c explosion.c smoke.c
    scurry.c bg.c math.c random.c pads.c mouse.c bcd.c
    gfx.c                 the "screen": BG tilemap, palette, sprite list (no PPU emulation)
    gl.c                  OpenGL renderer for gfx.c (SET_HW_RENDER)
    libretro.c            the libretro API, and plain play (one step per retro_run)
    events.c              dj_breakpoints, dj_handle_event, dj_handle_write
    compare.c             copy from the baseline, compare by policy, dj_parity_diffs
```

`examples/thwaite/config.toml` gets:

```toml
[[compare]]
name = "decomp"
path = ".decomp/build/thwaite_decomp_libretro"
# sync left out: the events default. No preroll: the decomp copies at the jump.
```

## How it runs

### Steps, not frames

The original's screens are nested loops, each waiting for the NMI (`lda nmis / cmp nmis /
beq`): the title screens, the practice menu, two cutscene waits, pause, and the main game
loop. In C each becomes a state in an explicit state machine. One **step** runs the code
from one wait's exit to the next wait's exit, the same span the original runs between two
of its NMIs.

- **Under dj**, the decomp runs a step when the baseline reaches a wait's exit (below). A
  lag frame needs no special handling: the step simply arrives late.
- **On its own** (RetroArch, or `dj run` with the decomp as the core), `retro_run` runs one
  step. It never lags.

A save state is a `memcpy` of `struct game`. Copying the baseline is "pick the state, fill
the fields".

### Breakpoints

`dj_breakpoints` returns a fixed table, generated from `thwaite.dbg` by source line, so a
rebuilt ROM moves the addresses with it. All in `PRG`.

| Kind | Where | What the decomp does |
| --- | --- | --- |
| **Wait exit**, one per wait loop: `game` (`main`), `pause` (`pauseScreen`), `title` ×3, `practice`, `cutscene` ×2 | The first instruction after the `beq` that spins on `nmis`. It runs once per wait, never in the spin (the Mesen spike saw a breakpoint inside a spin hit 3,891 times in a frame) | Runs its pending step, compares, answers |
| **NMI-counter read**: `updateAllExplosions`, `drawBalloonNumberX`, the missile-target drawing, `updateSmoke`, `cut_handle_state_wait_a` | The `lda nmis` that reads the counter for logic | Takes `nmis` through the handle and queues it for the step in progress; answers "pending" |

**Why the counter reads.** Thwaite's NMI handler is only `inc nmis`, but game logic reads
the counter. Which half of the explosions advances this frame is the important one; the
others decide sprite flicker and the balloon's float. On a normal frame the counter is
fixed for the whole step. On a lag frame the NMI lands mid-step, and a read can fall
before or after it, depending on 6502 cycle counts a port cannot reproduce. With a
breakpoint at each read, the decomp uses the value the original actually read. Thwaite's
7 lag frames in the segments can then be judged instead of answered "unjudged".

The breakpoint plan's notes say each place where timing-dependent data enters the logic
needs a breakpoint. For Thwaite these are the only such places apart from input (below).

### Events

| Event | What the decomp does |
| --- | --- |
| jump, power-on | Resets to power-on. Boot (the PPU warm-up and PAL detection, timing loops on the PPU's status) isn't modelled. The decomp answers "pending" until the first `title` wait exit, copies there and answers "unjudged" ("boot isn't modelled"). Boot's frames show as skipped, with that reason |
| jump, a state or "copy again" | Reads the registers. If the PC is inside a wait loop's spin (the generated PC ranges), the original's step is done, so the decomp copies now, parks at that wait's exit and answers "unjudged". The next wait-exit hit then has an empty step, and is compared. Otherwise (a lag frame, boot) it answers "pending" and copies at the next wait exit. No pre-roll is needed: after boot, the original waits once per frame in 11,013 of 11,020 frames (breakpoint plan, "What was measured"), so a jump almost always finds it in a spin. Milestone 0 checks the frame ends directly |
| console reset | Treated as a jump. The decomp is parked at its last wait exit while the original was in the spin after the step, not where the game reset from. No segment has a `[RESET]` today |
| break, wait exit `W` | Runs the pending step with the queued counter values. If the state machine reached a different wait than `W`, that's a mismatch in itself ("the decomp went to `title`, the original to `pause`"). Otherwise it compares (below). Then it draws the step's picture and parks at `W` |
| break, counter read | Queues `nmis`; "pending" |
| frame end | "Pending" |

The verdict after a copy is "unjudged", never "pending". Otherwise the first match would
credit frames that were compared with nothing (breakpoint plan, section 4).

### Input

A step reads the pads right after its wait exit (`read_pads` comes before the logic). dj
sets input per frame through libretro. So the decomp polls input at each wait-exit event
and keeps it for the step that starts there, which it runs at the next wait exit. That's
the frame `read_pads` actually ran in, lag frames included: the read is a few hundred
cycles after the exit, so it is in the same frame.

The SNES mouse (the Mouse segment) is read the same way, through libretro's mouse device.
Mesen reads mouse motion from libretro port 1 whatever port the mouse is in (the example's
README), and the decomp does the same.

If this turns out wrong anywhere, the breakpoint plan's fallback applies: a breakpoint in
`read_pads` after the raw bytes are read, copying them through the handle.

### Writes

`dj_handle_write` takes both kinds of `[WRITE]` line. A write to an `ignore` byte (below)
is answered "pending" and dropped. Outside `RAM`, it's "halt" with a reason.

- **With an address at a wait exit** (`@PRG:<address>`): the decomp is parked exactly
  there. It applies the bytes to the mapped field and answers "pending".
- **With no address (between frames), with the original in a spin:** the original's step
  is done, and the write lands after it. The decomp queues the write and applies it after
  it runs that step at the next wait exit, before comparing. This is exact. "Jump to
  Saturday" (`gameDay`) works without changing the segment.
- **Anywhere else** (a lag frame, an address mid-step): the decomp goes stale and copies
  at its next wait exit ("unjudged", with the reason).

### What is compared

At every wait exit, through one generated table of the original's RAM symbols, each with
a size and a **policy**:

| Policy | Used for |
| --- | --- |
| `exact` | persistent game state: scores, clock, houses, missiles, crosshairs, the random number, the level tables in RAM |
| `when(screen)` | state that means something only in some screens (the `missilesOverlay` area, reused by cutscenes and the practice menu) |
| `baseline` | `nmis`: the NMI's, not the logic's. Taken from the baseline at each wait exit and never compared. Pause writes it too (`sta nmis`), and that write is compared |
| `ignore` | scratch: zero-page temporaries, the stack, `xferBuf`, PAL detection state, all of Pently's sound state |
| `sprites` | the OAM buffer, which holds the step's sprites at the wait exit, compared as a set of sprites (milestone 2 decides how strict) |

- **Copying the baseline** reads `RAM` through the handle's `memory()` and fills every
  field that isn't `ignore`.
- **A mismatch** fills `dj_parity_diffs`: one entry per differing field. `label` is the C
  name (`game.missiles[5].y`), `symbol` the original's (`missileYHi`), `address` the
  byte, and both sides' bytes.
- **`retro_get_memory_data(SYSTEM_RAM)`** is a 2 KB view of `struct game` in the
  original's layout, refreshed after each step, so dj's RAM views work on the decomp too.

### Graphics (OpenGL)

`gfx.c` is a tiny "screen" the game code writes to: a 64x30 tile BG map (two nametables;
cutscenes scroll), 32 palette entries, a sprite list, and a scroll position. It is not a
PPU: no registers, no VRAM address latches. The game code calls things like
`gfx_put_text`, `gfx_set_tiles` and `gfx_sprite`.

`gl.c` draws it into the frontend's framebuffer (`SET_HW_RENDER`, OpenGL 3.2 core, GLSL
150, which dj gets on macOS, Linux EGL and Windows). CHR is an index texture and the
palette a lookup texture with the NTSC palette Mesen uses by default. BG is one quad, and
sprites are instanced quads with the 8-per-line limit applied. Output is 256x240.

- **When it draws:** at the end of each step, inside the wait-exit event, by calling the
  libretro video callback. dj shows the latest picture at the baseline's frame end
  (breakpoint plan, section 8). After a copy it draws at once from the copied state.
- **Inside the baseline's frame:** the decomp's GL calls run while the baseline is in
  `retro_run`, so they need the breakpoint plan's nested contexts (its step 4, open
  problem 3). This decomp is the first core to draw that way, so it is that problem's test
  case. Milestone 0 tries it before any game code depends on it.
- **When dj doesn't want a picture** (`GET_AUDIO_VIDEO_ENABLE`: seeks, `dj parity`), the
  decomp skips drawing.
- **Flicker:** Thwaite flickers explosions and smoke by cycling sprites. The default
  reproduces it. A core option, `thwaite_decomp_flicker = original|blend`, draws them
  translucent instead, as a first taste of what a decomp can change.

### Sound

None. The core sends silence at 48 kHz so frontends stay in time. The original's calls
into Pently (`pently_start_sound`, `pently_start_music`, ...) become empty stubs with the
original's names, so sounds can be added later where they play.

### What the decomp exports

| Function | In the decomp |
| --- | --- |
| `dj_api_version`, `dj_build_id` | the version the breakpoint API ships with; `thwaite-decomp@<git hash>` |
| `dj_breakpoints` | the generated table above |
| `dj_handle_event` | as above |
| `dj_handle_write` | as above |
| `dj_parity_diffs` | as above |
| `dj_region_count`, `dj_get_region`, `dj_clear` | coverage of its own C code: `SRC`, one flag byte per instrumented block (a `COV()` macro at each original routine's function and its branches), and `RAM` (the 2 KB view). dj ignores a comparison core's coverage today; this is for when it doesn't |
| `dj_trace_start`, `_stop`, `_read` | write events in the original's address space, from the 2 KB view, so the decomp's side of a failure can be traced too. dj traces only the baseline today |

`dj_sync`, `dj_write`, `dj_check_parity`, `dj_run` and the tick functions are gone from the
API, so the decomp has none of them. A routine the decomp hasn't written yet answers
"halt" with its name, where `dj_run` used to fail.

## Milestones

Each ends with `dj parity examples/thwaite --compare decomp` on the segments it covers.
Every segment can be checked from its own start (the decomp copies at the jump), so
gameplay doesn't wait on the cutscenes that come before it in the tree.

Until the breakpoint design reaches its step 6, game code is written and tried on its own
(`dj run` with the decomp as the core, or RetroArch). Parity checking starts when step 6
is in dj.

0. **Skeleton and checks.**
   - The core: libretro boilerplate, the GL renderer showing the CHR, every function
     above stubbed, the converters and their committed tables, `build.sh`, `[[compare]]`
     in config.toml.
   - The breakpoint table checked with the Mesen spike harness
     (`player/examples/mesen_spike.rs`, which takes a list of `PRG` addresses) on all 12
     segments. Each wait exit should be hit once per frame on its screen, apart from lag
     frames. The counter reads should be hit only in their steps. Every frame end except
     the lag frames should have the PC in a spin range.
   - Drawing from inside the baseline's frame, as soon as the breakpoint plan's step 4
     lands. If nested contexts don't work on a platform, that's a finding for the
     breakpoint plan, not a reason for a software renderer here.
1. **Boot and menus**: the title screens, the 2-player and mouse paths, the practice
   menu. Segments: Boot (from the first title wait), 2 Players, Mouse, Practice
   open/cancel.
2. **Gameplay**: new level, crosshairs, player and enemy missiles, balloons, explosions,
   smoke, villagers, scoring, the clock, BG updates, pause, game over, and the counter
   breakpoints. Segments: Defend, Idle, Pause, Practice play, Jump to Saturday (a queued
   write).
3. **Cutscenes and between levels**: DTE text from `texts.txt` and `cutscripts.txt`, the
   level reward, silo and house rebuilding. Segment: the opening cutscene. All 12 pass,
   with the 7 lag frames judged.
4. **Diffs and tracing**: `dj_parity_diffs` and the trace hook complete, checked with
   `dj parity --explain` on a build that is broken on purpose.
5. **CI and docs**: a CI job builds Mesen and the decomp and runs `dj parity` on the
   example; a README for the decomp; examples/thwaite/README and the ROADMAP updated.

New segments that reach more of the game (later days, a rebuilt house, a high score, and
heavier waves for more lag frames) are worth adding as the decomp gets there. The
coverage map shows where.

## For the breakpoint plan

What this decomp needs from [yield-parity/PLAN.md](yield-parity/PLAN.md), or tests in it:

- **Nested OpenGL contexts** (its section 11, open problem 3). This decomp draws only with
  OpenGL, from inside the baseline's frame.
- **`registers()` at a jump.** Copying at the jump relies on the PC between frames. The
  plan has registers in the handle for every event; the 6502 layout (its open problem 1)
  needs to be fixed before this decomp reads it.
- **Queued writes.** A between-frames write applied after the next step is something any
  core parked at a wait exit can do. It may belong in the plan's notes for a core's author
  or in the helper header.
- **Its build order, steps 7 to 9,** test with the Tetris decomp. Thwaite is a second test
  there, and the first whose lag frames are judged through counter breakpoints rather than
  answered "unjudged".

## Risks

- **The counter breakpoints miss a read.** If a lag-frame step reads `nmis` somewhere not
  listed, a lag frame mismatches; the diff names the field. The converter finds every
  `lda nmis` in the source, and milestone 0 lists which ones are in steps.
- **A between-frames write while the original isn't in a spin.** Answered "unjudged" and
  copied again. Rare: the segments have 7 lag frames.
- **The `missilesOverlay` reuse:** cutscenes and the practice menu reuse missile RAM for
  their own variables, and the policy table has to know which layout is live.
- **Sprite order:** if Thwaite's OAM rotation is part of its state (it decides flicker),
  it has to match exactly, and the `sprites` policy may become `exact`.
- **Boot isn't judged,** up to the first title wait. Modelling the PAL detection's timing
  loops would need cycle counts. Milestone 0 measures how many frames that is; the
  routines that run there are reset-time setup.
- **Committed tables drift** if the pinned Thwaite commit changes. The converters are
  rerun by hand then, and the README says so.
