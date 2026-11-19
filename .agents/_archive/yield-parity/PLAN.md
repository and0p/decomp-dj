> **Archived. Agents: do not read this file unless the developer tells you to.** It is an earlier revision or a long design discussion, kept for the record, and it contradicts the current design in places. The current design is [PLAN.md](../PLAN.md), with charts in [FLOW.md](../FLOW.md).

# Plan: parity at yields

Written 2026-11-12 by the meta-analysis thread, from decisions Andrew made in that
thread. Revised 2026-11-13 11:20 EST after a review by another thread that read only this file and
FLOW.md; what came from it is marked "Review". Revised again 2026-11-13 14:43 EST: input reaches the
decomp through a yield at the game's input routine (section 2a). Revised 2026-11-13 15:01 EST after a
second review, marked "Review 2". Revised 2026-11-13 16:23 EST: the comparison core runs in
the GUI's process (section 9). Nothing here is built.

- **This file is the design as it stands.** Charts are in [FLOW.md](FLOW.md).
- **The history is in [../CHECKPOINT_PARITY_PLAN.md](CHECKPOINT_PARITY_PLAN.md):** the
  experiments, the options that were weighed, and the ideas that were dropped. Read it
  for why, not for what to build.
- **"Thread"** marks a recommendation Andrew has not confirmed.

## The goal

Compare a decomp against the original game at moments that mean the same thing in both.
Today dj compares once per frame, at the frame end. That works when the game is waiting
at every frame end. It does not work when a frame ends in the middle of the game's logic
step, and a decomp has no frames of its own.

## Terms

| Term | Meaning |
| --- | --- |
| Baseline | The original game, running in an emulator core with a dj connector |
| Comparison core | What the baseline is compared against: a decomp, or a second emulator |
| Frame | One `retro_run` of the baseline. Segments, inputs, snapshots and seeking are in frames |
| Tick | One step of the game's logic. In No Mercy a tick takes two frames |
| Yield | A named place where the baseline calls dj and waits. `[tick]` is one; the frame end is always one |
| Sync | Bringing the comparison core to the baseline's state |
| In step | The comparison core has synced and has been told about every yield since |

## What exists today

- **One check per frame.** After each frame dj calls the comparison core's
  `dj_check_parity`, or compares system RAM, and gets match, mismatch or skip
  (`player/src/compare.rs`).
- **Sync after a jump** is the core's `dj_sync`, or loading the baseline's save state when
  the core is the same emulator (`sync = "state"`).
- **`[tick] exec`** names one address. The connector counts how often it runs. Nothing is
  called there.
- **The connector API is version 2** (`DJ_API_VERSION` in `player/src/libretro.rs`,
  `docs/connectors.md`).
- **In the GUI the comparison core runs in a worker process.** In batch commands each
  chain runs in one worker with both cores (`ISOLATION_PLAN.md`).

## The design

### 1. Yields in the project config

```toml
[tick]
exec = ["PRG:0xAA36", "PRG:0xAA4C"]   # where a tick ends; one address or a list

[[yield]]
name = "match_tick"
exec = "RDRAM:0x114514"

[[yield]]
name = "vblank"
exec = "CORE:nmi"                      # a yield the connector provides by name
```

- **`[tick]` stays its own table.** With no `[tick]`, the tick is the frame end, as now.
- **`exec` is one address or a list.** When it is a list, the core is told which one
  fired.
- **`CORE:name` is a built-in yield** that the connector provides without an address.
  `CORE:frame_end` is always on and needs no entry. A name the connector does not offer
  is an error when the project loads.
- **Yields have no guard for overlays.** The comparison core can see which code is loaded
  and answer "skip".
- **Yields live in the project config,** not the local one. They are facts about the
  game.
- **Yield definitions do not feed state hashes.** Changing one must not dirty the tree.
- **One address belongs to one yield.** The same address under `[tick]` and a
  `[[yield]]`, or under two yields, is an error when the project loads. If the game's
  input routine is also its tick, it goes in `[tick]`. (Review 2)
- **Keep the list short.** A yield in a hot loop is a mistake; dj reports a yield that
  fires more than a cap in one frame as an error.

### 2. The call at a yield

When the baseline reaches a yield, the connector calls a function dj gave it and waits.
The baseline stays inside `retro_run`. dj passes every yield to the comparison core;
there is no subscribing.

What the core is handed:

- The yield's name, and which address fired when `exec` is a list.
- Whether parity checking is on.
- How many times this yield has fired, since the segment start and since the last sync.
- On the N64, which thread reached the yield.
- The numbers of the frames since the last call.
- A handle to read the baseline's memory and registers.
- Once, at load: the project's patch list, so the core can confirm it was built with the
  same change.

The core runs itself for as long as it decides, then returns:

| Status | Meaning |
| --- | --- |
| match | The core checked and saw parity |
| mismatch | The core checked and parity is lost. Reasons follow |
| skip | The core did not judge this yield: it disregards it, the overlay is not one it knows, or checking is off |
| error | The core cannot continue |

- **A flag: the core completed a tick at this yield.** dj counts these as the
  comparison's ticks.
- **Reasons:** a list. An address and a length where the core knows them, otherwise text.
  Each reason carries both sides' values. (Review: the baseline runs on to the frame end
  before playback stops, so its memory has changed by the time anyone looks. The
  comparison core is not run past the yield, so only the baseline's side would be lost.)
- **The core gives every verdict.** dj does not compare memory itself and keeps no list
  of bytes to ignore.
- **dj does its costly work only for match and mismatch:** taking and clearing the
  coverage flags. A skip costs only the call.
- **dj counts match, skip and mismatch per segment and shows the share of yields that
  were judged.** (Review 2: a core can answer "skip" for ever. The share judged is the
  number a user needs.) dj can only count what the core reports; a core can copy the
  baseline's state through the handle without saying so.

**The handle at a yield reads memory and registers, and nothing else.** (Review 2.) That
is all the design asks of it there, and the baseline must not be run, saved, loaded or
reset while it is stopped. The handle is a pointer into the baseline, in the same
process (section 9), so nothing is copied unless the core copies it. Today's handle gives
a core any function of the baseline by name; the Tetris decomp uses it only for
`retro_get_memory_data` and `retro_get_memory_size`.

**What this asks of a comparison core's author.** dj delivers yields; the core does the
rest. The list has grown with each review, and it is the real cost of the design:

- **The core runs one step behind.** It is told which yield the baseline reached, then
  runs itself to that point.
- **A value read through the handle is the baseline's value at that yield.** It is the
  value the game used only if a yield sits where the game uses it. (Review 2.) So each
  place where timing-dependent data enters the logic needs a yield. This adds yields and
  legwork, against "keep the list short". It cannot cover the idle-loop generator, which
  runs in a hot loop; that needs the patch.

**A limit on judging:** a port cannot work out where an interrupt landed inside the
game's logic, and often cannot act on it when told. On a lag frame the handler runs in
the middle of a tick. Thwaite had 7 such frames and Tetris 15.

- **Why:** the interrupt lands after a number of CPU cycles, on whichever instruction is
  running. A port is the same logic compiled for another CPU, with no such count, and
  the landing point may fall inside what is one statement in the port.
- **When it matters:** only where the handler changes something the interrupted logic
  reads afterwards. A port can then differ without being wrong, and the core answers
  "skip".
- **When it does not:** most frames, where the game is in its wait loop; a handler that
  shares no state with the logic; and a rebuilt ROM that matches, run in an emulator.
- It applies at the frame end too, since on a lag frame the frame end is mid-tick.

### 2a. Input

- **dj plays input by frame, to both cores, always,** as an emulator or a TAS tool
  would. The comparison core is a libretro core and reads the frame's input through
  libretro, as the baseline does. Adding yields does not change this. (Andrew,
  2026-11-13. Review 2 read the earlier wording as "a yield removes the core's input".)
- **How the game polls input is the comparison core's business, not dj's.** dj records
  no controller reads and no connector needs a hook for them.

A technique for a core, when a tick can span two frames and it must know which frame's
input the tick used:

- **The project lists the game's input routine as a `[[yield]]`,** an ordinary one with a
  name such as `pads_read`.
- **Place it just after the raw bytes are final,** not at the end of the routine. Most
  input routines go on to work out pressed and released buttons or to filter the stick.
  If the core copied that result, its own version of that code would never be tested.
  (Review 2.)
- **The core copies the game's controller buffer from the baseline through the handle.**

How the two example games take input:

- **Tetris** reads the controller in its NMI handler, twice per frame. Its decomp reads
  the frame's input through libretro in each `retro_run`. That is enough: the game reads
  input once per frame.
- **No Mercy** calls `pads_read` once per pass of its game loop, then runs one
  `match_tick`. From the AKI decomp's notes, not measured here: the loop does not count
  elapsed video interrupts, and `osGetTime` is used only at start-up. So the pad and the
  idle-loop generator are the only timing-dependent inputs known.

When each emulator asks dj for input (read from source, not run):

| Emulator | When |
| --- | --- |
| Mesen (NES) | Once per frame, at a set scanline. The game's reads are served from that |
| mGBA (GBA) | Once per frame, at the start of `retro_run` |
| Mupen64Plus-Next (N64) | When the game sends a controller command |

- **bsnes (SNES) is not in the table:** the source read was the newer
  `libretro/bsnes-libretro`, not the `bsnes_accuracy` core. There, input is asked for
  when the controller is latched.
- **Mupen64Plus does not randomise controller timing.** `randomize_interrupt` is
  hard-coded to 0 in this port's `main.c`. The connector build pins the upstream commit
  and the build id changes with the source, so this cannot change unnoticed.

### 3. Sync

- **dj offers a sync at every yield** while the core is out of step, the frame end
  included. The core accepts where it can, declines where it cannot, or fails.
- **The core brings itself fully into step from the baseline.** dj does not manage a
  decomp's or a port's memory, and there are no checkpoints.
- **A project with only the frame end syncs at a frame boundary.** That works today for
  Tetris. A decomp of a game whose frame ends fall inside a tick declines the frame end
  and syncs at its tick.
- **Nothing before the sync point is checked.** Checking starts at the next yield.
- **A parent segment playing into its child is not a jump.** The core stays in step.

- **A `[WRITE]` line puts a decomp out of step.** (Review 2; thread's choice of fix.)
  The baseline gets the write at a frame start, which can be in the middle of a tick. A
  decomp parked at its last yield cannot apply it at the same moment. It syncs again at
  the next yield, as a core without `dj_write` does today. A comparison core that is the
  same emulator still gets the write.

### 4. On a mismatch or an error

- **Playback stops, at the frame boundary.** The baseline cannot be left stopped in the
  middle of a frame, so the frame finishes first.
- **There is no setting for this.** When the aim is parity there is no reason to keep
  playing. A user, or an agent through the MCP, can step on by hand to look at the drift.

### 5. Frames stay underneath

- **Segments, inputs, snapshots and seeking stay in frames.** A tick-based baseline was
  considered and rejected.
- **Ticks may be shown on the timeline,** from a frame-to-tick map that dj records as
  segments play. The map's cache key must include the `[tick]` definition. Can wait.

### 6. The same emulator as the comparison core

Two emulators cannot each wait inside the other's call. Both get the same yields; each
records the yield and a hash of RAM as it runs a frame, and dj compares the two lists.
With only the frame end this is today's check. (Thread; Andrew has not ruled on it.)

This only works when both emulators run the same ROM, so that the same addresses hold
the same code and RAM can be hashed whole (Review). A rebuilt ROM that does not match
byte for byte is not covered: see open problem 11.

### 7. Patches

For games whose state depends on emulator timing. In No Mercy the idle thread advances
the random number generator, so its state follows spare CPU time. Removing that one call
makes a match replay exactly, which is what the AKI decomp does.

- **A patch belongs to the project.** `[[patch]]` in config.toml points to a patch file.
  The file holds the offsets and bytes; the config does not.
- **dj applies it to the ROM in memory, before boot,** and hands the core the buffer. The
  ROM on disk is not changed.
- **Every emulator core gets it:** the baseline, and a comparison core that is an
  emulator. A decomp carries the change in its own build.
- **The patch's hash is part of the setup.** A patch with a different hash means every
  segment is replayed. There is no extra confirmation: a segment plays, and the user
  saves it or does not.
- **Each core must take the ROM from the buffer.** Mupen64Plus-Next does. mGBA does when
  given one. Mesen reads the path, so its connector patch must make `retro_load_game`
  use the buffer, and make a power cycle (`Console::ReloadRom`) reuse that buffer.
- **The file format is not decided.** Thread: BPS, because it checks that it is being
  applied to the right ROM, which also catches a byte-swapped N64 dump.

### 8. The connector API

Changed in place. dj is not released, so no older API is kept. Thread: raise the version
number anyway, so a core built before the change gets today's "rebuild the core" warning.

### 9. Where the comparison core runs

Decided by Andrew 2026-11-13: **in the GUI's process, not in a worker.** Batch commands
already run both cores in one process, so the GUI and batch commands run a comparison
core the same way.

- **Why:** the handle is a pointer, a yield in the middle of a frame needs no copy of the
  baseline, and there is no protocol between processes to extend. It is also less work:
  the in-process code (`Comparison` in `player/src/compare.rs`) is what batch commands
  use.
- **What is given up:** a crash in a decomp closes the GUI, where today it shows "the
  comparison core crashed at frame N" and the baseline plays on. A hang can be reported
  by the GUI's watchdog but not ended.
- **What covers it:** unsaved edits are saved to `.local/` as they are made, and a crash
  handler writes a note shown on the next launch (`ISOLATION_PLAN.md`, step 4).
- **To build with it:** the crash note names the comparison core and what it was doing
  (Andrew: worth implementing).
- **OpenGL:** each core that asks for it already gets its own private context
  (`player/src/gl.rs`), so a comparison core gets one separate from the baseline's and
  the GUI's. The Tetris decomp draws in software.
- **This reverses step 2 of `ISOLATION_PLAN.md`** (agreed 2026-11-10), which has not been
  edited. Thread: leave `RemoteComparison` in the code, unused, so going back is cheap.

## What is not in the design

Each of these was considered and dropped. The history file has the reasoning.

| Dropped | In its place |
| --- | --- |
| Stopping the baseline and returning from `retro_run` at a yield | The blocking call |
| dj telling the core to "step one tick" | The core runs itself and says when it completed a tick |
| A guard on N64 yields for which overlay is loaded | The core answers "skip" |
| The core subscribing to yields | Every yield is passed |
| `tick = true` on a yield | `[tick]` as its own table |
| A tick's optional start address | `[tick] exec` lists where a tick ends |
| A tick-based baseline | Frames, with ticks as a display choice |
| A halt setting, or the core asking for a halt | Always stop on a mismatch or an error |
| dj comparing memory, and ignore rules in the config | The core gives the verdict |
| Checkpoints of the comparison core, and per-yield records in the cache | The core syncs itself from the baseline |
| dj copying timing-dependent values from the baseline at each yield | A patch for state that only looks like an input, such as the idle-loop generator. For true inputs, such as a retrace counter or a clock, the core reads them through the handle, at a yield placed where the game uses them |
| A new time limit on the call | None in the GUI. Batch commands keep their rule: no frame within 30 seconds ends the chain |
| The comparison core in a worker process in the GUI, and the handle reading through a pipe or shared memory | The comparison core in the GUI's process |
| A special "sync at the frame start when the game is waiting" path | The frame end is a yield like any other |
| dj recording each controller read and handing its bytes to the core, with a hook in each connector | A yield at the game's input routine; the core copies the buffer |
| Patch bytes written in the config, and patching RAM after boot | A patch file applied to the ROM before boot |

## What was measured

All on dj's own connectors. Details and method are in the history file.

| Game | Result |
| --- | --- |
| Thwaite (NES, Mesen) | The game waits once per frame in 11,013 of 11,020 frames after boot; 7 lag frames |
| Tetris (NES, Mesen) | 30 segments, 24,392 frames, 15 lag frames |
| No Mercy singles match (N64) | 467 ticks in 934 frames. 1 tick started in one frame and finished in the next |
| No Mercy tag match, four CPUs (N64) | 303 ticks in 607 frames. 162 ticks (53%) started in one frame and finished in the next |

- **Mesen save states replay byte for byte.** Tick positions were the same from power-on
  and from snapshots, except in one Thwaite segment hit by the port-device bug (open
  problem 10).
- **Where a tick falls inside a frame comes from the emulator's timing,** not only from
  the game.
- **N64 overlays reuse addresses.** `match_tick`'s address ran in 84 menu frames as other
  code.
- **Loads have no ticks.** No Mercy has stretches of up to 209 frames with no game-loop
  pass.
- **No Mercy's random number state changes every frame,** from the idle loop.

## Open problems

1. **A decomp that corrupts memory before it crashes.** With the comparison core in the
   GUI's process (section 9), a buggy core can damage dj's own memory. Saved edits cover
   a crash; they do not cover a backup written from damaged memory. Thought unlikely; no
   answer.
2. **A comparison core that draws with OpenGL, in the GUI's process.** Not tried: until
   now its context lived in the worker. The first case would be Mupen64Plus against
   Mupen64Plus. To test: switching between three contexts on macOS, and on Linux the
   existing workaround for a thread that cannot hold a GLX and an EGL context at once.
3. **The names** of the call and of the struct it returns.
4. **The N64 connector has no yields.** Its tick functions are stubs. It also needs to
   report the thread, and the game runs on a coroutine whose stack may be too small for
   the call. Review: the connector can switch to the main coroutine to make the call,
   without returning from `retro_run`, so the game's stack stops mattering. Not checked:
   whether anything treats that switch as the end of the frame.
5. **Reading registers.** The connectors have no function for it, and the handle at a
   yield should offer one.
6. **A load must not fire yields.** On Mupen64Plus a load ran 1,951 distinct instructions
   inside `retro_unserialize`.
7. **Where a yield goes in a spin loop.** It must be an instruction that runs once per
   wait. In Thwaite's boot, one loop ran its compare 2,693 times in a frame.
8. **dj's own tick count with no core loaded** counts every hit at the `[tick]` address,
   including other code at that address on the N64. A project should pick an address in
   code that is always loaded. The review suggests the yield also names the instruction
   expected at the address. Thread: one word is a weak test, because functions often
   start with the same instruction; add an optional check only if a project has no
   always-loaded address for its tick. Andrew dropped the guard and has not reopened it.
9. **For the patch:** the file format; whether a game's overlays are stored compressed,
   which a byte patch cannot reach; the ROM offset of No Mercy's idle-loop call. The N64
   boot checksum is set aside.
10. **Two Mesen bugs on the roadmap** change counts, not RAM: the lag counter after a
    load, and a port keeping a device from an earlier segment.
11. **A rebuilt ROM that does not match, run in an emulator** (Review). It cannot use
    section 6: its addresses differ and its RAM cannot be hashed whole. It also cannot
    "run itself" to its own yield, because an emulator core only runs whole frames. No
    answer yet. How common the case is for dj's users is not known.

## Build order

Thread's proposal. Each step says what "done" looks like.

1. **Move the comparison core into the GUI's process,** and make the crash note name it.
   Done when the GUI's existing comparison features work as before without a worker, and
   a core that crashes on purpose leaves a note naming it and what it was doing.
2. **A spike in the Mesen connector.** One yield at Thwaite's game wait, the blocking
   call, and the frame end as a built-in yield. Done when the yields and RAM hashes are
   the same across two runs and after loading a snapshot, and the cost per yield is
   measured.
3. **Two stub comparison cores.** One copies the baseline at every yield and always
   answers match. One diverges on purpose. Done when match, mismatch with both sides'
   values, skip, error, a declined sync and the stop at the frame boundary have each been
   seen.
4. **The patch feature,** with the Mesen buffer change. Independent of the rest. Done
   when No Mercy's CPU match replays to the same hash twice with the generator pinned.
5. **N64 yields** in the Mupen64Plus connector. Done when `[tick]` at `pads_read` counts
   the same from power-on and from a snapshot, and a stub core has copied the pad buffer
   at that yield.
6. **The frame-to-tick map and ticks on the timeline.** Can wait.

A real decomp is the last test, and the only one that proves the design. The Tetris
decomp, which syncs at the frame end today, is the nearest.
