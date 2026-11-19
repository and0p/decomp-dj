# Plan: a C reimplementation of Thwaite as a comparison core

Status (2026-11-16): all milestones done (0 to 5). Every screen the
segments reach is in parity on every frame: the legal screen, the title, the practice menu,
gameplay (pause, game over, clearing a level, balloons, MIRVs, the silo rebuild, the hour
changing), the cutscenes, and the day changing with its house rebuild. Only boot's first
frames aren't judged. The pictures match the original's pixel for pixel, `dj parity
--explain` traces a failure to the original's instruction that wrote each differing byte,
and CI checks parity on every change. Results are under "Milestone 0 results" to
"Milestone 5 results". Written 2026-11-11, rewritten 2026-11-15 for parity by breakpoints
([yield-parity/PLAN.md](yield-parity/PLAN.md)).

Decided by the developer:

- 2026-11-11: the C is written from the GPL source (a reimplementation, not clean room);
  generated data is committed as C tables; no sound.
- 2026-11-15: built on the breakpoint design; drawn with OpenGL (no software renderer).

This plan changes nothing in dj. It is built on the breakpoint design's events
(`docs/connectors.md#events`), which dj has had since 2026-11-15. Where this plan finds a
problem in that design, it says so under "For the breakpoint plan".

## Goal

`examples/thwaite/` gets a second core: Thwaite rewritten in C, drawn with OpenGL, built as
a libretro core that exports the comparison-core API. `dj parity` and the GUI check it
against the original ROM (Mesen) on the project's segments. It becomes the example of
the "decomp" half of decomp-dj, and a second real decomp for the breakpoint design beside
Tetris: one whose steps that cross a frame boundary are judged rather than skipped.

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
  step that crosses a frame boundary (a lag frame, or a screen's setup) needs no special
  handling: it simply arrives late.
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
| **NMI-counter read**: `updateAllExplosions`, `drawBalloonNumberX`, the missile-target drawing, `updateSmoke` (two, one in a loop), `cut_handle_state_wait_a`, `load_cutscene`, `titleScreen` | The `lda nmis` that reads the counter for logic | Takes `nmis` through the handle and queues it for the step in progress; answers "pending" |
| **Input**: `read_pads`, `read_mouse` | Where the raw bits are complete: after `read_pads` calls `read_pads_once` ($00-$01 hold the two ports), and `read_mouse`'s `rts` ($01-$03 hold the report, X the port) | Takes the raw bytes through the handle and queues them for the step in progress; answers "pending" |

| **Sound leak**: `splitBalloon`, after `jsr pently_start_sound` | The `ldx $02` that reloads the balloon's slot, where the sound engine has just put the split sound's channel (0 or 4, depending on its own state) | Takes `$02` and queues it; the missile loop goes on from that index, as the original's does |

The generator finds 9 wait loops, 8 counter reads, the 2 input points and the sound leak. One wait,
`ppu_wait_vblank`, is in a routine nothing calls; it stays in the table and is never hit.
The waits in `getTVSystem` (PAL detection, at boot) are left out.

**Why the counter reads.** Thwaite's NMI handler is only `inc nmis`, but game logic reads
the counter. Which half of the explosions advances this frame is the important one; the
others decide sprite flicker, the cutscene's scrolling and the balloon's float. In most
steps the counter is fixed for the whole step. In a step that crosses a frame boundary the
NMI lands mid-step, and a read can fall before or after it, depending on 6502 cycle counts
a port cannot reproduce. With a breakpoint at each read, the decomp uses the value the
original actually read. Milestone 0 found such steps at every screen change, and reads
after the NMI in four of them.

The breakpoint plan's notes say each place where timing-dependent data enters the logic
needs a breakpoint. For Thwaite these are the only such places apart from input (below).

### Events

| Event | What the decomp does |
| --- | --- |
| jump, power-on | Resets to power-on. Boot (the PPU warm-up and PAL detection, timing loops on the PPU's status) isn't modelled. The decomp answers "pending" until the first wait exit (frame 5, `screen_off_title_palette`), copies there and answers "unjudged" ("boot isn't modelled"). Boot's 5 frames show as skipped, with that reason |
| jump, a state or "copy again" | Reads the registers. If the PC is inside a wait loop's spin (the generated PC ranges), the original's step is done, so the decomp copies now, parks at that wait's exit and answers "unjudged". The next wait-exit hit then has an empty step, and is compared. Otherwise (mid-step, boot) it answers "pending" and copies at the next wait exit. No pre-roll is needed: in the 12 segments every frame end after boot is in a spin except 8, each in a screen's setup (milestone 0) |
| console reset | Treated as a jump. The decomp is parked at its last wait exit while the original was in the spin after the step, not where the game reset from. No segment has a `[RESET]` today |
| break, wait exit `W` | Runs the pending step with the queued counter values. If the state machine reached a different wait than `W`, that's a mismatch in itself ("the decomp went to `title`, the original to `pause`"). Otherwise it compares (below). Then it draws the step's picture and parks at `W` |
| break, counter read | Queues `nmis`; "pending" |
| break, input | Queues the raw bytes; "pending" |
| a step into code the decomp doesn't have | "Unjudged", with what isn't written ("newGame and the practice menu (milestone 2)"). The decomp stops following, and copies the original again at the next wait it has. dj's docs give "an overlay not decompiled" as a reason for "unjudged"; this is the same case, and it lets every segment be checked as far as the decomp goes |
| frame end | "Pending" |

The verdict after a copy is "unjudged", never "pending". Otherwise the first match would
credit frames that were compared with nothing (breakpoint plan, section 4).

### Input

Under dj, the decomp takes the controllers' raw bits from the original, at the input
breakpoints, and runs its own version of everything after: new presses, mouse buttons,
mouse detection and motion. Two reasons:

- **The frame.** The decomp has to use the input of the frame the original read it in. In
  the step from the cutscene into the game the wait exit is in frame 62 and `read_pads`
  runs in frame 63, after `setupGameBG` crosses the frame boundary (milestone 0).
- **What is in the port is the emulator's business.** `read_mouse` runs on both ports in
  every menu step, joypad or not, and reads past a joypad's 8 bits; a mouse's report
  depends on how Mesen turns libretro's mouse motion into SNES mouse counts. Neither is
  game logic. (Polling libretro at the input breakpoint was the first design; reading the
  raw bits is simpler and exact.)

Played on its own (RetroArch), the decomp builds the raw bits from libretro's joypad, and
reports no mouse.

### Writes

`dj_handle_write` takes both kinds of `[WRITE]` line. A write to an `ignore` byte (below)
is answered "pending" and dropped. Outside `RAM`, it's "halt" with a reason.

- **With an address at a wait exit** (`@PRG:<address>`): the decomp is parked exactly
  there. It applies the bytes to the mapped field and answers "pending".
- **With no address (between frames), with the original in a spin:** the original's step
  is done, and the write lands after it. The decomp queues the write and applies it after
  it runs that step at the next wait exit, before comparing. This is exact. "Jump to
  Saturday" (`gameDay`) works without changing the segment.
- **Anywhere else** (mid-step, or an address mid-step): the decomp goes stale and copies
  at its next wait exit ("unjudged", with the reason).

### What is compared

At every wait exit, through one generated table of the original's RAM symbols, each with
a size and a **policy**:

| Policy | Used for |
| --- | --- |
| `exact` | persistent game state: scores, clock, houses, missiles, crosshairs, the random number, the level tables in RAM, and the OAM buffer, byte for byte (it holds the step's sprites at the wait exit; byte order turned out to match exactly, so no looser rule was needed) |
| `play` | the playfield's pending background update (`houseXferDstHi`, `houseXferDstLo`, `houseXferBuf`, inside `xferBuf`): compared on the playfield's screens (game, pause, practice menu), copied but not compared elsewhere, where the title's stripes use the same bytes |
| `baseline` | `nmis`: the NMI's, not the logic's. Taken from the baseline at each wait exit and never compared. Every read of it in the logic comes from a counter breakpoint, so pause's `sta nmis` needs no model |
| `scratch` | copied, never compared: zero page `$00`-`$0F` (the original's routines hand values to each other there, so the decomp keeps it as `g->zp`), and popslide's length and terminator bytes around the update |
| `cut` | the text decoder's line (`dte_output_buf`, up to its control character) and `popslide_used`, with the popslide packets from `$0108` to it: compared in a cutscene's frame loop (`load_cutscene`'s second wait), where the decomp builds them as the original does; copied elsewhere |
| `ignore` | the rest of `xferBuf`, popslide's other bookkeeping, all of Pently's sound state |

The decomp's state is its own structs: `struct missile`, `struct explosion`, `struct smoke`,
`struct villager`, `struct crosshair`. The original keeps each as parallel arrays
(`missileXHi`, `missileYHi`, ...), so the table maps them with a stride, and a difference is
named in both layouts: `game.missiles[4].time (missileTime)`.

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
palette a lookup texture with the NTSC palette Mesen uses by default. One full-screen pass
puts together the BG and a sprite layer the CPU works out the console's way: at most 8
sprites a line, and at each pixel the lowest-numbered opaque sprite wins before its
priority counts, so a sprite behind the BG still hides a higher-numbered one in front (the
cutscene's trees need it). The cutscenes' mid-frame changes (pattern table at sprite 0's
hit, the scroll 16 lines later) are a row split. Output is 256x240.

- **When it draws:** at the end of each step, inside the wait-exit event, by calling the
  libretro video callback. The original copies its pending VRAM updates and sets the scroll
  right after the wait, before the frame is drawn, so the decomp's picture at the end of a
  step already shows what is pending (`draw_bg_update`, `popslide_draw`); the blit at the
  next step's start does the RAM side. dj shows the latest picture at the baseline's frame end
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
| `dj_trace_start`, `_stop`, `_read` | after each step, a write event (region `RAM`, `cycle` the step's number) for every byte of the 2 KB view the step changed, so the decomp's side of a failure can be traced too. dj traces only the baseline today, so nothing reads it yet |

`dj_sync`, `dj_write`, `dj_check_parity`, `dj_run` and the tick functions are gone from the
API, so the decomp has none of them. A step into code the decomp hasn't written yet
answers "unjudged" with what is missing (Events, above), and "halt" is kept for what the
decomp can't go on from at all (a handle without RAM, a write outside RAM).

## Milestones

Each ends with `dj parity examples/thwaite --compare decomp` on the segments it covers.
Every segment can be checked from its own start (the decomp copies at the jump), so
gameplay doesn't wait on the cutscenes that come before it in the tree.

0. **Skeleton and checks.**
   - *Done:* the core: libretro boilerplate, the GL renderer showing the CHR, every
     function above stubbed, the converters and their committed tables, `build.sh`. It
     loads and draws in `dj play`.
   - *Done:* the breakpoint table checked on all 12 segments with
     `player/examples/thwaite_breaks.rs`, against the version 3 Mesen connector. Results
     below.
   - *Done 2026-11-16:* the `[[compare]]` entry in config.toml (`decomp`).
   - *Done 2026-11-16:* drawing from inside the baseline's frame, on macOS: the decomp's
     pictures match the original's (milestone 1 results). Linux and Windows aren't tried.
1. **Boot and menus.** *Done 2026-11-16* for the legal screen and the title, with the
   2-player and mouse paths. Segments: Boot, and the title part of 1 Player, 2 Players,
   Mouse, Practice open/cancel, and Idle's return to the legal screen. The practice menu
   moved to milestone 2: it is built from the game's own routines (`moveCrosshairPlayerX`,
   `doStateNewLevel`, `setupGameBG`, the status bar).
2. **Gameplay.** *Done 2026-11-16* as far as the segments reach: new level, crosshairs
   (joypad and mouse), player and enemy missiles, explosions, smoke, villagers and threats,
   the clock, background updates, pause, game over, the practice menu, and the queued
   write. Segments: Defend, Idle, Pause, Practice open, cancel and play, Jump to Saturday,
   and the game part of the opening cutscene's segment. Ported but not reached by any
   segment (milestone 2 results): balloons and their split, MIRVs, clearing a level, silo
   rebuilding, and the next hour.
3. **Cutscenes and between levels.** *Done 2026-11-16:* `cutscene.c` (the script engine,
   the DTE decoder working in place as the original's does, the villagers' roles, the
   scenery), the end-of-day choice of cutscene, the house rebuild and its name bar. The
   level reward and silo rebuild were done in milestone 2. All segments pass on every
   frame, the steps that cross a frame boundary included.
4. **Diffs and tracing.** *Done 2026-11-16:* `dj parity --explain` on the build broken on
   purpose traces each difference to the original's writer; the diffs give region and
   address (not `symbol`), and failures where the decomp went another way give the diffs
   of its state too; the decomp's trace hook; dj's own headers (`dj_connector.h`,
   `dj_compare.h`) in place of the decomp's copy of the API.
5. **CI and docs.** *Done 2026-11-16:* the CI's Thwaite job builds the decomp after the
   replay and runs `dj parity` (Linux with Mesa's EGL, and macOS); the decomp's README; the
   ROADMAP; a pointer from docs/connectors.md's CI section. examples/thwaite/README is the
   developer's to update (wording suggested).

New segments that reach more of the game (later days, a rebuilt house, a high score, and
heavier waves that might lag in play) are worth adding as the decomp gets there. The
coverage map shows where. No balloon appears in today's segments, so
`drawBalloonNumberX`'s counter read is never hit.

### Milestone 5 results

- **CI** (`.github/workflows/ci.yml`): the "Replay Thwaite, check its decomp" job, after
  replaying every segment from power-on, installs EGL and Mesa (Linux), builds the decomp
  with `build.sh`, and runs `dj parity examples/thwaite --compare decomp --rom ... --core
  ...`. The decomp's build is new each run, so every segment is checked; a failing frame
  fails the job.
- **Tried in Ubuntu 22.04** (Docker, arm64; the runner is x86_64), from a copy of the
  working tree: Mesen, dj and the decomp built, and `dj parity` passed all 15 segments
  (27,193 frames, 52 s, 0 failed) with the decomp's OpenGL 3.2 context from Mesa through
  EGL's surfaceless platform. Not tried: the x86_64 runner itself, and the macOS runner,
  where dj's context is CGL inside a virtual machine. Push to run them.
- **Docs:** the decomp's README (CI, `--explain`, the API headers), the ROADMAP's table, and
  docs/connectors.md's CI section, which now points at this job as a worked example.

### Milestone 4 results

`dj parity --explain` on the build broken on purpose (held keys count as new presses;
enemy smoke every 21 frames, not 20), 2026-11-16. It fails every segment that gets past
boot's first frames, and each failure now names the original's code to compare with:

| Failure | The difference, and the original's writer `--explain` finds |
| --- | --- |
| Idle, frame 83 | `game.missiles[4].time (missileTime)`: `moveMissileNumberX+0xc1` (`DEC $04C6,X`), the smoke countdown, which is the planted 21 |
| 2 Players, frame 7 | `game.new_keys (new_keys)`: `read_pads+0x17` (`STA $36,X`), the planted bug; then `numPlayers` (`title_move_numPlayers`) and the arrow's sprite (`title_draw_sprites`), which follow from it |
| Pause, frame 32 | the decomp read the NMI counter where the original didn't; its state partway through the step: `new_keys` (`read_pads`) holds START again, so it left pause a step early |

The search stops at the breakpoint where the decomp gave its verdict ("the search for
writers stops at wait:main"), so it doesn't pick up what the original wrote after it in
the frame.

- **Found and fixed: the diffs weren't traceable.** dj takes a diff's `symbol` *instead
  of* its address (docs/connectors.md), and the decomp gave both, so dj dropped the
  address: every difference was "given by a data symbol alone". The decomp now gives
  region and address, with the original's name in the label:
  `game.missiles[4].time (missileTime)`.
- **Failures with no diffs.** When the decomp reads input or the NMI counter where the
  original didn't, it went another way earlier in the step and stops there. It now
  compares its state as it stands too, with the reason saying so; the field that sent it
  the other way is usually among the differences, and they can be traced.
- **The decomp's trace hook** records, after each step, a write for every byte of its 2 KB
  view that changed. Checked with a scratch run through dj's `Checker` on Defend (the
  first steps write `cur_keys`, `new_keys`, the crosshair, its sprite's x and the clock);
  dj doesn't read a comparison core's trace yet.
- **The API headers** are dj's own, included from the repository:
  `connectors/include/dj_connector.h` (version, coverage, build id, trace hook) and
  `dj_compare.h` (the comparison-core API). `src/dj_api.h` only includes the two.
- The good build still passes all 15 segments, 27,193 frames, 0 failed. The broken build's
  results were cleared and its entry removed from `.local/local.toml` again.

### Milestone 3 results

`dj parity examples/thwaite --compare decomp --all`, 2026-11-16, all 15 segments (27,193
frames in 38 s): **0 failed, and every frame passed** but boot's 8 (before the first wait
exit). The cutscenes are judged every frame: the opening one in 1 Player, 2 Players, Mouse
and "Click through the opening cutscene", and the end of Sunday in the Sunday segments.
`dj parity --report`: 4,263 instructions with parity, 0 failed, 81 not checked, all in
boot (`reset`, `pently_init`, `getTVSystem`); 122 of the 125 functions that ran are in
parity in all they ran.

- **New segment:** "Click through Sunday night, into Monday" (846 frames, bot-recorded),
  under the Sunday segment, which now also lets a house fall at 5 AM. It runs the end of
  the day's cutscene (`discover_clue_1`, chosen because a building fell), the house
  rebuild with the villager's name in the tip bar (`buildHouseRebuiltBar`), and Monday's
  first level.
- **The data:** `tools/gen_assets.py` writes `data/cutscripts.c`: the scripts still
  compressed, as the ROM holds them, at their CPU addresses (checked against the ROM),
  because the original keeps `script_ptr` into them in RAM and decodes a line at a time.
  The scenery, names and roles are small tables written from `cutscene.s`.
- **Caught:** a scratch build that moves the text cursor two columns a letter, not one,
  fails all 5 segments that reach a cutscene at its first letter:
  `cutscene_vram_dst_lo at RAM 0x057 (game.cut_dst_lo) is 0xA4, the original's is 0xA3`.
  (Results cleared, entry removed.)
- **Pictures, pixel for pixel** (`thwaite_pictures`, compared frame by frame): the title,
  both cutscenes (fade, scroll, trees, text, the "A" marker), gameplay, pause and the
  practice menu. Getting there fixed three things outside the cutscene's logic: the
  picture was a frame late wherever VRAM updates wait for the next vblank (now drawn at
  the step's end); the console's sprite priority (above, "Graphics"); and the practice
  menu's redraw after a copy (its meter, and no tip bar).
- **Pictures after a copy** (a rewind, or Sync now): some of what is on screen is only in
  VRAM, so the redraw works it out from RAM. The cutscene's dialogue box: the script is
  played from its start on a copy of the state, pressing A at each paragraph, until it is
  where the state is. The level's reward bar: its numbers worked out again from the houses
  and missiles left. The rebuilt house's bar: the villager of the building the cutscene's
  camera went to (`houseToRebuild` is cleared by then). A practice game's rows 8-9:
  `setupGameBG`'s stars until a tip comes. Checked with `thwaite_pictures`, copying every
  37 frames (101 in the long day) in every segment: all pictures match but two, in the
  poked segment, where the original's status bar still shows the day before the poke.
  Still not matched: the two frames of the step that loads a cutscene, where the original
  turns rendering off partway (the decomp draws at a step's end).
- **Still not reached by a segment:** the cutscenes for houses lost (8, 9), the later
  clues (11-13), the end (14), `findRandomDestroyedHouse` (a house to rebuild is always
  already set when one falls), and the laid-back villager's re-pick in
  `cut_choose_villagers` (when the one drawn is already the paranoid one).

### Milestone 2 results

`dj parity examples/thwaite --compare decomp --all`, 2026-11-16, all 12 segments (11,131
frames in 16 s): **0 failed**, and every frame of every gameplay and practice segment
passed: Defend 2,399, Idle 3,599 (play, game over, the legal screen after it), Jump to
Saturday 1,799 (the write queued after its step), Pause 299, Practice open 199, cancel 119,
play 899, and the game part of "Click through the opening cutscene" (34; the cutscene's 65
frames are skipped, not written yet). `dj parity --report`: 3,113 instructions with parity,
0 failed, 608 not checked; 92 of the 114 functions that ran are in parity in all they ran.
What isn't checked is the cutscenes (milestone 3), boot, and `pently_init`.

- **Caught in gameplay too.** The bug planted on purpose fails Defend at frame 2 and
  Practice play at frame 7 on `new_keys`; a second planted bug (enemy smoke every 21
  frames, not 20) fails Idle at frame 83 on `game.missiles[4].time (missileTime)`.
- **The pictures match:** the playfield with missiles, smoke, crosshair and status bar, the
  practice menu with its arrows and salvo icons, and pause with its tip
  (`thwaite_pictures`).
- **Then checked with two new segments** (2026-11-16), recorded by
  `player/examples/thwaite_bot.rs`, a scratch bot that plays the original, steers the
  crosshair at the lowest missile it sees in RAM and fires from the nearer silo, and writes
  what it pressed into an empty segment (`dj new`, then `dj clean` for the hashes):
  - "Practice: Monday 2 AM, balloons and a MIRV, cleared" (2,191 frames): the level
    cleared with every house standing, so `doStateLevelReward`'s scoring, `addScore` and
    `buildLevelRewardBar`.
  - "Defend through Sunday; lose the left silo at 5 AM" (13,037 frames): all five hours,
    with balloons from 2 AM and MIRVs from 3 AM; the bot lets the missiles aimed at the
    left silo through late in the last hour, so the silo is rebuilt (`doStateRebuildSilo`),
    and the day ends in the cutscene.

  Both passed the first time: 2,188 and 13,003 frames, 0 failed (the end of Sunday's
  cutscene is "not checked"). `dj parity --report` now: 3,634 instructions with parity, 0
  failed, 651 not checked; 99 of 124 functions in parity in all they ran. `makeBalloon`,
  `splitBalloon` (so the sound-leak breakpoint, with the missile loop going back),
  `handleMIRV`, `drawBalloonNumberX`, `addScore`, `buildLevelRewardBar`,
  `doStateRebuildSilo` and the hour changing are all checked. Still unchecked: the cutscenes
  and what they lead to (`doStateCutscene` past 5 AM, `findRandomDestroyedHouse`, the house
  rebuild), boot and `pently_init`.
- **Quirks of the original the decomp reproduces,** found while porting: the overlapping
  hotspot tables (`missileTileHotspotY` is one byte), the sine table running into the
  cosine table, a villager stepping toward its old target in the tick it picks a new one,
  explosions comparing against their cached position after their slot is reused, the
  shift's carry in the "!" sprite's height, the ones' complement (not two's) of a player
  missile's leftward speed, and the split sound's channel leaking into the missile loop.

### Milestone 1 results

`dj parity examples/thwaite --compare decomp --all`, 2026-11-16, all 12 segments (11,137
frames in 16 s): **0 failed.** 985 frames passed: Boot 111, 1 Player 5, 2 Players 15,
Mouse 70, Practice open 25, Practice cancel 70, Idle 689 (its return to the legal screen
after the game over). The frames before the decomp copies the original are skipped, and
everything after the title is "not checked", with the reason. `dj parity --report`: 572
instructions of the original with parity, in 27 functions in full.

- **The comparison catches bugs.** A build with a bug planted on purpose (held keys count
  as new presses) fails 4 segments at the first frame it matters, with diffs in both
  names: `new_keys (game.new_keys): 04 here, 00 in the original`, then `numPlayers` and the
  sprite it moves.
- **The pictures match.** `player/examples/thwaite_pictures.rs` plays a segment through
  dj's `Checker`, as dj-gui does, with the decomp drawing with OpenGL inside the baseline's
  frame. The legal screen and the title (with the mouse's cursor and icon) look the same
  as the original's.
- **What the decomp models so far:** 23 of the original's RAM symbols as its own fields;
  the rest is carried as copied and still compared, so any change the original makes that
  the decomp doesn't shows as a mismatch. Sound, the VRAM transfer buffer and the text
  buffer are ignored, and `nmis` is taken from the original.
- **A slip, put right:** `dj play --core` saves the core it is given as the project's own,
  so a milestone 0 check had set the decomp as Thwaite's baseline in `.local/local.toml`.
  It is back to Mesen, and the README warns against it.

### Milestone 0 results

From `player/examples/thwaite_breaks.rs`, run 2026-11-15 on all 12 segments (11,140 frames)
with the version 3 Mesen connector (`build/cores/mesen_libretro`):

```bash
cd player && cargo run --release --example thwaite_breaks -- ../examples/thwaite ../examples/thwaite/build/thwaite.nes ../build/cores/mesen_libretro ../examples/thwaite/.decomp/src/data/breakpoints.tsv
```

| Question | Result |
| --- | --- |
| Is each wait exit hit at most once a frame? | Yes, every wait, every frame. No frame has two |
| Does gameplay lag? | No. `wait:main` is hit in every frame of Defend (2,400), Jump to Saturday (1,800) and Practice play, and in every frame of play in Idle and Pause |
| Frames with no wait exit, after the first | 8, one at a screen change: title to cutscene (3 segments), cutscene to game, title to the practice menu, and three into a text screen (boot's legal screen, after game over, after leaving practice). Each frame ends inside the new screen's setup, with rendering off: `cut_draw_buildings`, `setupGameBG`, or the text decoder `undte_line0` |
| Frame ends in a spin | Every frame after boot except those 8. So the decomp can copy at almost any jump, with no pre-roll |
| Counter reads that saw the NMI | 4 steps: the cutscene-to-game step reads it three times (explosions, missiles, smoke) after `setupGameBG` crosses the frame boundary, and the title-to-cutscene step once (`load_cutscene`, in 3 segments). Pause's resume step also reads a value other than its wait exit's, from pause's own `sta nmis`, not an NMI |
| Input read in a later frame than its step's wait exit | Once: the cutscene-to-game step (wait exit in frame 62, `read_pads` in 63). Hence the input breakpoints |
| Boot | 5 frames to the first wait exit (`screen_off_title_palette`) |
| Loops | `updateSmoke`'s second read is in a loop: up to 24 hits a frame, far under dj's cap of 1,024 |
| Never hit | `ppu_wait_vblank` (nothing calls it) and `drawBalloonNumberX` (no balloon in the segments) |

This corrects the plan's earlier "7 lag frames": in these segments Thwaite never lags in
play. Its frames without a wait are all screen setup, which the breakpoint plan's
measurement also counted. The decomp handles both the same way.

## For the breakpoint plan

What this decomp needs from [yield-parity/PLAN.md](yield-parity/PLAN.md), or tests in it:

- **Nested OpenGL contexts** (its section 11, open problem 3). This decomp draws only with
  OpenGL, from inside the baseline's frame.
- **`registers()` at a jump.** Copying at the jump relies on the PC between frames. The
  6502 layout is now fixed in connector API 3 (A, X, Y, SP, P, PC low byte first), and
  milestone 0 read it at every frame end; the handle still needs to pass it on.
- **Queued writes.** A between-frames write applied after the next step is something any
  core parked at a wait exit can do. It may belong in the plan's notes for a core's author
  or in the helper header.
- **Its build order, steps 7 to 9,** test with the Tetris decomp. Thwaite is a second test
  there, and the first whose steps across a frame boundary are judged through counter and
  input breakpoints rather than answered "unjudged".
- **"Lag frames" in its measurements.** For Thwaite, the 7 frames without a wait that it
  counted are screen setup, not lag in play (milestone 0 results). The same may be true of
  some of Tetris's 15.
- **A diff with both `symbol` and an address.** dj keeps the address only when there is no
  symbol, so a core that names the original's symbol beside the address loses `--explain`.
  Keeping the address whenever there is one, and showing the symbol beside it, would make
  the obvious way to fill `dj_parity_diffs` traceable.
- **Input in a step that crosses a frame.** Milestone 0 found the case its notes for a
  core's author describe: the step's controller read lands in the next frame. A breakpoint
  at the input routine's entry, polling libretro there, was enough; the raw bytes didn't
  need copying through the handle.

## Risks

- **The counter breakpoints miss a read.** If a step that crosses a frame reads `nmis`
  somewhere not listed, it mismatches; the diff names the field. The converter finds every
  `lda nmis` in the source; another instruction reading `nmis` would be missed (none today).
- **A between-frames write while the original isn't in a spin.** Answered "unjudged" and
  copied again. Rare: 8 frame ends in the segments, all in screen setup.
- **The `missilesOverlay` reuse:** feared, not seen: the cutscenes don't write missile
  RAM in any segment.
- **Sprite order:** if Thwaite's OAM rotation is part of its state (it decides flicker),
  it has to match exactly, and the `sprites` policy may become `exact`.
- **Boot isn't judged:** its first 5 frames, up to the first wait exit. Modelling the PAL
  detection's timing loops would need cycle counts.
- **Committed tables drift** if the pinned Thwaite commit changes. The converters are
  rerun by hand then, and the README says so.
