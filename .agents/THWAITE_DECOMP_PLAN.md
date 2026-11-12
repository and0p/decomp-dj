# Plan: a C reimplementation of Thwaite as a comparison core

Status: draft, 2026-11-11. Not started. Decided 2026-11-11: the C is written from the
GPL source (a reimplementation, not clean room); generated data is committed as C tables;
no sound. This plan changes nothing in dj's connector API itself. It builds on the changes
[CHECKPOINT_PARITY_PLAN.md](CHECKPOINT_PARITY_PLAN.md) makes (the version 2 `dj_baseline`
handle, `dj_get_cpu_state`, check points), and works without them until they land.

## Goal

`examples/thwaite/` gets a second core: Thwaite rewritten in C, drawn with OpenGL, built as
a libretro core that exports every `dj_*` function. `dj parity` and the GUI then check it
against the original ROM (Mesen) on the project's 12 segments, and it becomes the
end-to-end example of the "decomp" half of decomp-dj: sync, `[WRITE]`, parity, diffs in the
original's terms, `parity_explain`.

What it is:

- **Its own C code**, organised the way C wants (structs, enums, explicit state machines),
  not a line-by-line translation of the 6502. Each C function names the original routine
  it stands in for (`/* orig: moveCrosshairPlayerX */`) so failures point at both.
- **Its own memory layout.** One `struct game` holds all state. A generated table maps its
  fields to the original's RAM symbols for sync, `dj_write`, checking and diffs.
- **The original graphics for now**: CHR, palettes, nametables, texts and level tables
  converted once from the GPL sources (`tilesets/*.png`, `texts.txt`, the `.s` tables) and
  committed as C tables. The converters stay in `tools/` to regenerate them; the core
  builds with only a C compiler and CMake, and never reads the ROM.
- **Written from the source.** The C is written reading Thwaite's GPL source, in its own
  structure. It's a reimplementation, not a clean-room one.
- **GPL-3.0**, like Thwaite (and the repo, once relicensed).

Not goals: sound (the core outputs silence), cycle accuracy, the PAL/Dendy speed paths (it
runs as NTSC), replacing the art, a standalone (non-libretro) build. Changes to dj's
connector API belong to [CHECKPOINT_PARITY_PLAN.md](CHECKPOINT_PARITY_PLAN.md).

## Layout

```
examples/thwaite/decomp/
  README.md               what it is, how to build, how it maps to the original
  CMakeLists.txt          builds thwaite_decomp_libretro.{dylib,so,dll}
  build.sh                runs cmake (CI and the README use it)
  tools/                  run by hand when the pinned Thwaite commit changes
    gen_assets.py         tilesets/*.png -> CHR; palettes, nametables, level/sine tables, texts
    gen_rammap.py         thwaite.dbg -> RAM symbols (name, address, size) and the
                          return addresses of each JSR into a wait loop (for sync)
  src/
    data/                 their output, committed: chr.c, tables.c, texts.c, rammap_syms.c
    game.h                struct game: every piece of game state
    main.c                the frame loop and the screen state machine (orig: main.s)
    title.c practice.c cutscene.c levels.c missiles.c explosion.c smoke.c
    scurry.c bg.c math.c random.c pads.c mouse.c bcd.c
    gfx.c                 the "screen": BG tilemap, palette, sprite list (no PPU emulation)
    gl.c                  OpenGL renderer for gfx.c (SET_HW_RENDER)
    libretro.c            the libretro API
    dj.c                  every dj_* function
    rammap.c rammap.h     field <-> original-symbol table, import/export/compare
```

`examples/thwaite/config.toml` gets:

```toml
[[compare]]
name = "decomp"
path = "decomp/build/thwaite_decomp_libretro"
sync = "dj_sync"
```

## How it runs

### Frames, not coroutines

The original's screens are nested loops that each wait for NMI (`lda nmis / cmp nmis / beq`):
the title, the practice menu, cutscenes, pause, and the main game loop. In C each of these
becomes a state in an explicit state machine, and `retro_run` advances it by one wait: it
runs the code between two of the original's waits, then returns.

This costs more than writing straight-line C on a coroutine, but everything that matters
here becomes easy: a save state is a `memcpy` of `struct game`, and a sync is "pick the
state, fill the fields".

### Lining the two up: check points first, frame ends until then

The decomp is built for check points
([CHECKPOINT_PARITY_PLAN.md](CHECKPOINT_PARITY_PLAN.md)): the baseline stops where a step
of the original's logic is done, the decomp runs to its own next one, and the two are
compared there. Until that lands, it is checked at frame ends like any comparison core
today. Both modes use the same state machine; only who decides when a step runs differs.

**Thwaite's check points** go just past each wait loop's exit (the first instruction after
the `beq` that spins on `nmis`), one per screen: `game` (in `main`, before
`blitBGUpdate`), `pause`, `title`, `practice`, and one per cutscene wait. Just past the
exit, not at the wait itself, because of how check points hand over input: a step gets
the inputs of the frame its previous check point was in. Past the exit, that is the frame
whose NMI started the step, which is the frame `read_pads` runs in. At the wait's entry it
would be the frame before, one frame early. Where the check is, the step's logic is done
and its BG update hasn't been sent yet, the same state the frame-end check sees.

**With check points** (`check_at = "checkpoints"`):

- `dj_run_to_checkpoint_named` advances the state machine one state and names the check
  point it reached. A lag frame has no check point, the next has two, and the decomp
  follows without knowing about lag.
- `dj_sync` is told which check point the baseline is stopped at (version 2 handle), so it
  never guesses the screen: the name picks the state, the RAM fills `struct game`.
- `retro_run` still runs whole frames for plain play (one step per frame, never lagging).

**At frame ends** (now, and for any baseline without check points):

At the end of Mesen's `retro_run` (start of vblank, before NMI) the original is, on a
normal frame, spinning in one of its wait loops with that frame's logic done. The decomp's
frame is the same span. Milestone 0 checks this on every segment, with Mesen's trace hook
now, or the frame-end PC once `dj_get_cpu_state` exists.

*Lag frames* (logic that runs past vblank) are the one place a frame-stepped rewrite can't
follow by itself, because they depend on 6502 cycle counts:

- **Signal: `frame_ticks` in the version 2 `dj_baseline`** (the ticks the real baseline ran
  this frame; Thwaite's `[tick]` is `doStateAction`). In the game loop, a frame with
  `frame_ticks == 0` while not paused is a lag frame. **Not** the baseline's
  `dj_get_tick_count` through `get_proc`: under dj-gui and `dj parity` the handle reaches
  a copy of the baseline that only has its save state loaded, never runs, and never has
  its tick location set, so it always reads 0.
- **Before version 2, or without `has_ticks`:** the game clock in RAM, which the copy does
  have. `incGameClock` runs once per game-loop iteration, so an unchanged clock
  (`gameSubTenth` and up) means the iteration didn't finish.
- **Response:** `dj_check_parity` returns **skip** ("baseline lagged"), restores the
  decomp's state from before this frame and keeps the frame's input, so the next frame
  runs the same step. When the original finishes the late iteration, the two are level
  again.
- Menus, cutscenes and pause have no ticks and no clock. They do almost nothing per frame
  and shouldn't lag; milestone 0 confirms that.
- Outside comparison, the decomp just doesn't lag. That's a documented difference.

*Which screen the original is in* comes from RAM alone in this mode, since the frame-end
handle names no check point. The 6502's stack is RAM (page 1), and the call chains into the
wait loops are fixed:

1. `gen_rammap.py` lists, for every wait loop, the chain of JSR return addresses that
   reaches it (`main -> pauseScreen`, `main -> titleScreen`, ...) and where on the stack
   page each sits. The decomp's states carry the same list.
2. `dj_sync` matches the baseline's stack page against those signatures, deepest first,
   then checks the RAM agrees (`gameState`, `isPractice`, the cutscene and pause
   variables). Stale return addresses below the live stack can fool the signature alone;
   the RAM check rules them out.
3. That picks the decomp's state; the RAM fields fill `struct game`.
4. When nothing matches, or more than one does, it returns 1 with the candidates in
   `reason`, and the player tries again at the next jump. Mid-logic (a lag frame) is one
   of these.

With `dj_get_cpu_state` (Part 1 of the check point plan), the frame-end PC replaces step 2's
guesswork: it names the wait loop directly, and the stack signature only picks the
cutscene step. The stack matching is kept anyway, for baselines without it.

`DJ_SYNC_BOOT` skips all of this: the decomp boots itself.

### Syncing, writing and checking

All three go through one generated table (`ram_map.c`) of the original's RAM symbols from
`thwaite.dbg`, each with a size and a **policy**:

| Policy | Used for |
| --- | --- |
| `exact` | persistent game state: scores, clock, houses, missiles, crosshairs, RNG, level tables in RAM |
| `when(cond)` | state that only means something in some screens (the `missilesOverlay` area, reused by cutscenes and the practice menu) |
| `ignore` | scratch: zero-page temporaries, the stack (read for sync, never compared), `xferBuf`, PAL detection state, all of Pently's sound state |
| `sprites` | the OAM buffer, compared as a set of sprites per frame, not byte order (milestone 2 decides how strict) |

- **`dj_sync`**: reads baseline RAM (`retro_get_memory_data`), fills every non-ignored field;
  `DJ_SYNC_BOOT` boots the decomp itself instead.
- **`dj_write`**: a `[WRITE]` to RAM is looked up in the table and applied to the field
  (`Jump to Saturday` writes `gameDay`); a write to an `ignore` byte returns 1 (ignored on
  purpose); outside RAM returns 1.
- **`dj_check_parity`**: exports the decomp's state to the original's layout, compares by
  policy, returns 0/1/2.
- **`dj_parity_diffs`**: one entry per differing field: `label` is the C name
  (`game.missiles[5].y`), `symbol` the original's (`missileYHi`), `address` the byte,
  expected/actual bytes.
- **`retro_get_memory_data(SYSTEM_RAM)`**: a 2 KB view of `struct game` in the original's
  layout, refreshed each frame, so dj's RAM views, `[hash]` and the RAM fallback work.

### Graphics (OpenGL)

`gfx.c` is a tiny "screen" the game code writes to: a 64x30 tile BG map (two nametables;
cutscenes scroll), 32 palette entries, a sprite list, and a scroll position. It is not a
PPU: no registers, no VRAM address latches. The game code calls things like
`gfx_put_text`, `gfx_set_tiles`, `gfx_sprite`.

`gl.c` draws it into the frontend's framebuffer (`SET_HW_RENDER`, OpenGL 3.2 core, GLSL 150,
which macOS, Linux EGL and Windows all give dj): CHR as an index texture, the palette as a
lookup texture with the NTSC palette Mesen uses by default, BG as one quad, sprites as
instanced quads with the 8-per-line limit applied. Output 256x240.

Flicker: Thwaite flickers explosions and smoke by cycling sprites. The default reproduces
it; a core option `thwaite_decomp_flicker = original|blend` draws them translucent instead,
as a first taste of what a decomp can change.

### Sound

None. The core sends silence at 48 kHz so frontends stay in time. The original's calls
into Pently (`pently_start_sound`, `pently_start_music`, ...) become empty stubs with the
original's name, so a sound can be added later where it plays.

### The rest of the dj API

Everything is exported, as asked:

| Function | In the decomp |
| --- | --- |
| `dj_api_version`, `dj_build_id`, `dj_forced_options` | 2; `thwaite-decomp@<git hash>`; no forced options |
| `dj_region_count`, `dj_get_region` | `RAM` (the 2 KB view, cpu NULL) and `SRC`: one flag byte per instrumented block of the C code (a `COV()` macro at the start of each original-routine function and branch), cpu NULL |
| `dj_clear`, `dj_set_tick_exec`, `dj_get_tick_count` | over `SRC`; the tick block is `doStateAction`'s |
| `dj_trace_start/stop/read` | write events (kind 3) in the original's address space, from the export step, so `parity_explain` can trace both sides |
| `dj_sync`, `dj_write`, `dj_check_parity`, `dj_parity_diffs`, `dj_run` | as above; `dj_run` reports an unimplemented screen or routine by name instead of running on. All of them read the handle's version and use the version 2 fields only when present |
| `dj_run_to_checkpoint_named` | once check points exist: one state of the state machine, naming the check point reached |

dj ignores a comparison core's coverage today (connectors.md); `SRC` is there for when it
doesn't.

## Milestones

Each one ends with `dj parity examples/thwaite --compare decomp` on the segments it covers.
Segments can be checked from their own start (sync mid-game), so gameplay doesn't wait on
the cutscenes that come before it in the tree.

0. **Probe and skeleton.**
   - A throwaway probe (an `examples/` program in `player/`) that plays every segment with
     Mesen's trace hook on and records, per frame, where the CPU was when the frame ended,
     and the stack page. Answers: is it always in a wait loop, which ones, how many lag
     frames, and whether the stack signatures tell the screens apart.
   - The decomp core skeleton: libretro boilerplate, GL renderer showing the CHR, every
     `dj_*` stubbed, the converters and their committed tables, `build.sh`,
     `[[compare]]` in config.toml. It loads in dj-gui beside Mesen.
   - Also answers, for the check point plan: does a spin-loop exit ever fall on the other
     side of Mesen's frame boundary from the NMI that caused it (it decides which frame a
     step's input comes from).

Milestones 1 to 3 check at frame ends, which works today. When check points land (their
plan's step 10 is this decomp), the `[[checkpoint]]` entries go into config.toml,
`dj_run_to_checkpoint_named` is added, and all segments are checked again with
`check_at = "checkpoints"`, lag frames included. The frame-end mode stays working.
1. **Boot and menus**: reset, the legal screen, title, the 2-player and mouse paths,
   practice menu. Segments: Boot, 2 Players, Mouse, Practice open/cancel.
2. **Gameplay**: new level, crosshairs, player and enemy missiles, balloons, explosions,
   smoke, villagers, scoring, the clock, BG updates, pause, game over. Segments: Defend,
   Idle, Pause, Practice play, Jump to Saturday (`dj_write`).
3. **Cutscenes and between levels**: DTE text from `texts.txt`/`cutscripts.txt`, level
   reward, silo and house rebuilding. Segment: the opening cutscene. All 12 pass.
4. **Diffs and tracing**: `dj_parity_diffs` and `dj_trace_*` complete, checked with
   `parity_explain` on a deliberately broken build.
5. **CI and docs**: a CI job builds Mesen and the decomp and runs `dj parity` on the
   example; README for the decomp; examples/thwaite/README updated; ROADMAP.

New segments that reach more of the game (later days, a rebuilt house, a high score) are
worth adding as the decomp gets there; the coverage map shows where.

## Risks

- **The frame boundary isn't where it's assumed.** Milestone 0 exists to find out before
  any game code is written.
- **Lag frames** in heavy waves: at frame ends, skip-and-hold above; with check points,
  nothing to do.
- **The version 2 handle isn't there yet** when gameplay is written: the clock fallback
  covers lag in the game loop until it is.
- **The `missilesOverlay` reuse**: cutscenes and the practice menu reuse missile RAM for
  their own variables; the policy table has to know which layout is live.
- **Sprite order**: if Thwaite's OAM rotation is part of its state (it decides flicker),
  it needs to match exactly; the `sprites` policy may become `exact`.
- **Frame-end syncing from RAM is ambiguous somewhere.** Stack signatures plus RAM checks
  should tell the screens apart, but stale stack bytes or two screens with the same chain
  could defeat them. Such a sync fails with a reason instead of guessing. Check points
  (the name in the handle) and `dj_get_cpu_state` (the PC) both remove the guess.
- **Committed tables drift** if the pinned Thwaite commit changes; the converters are
  rerun by hand then, and the README says so.
