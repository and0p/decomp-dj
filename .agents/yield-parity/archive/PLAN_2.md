> **Archived. Agents: do not read this file unless the developer tells you to.** It is an earlier revision or a long design discussion, kept for the record, and it contradicts the current design in places. The current design is [PLAN.md](../PLAN.md), with charts in [FLOW.md](../FLOW.md).

# Plan: parity at yields (revision 2)

Written 2026-11-13 16:56 EST from [PLAN.md](PLAN.md) and a third review. That review read
the docs, the player and GUI code and the connector patches, but not the history file.
PLAN.md is left as it was. Nothing here is built.

- **This file is the design as it stands after Review 3.** Charts are in
  [FLOW_2.md](FLOW_2.md).
- **The history is in [../CHECKPOINT_PARITY_PLAN.md](CHECKPOINT_PARITY_PLAN.md).** Read
  it for why, not for what to build.
- **"Review 3"** marks what this revision changed. The full list is at the end, under
  "What changed from PLAN.md".
- **"Thread"** marks a recommendation the developer has not confirmed. Every Review 3 change is
  also unconfirmed until the developer rules on it.
- "Review" and "Review 2" mark changes from the earlier reviews, as in PLAN.md.

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
| Yield | A named place where dj is told the baseline has arrived, and the baseline waits. `[tick]` is one; the frame end is always one |
| Sync | Bringing the comparison core to the baseline's state |
| In step | The comparison core has synced and has been told about every yield since |

## What exists today

- **One check per frame.** After each frame dj calls the comparison core's
  `dj_check_parity`, or compares system RAM, and gets match, mismatch or skip
  (`player/src/compare.rs`).
- **Sync after a jump** is the core's `dj_sync`, or loading the baseline's save state when
  the core is the same emulator (`sync = "state"`).
- **`[tick] exec`** names one address. The connector counts how often it runs. Nothing is
  called there. `dj_clear` zeroes the count, so a segment's stored ticks are too low
  whenever the GUI or a parity check cleared the flags.
- **With no `[tick]`, no ticks are counted.** (Review 3: PLAN.md said the tick is then the
  frame end.)
- **The connector API is version 2** (`DJ_API_VERSION` in `player/src/libretro.rs`,
  `docs/connectors.md`).
- **The comparison core runs in a worker process everywhere.** (Review 3: PLAN.md said
  batch commands run both cores in one process. They do not.) The GUI and `dj parity`
  both load `RemoteComparison`, and `parity::Checker` and `check_span` take that type.
  `dj parity` also runs as a supervised child. The in-process `Comparison` is used only
  by the tests and by the worker itself (`ISOLATION_PLAN.md`, step 2, "Not done").
- **Coverage flags have three readers.** The recorder gathers them after every frame
  (`Session::advance`). The GUI reads and clears them on every screen refresh. A parity
  check takes and clears them after every frame.

## The design

### 1. Yields in the project config

```toml
[tick]
exec = ["PRG:0xAA36", "PRG:0xAA4C"]   # where a tick ends; one address or a list

[[yield]]
name = "match_tick"
exec = "RDRAM:0x114514"
```

- **`[tick]` stays its own table.** With no `[tick]`, dj counts no ticks, as now.
- **`exec` is one address or a list.** When it is a list, the core is told which one
  fired.
- **The frame end is always a yield, named `frame_end`, and it is dj's own.** dj raises it
  after `retro_run` returns. The connector is not involved, so a baseline without a
  connector still has it. (Review 3.)
- **There are no yields that a connector provides by name.** (Review 3: PLAN.md had
  `CORE:name`, with `CORE:nmi` as its example.) The frame end does not need one, and an
  interrupt handler starts at a fixed address, which an ordinary `exec` covers. This
  removes the connector's list of names and the load-time check against it.
- **Yields have no guard for overlays.** The comparison core can see which code is loaded
  and answer "ignore" or "skip".
- **Yields live in the project config,** not the local one. They are facts about the
  game.
- **Yield definitions do not feed state hashes.** Changing one must not dirty the tree.
- **Yield definitions do feed parity results and tick counts.** (Review 3.) The key of a
  stored parity run (`parity_runs`), a segment's stored tick count and the frame-to-tick
  map must each include the `[tick]` and `[[yield]]` definitions. Otherwise `dj parity`
  answers "checked with this build before" after a yield has moved.
- **One address belongs to one yield.** The same address under `[tick]` and a
  `[[yield]]`, or under two yields, is an error when the project loads. If the game's
  input routine is also its tick, it goes in `[tick]`. (Review 2)
- **Keep the list short.** A yield in a hot loop is a mistake.
- **A yield that fires more than a cap in one frame is switched off until the frame
  ends,** with one warning that names it. (Review 3: PLAN.md made this an error, and an
  error stops playback.) On the N64 other code can sit at a yield's address in another
  overlay and run in a loop. dj counts hits before the comparison core is asked, so the
  core's "ignore" could not prevent the stop, and it would happen with no comparison core
  loaded.

### 2. The call at a yield

When the baseline reaches a yield, the connector calls a function dj gave it and waits.
The baseline stays inside `retro_run`. dj passes every yield to the comparison core;
there is no subscribing.

What the core is handed:

- The yield's name, and which address fired when `exec` is a list.
- Whether parity checking is on.
- How many times this yield has fired since the last sync.
- The segment and the number of the frame the baseline is in.
- A handle to read the baseline's memory and registers.
- Once, at load: the hash of each of the project's patches, so the core can confirm it
  was built with the same change.

Taken out by Review 3:

- **The count since the segment start.** After a seek dj cannot know it, because per-yield
  records in the cache were dropped.
- **The list of frames since the last call.** Every frame end is passed to the core, so
  while the core is in step the list never holds more than the current frame.
- **Which N64 thread reached the yield.** A connector can only find the running thread at
  an address that differs per game. The core knows that address and can read it through
  the handle.

The core runs itself for as long as it decides, then returns:

| Status | Meaning | What dj does with the coverage flags |
| --- | --- | --- |
| match | The core checked and saw parity | Takes them, adds them to the build's passed set, clears them |
| mismatch | The core checked and parity is lost. Reasons follow | Takes them for the failure record, clears them |
| skip | The core cannot vouch for what ran since its last verdict: an overlay it does not know, a load, checking is off | Clears them. Nothing is credited |
| ignore | This yield is not one the core acts on. It did nothing | Leaves them. They count towards the next verdict |
| error | The core cannot continue | Clears them |

- **"Skip" and "ignore" are separate.** (Review 3: PLAN.md had one "skip" for both.) With
  one status, flags were kept across every skip. A stretch of 209 loading frames answered
  "skip" was then credited as parity by the first match after it. A core that judges only
  at its tick answers "ignore" at the frame end and at `pads_read`, and "skip" where it
  cannot judge.
- **Reasons:** a list. An address and a length where the core knows them, otherwise text.
  Each reason carries both sides' values. (Review: the baseline runs on to the frame end
  before playback stops, so its memory has changed by the time anyone looks.)
- **The core gives every verdict.** dj does not compare memory itself and keeps no list
  of bytes to ignore. The one exception is section 6.
- **dj counts match, mismatch, skip and ignore per segment.** The share judged is match
  and mismatch over match, mismatch and skip. Ignored yields are shown apart and are not
  in that share. (Review 2 asked for the share. Review 3: without "ignore", a core that
  disregards the frame end could never show more than half.) dj can only count what the
  core reports; a core can copy the baseline's state through the handle without saying
  so.
- **No "completed a tick" flag.** (Review 3; Thread.) PLAN.md had the core report it and
  dj count it, but gave dj nothing to do with the count. If dj should compare the two tick
  counts, put the flag back and say what a difference means.
- **dj counts the baseline's yields itself,** in the function the connector calls.
  `dj_clear` no longer touches a tick count, which fixes the low counts described under
  "What exists today".

**Before dj clears the flags at a yield, it gathers them for the other readers.** (Review
3.) The recorder and the GUI's live view read the flags after a frame. A clear in the
middle of a frame would lose what ran before it. So at match, mismatch, skip and error,
and at a sync, dj first adds the flags to the recording in progress and to the live view,
then clears.

**How a decomp is driven.** (Review 3; Thread. PLAN.md did not say.)

- **The call at a yield is the only way dj runs a decomp.** dj does not call `retro_run`
  on it. The call takes the place of `retro_run`, `dj_run`, `dj_check_parity` and
  `dj_parity_diffs`. A failed `dj_run` becomes the status "error".
- **The picture:** the core calls the libretro video callback whenever it has finished a
  picture, during a call. dj keeps the latest one and shows it at the frame end.
- **After a sync the core has no picture** until it has run. The comparison picture is
  therefore one yield late after every jump, where today it is drawn for the first frame
  shown.
- **A core that is an emulator is not driven this way:** see section 6.

**The handle at a yield reads memory and registers, and nothing else.** (Review 2.) That
is all the design asks of it there, and the baseline must not be run, saved, loaded or
reset while it is stopped. The handle is a pointer into the baseline, in the same
process (section 9), so nothing is copied unless the core copies it. Today's handle gives
a core any function of the baseline by name; the Tetris decomp uses it only for
`retro_get_memory_data` and `retro_get_memory_size`.

- **Nothing stops a core writing through that pointer.** (Review 3.) Today the handle
  points at a copy, so a write cannot reach the baseline. In the same process a write
  changes the baseline, and its state hashes change without any error.

**What this asks of a comparison core's author.** dj delivers yields; the core does the
rest. The list has grown with each review, and it is the real cost of the design:

- **The core runs one step behind.** It is told which yield the baseline reached, then
  runs itself to that point.
- **A value read through the handle is the baseline's value at that yield.** It is the
  value the game used only if a yield sits where the game uses it. (Review 2.) So each
  place where timing-dependent data enters the logic needs a yield. This adds yields and
  legwork, against "keep the list short". It cannot cover the idle-loop generator, which
  runs in a hot loop; that needs the patch.
- **The core says which yields it ignores and where it cannot judge.** (Review 3.)

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
  libretro, as the baseline does. Adding yields does not change this. (Developer,
  2026-11-13.)
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
- **The core copies the game's controller buffer from the baseline through the handle,**
  and answers "ignore".

How the two example games take input:

- **Tetris** reads the controller in its NMI handler, twice per frame. Its decomp reads
  the frame's input through libretro. That is enough: the game reads input once per
  frame.
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
  included.
- **The core gives one of three answers:** synced, declined, or failed. (Review 3: today
  `dj_sync` has only "synced" and "can't match it here", which the GUI shows as a
  problem.) "Declined" means "not at this yield" and shows nothing. "Failed" shows the
  reason and ends the offers until the next load or jump.
- **The offer says whether the load was a fresh power-on,** as `DJ_SYNC_BOOT` does today.
  (Review 3; Thread.) The baseline has run at least part of a frame by the first offer.
- **The core brings itself fully into step from the baseline.** dj does not manage a
  decomp's or a port's memory, and there are no checkpoints.
- **dj clears the coverage flags at a sync,** after gathering them (section 2). What ran
  before the sync is not credited to the first match after it. (Review 3.)
- **A project with only the frame end syncs at a frame boundary.** That works today for
  Tetris. A decomp of a game whose frame ends fall inside a tick declines the frame end
  and syncs at its tick.
- **Nothing before the sync point is checked.** Checking starts at the next yield. So the
  frame in which a sync happens is never checked. Today the sync comes before the first
  frame shown and that frame is checked.
- **A parent segment playing into its child is not a jump.** The core stays in step.
- **A `[WRITE]` line puts a decomp out of step.** (Review 2; thread's choice of fix.)
  The baseline gets the write at a frame start, which can be in the middle of a tick. A
  decomp parked at its last yield cannot apply it at the same moment. It syncs again at
  the next yield. `dj_write` goes. A comparison core that is the same emulator still gets
  the write.
  - **This costs Tetris something.** (Review 3.) Its decomp follows writes today through
    `dj_write`, because it is parked at a frame boundary. It will sync again instead, and
    the frame with the write is not checked.

### 4. On a mismatch or an error

- **Playback stops, at the frame boundary.** The baseline cannot be left stopped in the
  middle of a frame, so the frame finishes first.
- **There is no setting for this.** When the aim is parity there is no reason to keep
  playing.
- **The comparison core is out of step from the mismatch on.** (Review 3: PLAN.md said a
  user could step on by hand to look at the drift.) dj passes it no more yields in that
  frame, so by the definition of "in step" it must sync again, and the sync replaces the
  drifted state. The drift can be read from the reasons, which carry both sides' values.
  To watch drift play on, dj would have to keep passing yields after a mismatch. That is
  not in the design.

### 5. Frames stay underneath

- **Segments, inputs, snapshots and seeking stay in frames.** A tick-based baseline was
  considered and rejected.
- **Ticks may be shown on the timeline,** from a frame-to-tick map that dj records as
  segments play. The map's cache key must include the `[tick]` definition (section 1).
  Can wait.

### 6. The same emulator as the comparison core

Two emulators cannot each wait inside the other's call. So neither core is asked for a
verdict. Each runs the whole frame, and its connector calls dj at every yield as usual.
In that function dj records the yield and a hash of the core's memory, using the
project's `[hash]` regions. After both frames dj compares the two lists. With only the
frame end this is today's check. (Thread; the developer has not ruled on it.)

- **dj does the hashing, not the connector.** (Review 3: PLAN.md had each connector record
  the yield and the hash.) The core is stopped at the yield, so dj can read its memory
  the way it does for a state hash. Connectors need no new recording or hashing.
- **This is the one place where dj compares memory itself.** (Review 3: PLAN.md said dj
  never does.)
- **At the frame end dj still compares byte for byte and lists what differed,** as today.
- **At a yield inside a frame a hash gives no values.** (Review 3.) dj can name the first
  yield that differs, but not the bytes. To get them dj would replay the frame on both
  cores and copy memory at that yield. Not designed: see open problem 12.

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
  emulator. A decomp carries the change in its own build, and is handed each patch's
  hash at load.
- **The patch's hash is part of the setup.** A patch with a different hash means every
  segment is replayed. There is no extra confirmation: a segment plays, and the user
  saves it or does not.
- **Each core must take the ROM from the buffer.** Mupen64Plus-Next does. mGBA does when
  given one. Mesen reads the path, so its connector patch must make `retro_load_game`
  use the buffer, and make a power cycle (`Console::ReloadRom`) reuse that buffer.
- **The file format is not decided.** PLAN.md's thread chose BPS, because it checks that
  it is applied to the right ROM. Review 3: `[rom] sha256` already refuses any other ROM,
  a byte-swapped dump included, so that check adds nothing. Thread: a text file of
  offsets and bytes. A one-word patch can then be read and reviewed in git, and dj needs
  no BPS reader.

### 8. The connector API

Changed in place. dj is not released, so no older API is kept. Thread: raise the version
number anyway, so a core built before the change gets today's "rebuild the core" warning.

What changes (Review 3: PLAN.md did not list it):

| Side | Goes | Comes |
| --- | --- | --- |
| Baseline connector | `dj_set_tick_exec`, `dj_get_tick_count`, the tick count in `dj_clear` | One function that takes the list of yield addresses and the function to call at each. One function to read registers |
| Comparison core | `dj_run`, `dj_check_parity`, `dj_parity_diffs`, `dj_write`, `get_proc` by any name in the handle | The call at a yield, with its statuses and reasons. `dj_sync` takes the yield and answers synced, declined or failed |

### 9. Where the comparison core runs

Decided by the developer 2026-11-13: **in the GUI's process, not in a worker.** The same holds
for `dj parity`: both cores in the batch child.

- **Why:** the handle is a pointer, a yield in the middle of a frame needs no copy of the
  baseline, and there is no protocol between processes to extend.
- **It is not less work.** (Review 3: PLAN.md said the in-process code is what batch
  commands use. It is not.) `Checker`, `check_span`, the GUI and `dj parity` are all
  written against `RemoteComparison` and must move to `Comparison`.
- **What is given up in the GUI:** a crash in a decomp closes the GUI, where today it
  shows "the comparison core crashed at frame N" and the baseline plays on. A hang can be
  reported by the GUI's watchdog but not ended.
- **What is given up in `dj parity`:** (Review 3.) today a crash or hang in the decomp is
  a failure at that frame, and `docs/connectors.md` promises that for CI. In the batch
  child it ends the child. The supervisor then reports that the baseline crashed, leaves
  the segment out, and marks the segments below it blocked.
- **What covers it:** unsaved edits are saved to `.local/` as they are made, and a crash
  handler writes a note shown on the next launch (`ISOLATION_PLAN.md`, step 4).
- **To build with it:**
  - The crash note names the comparison core and what it was doing (Developer: worth
    implementing). The note's "call in progress" is one value today; the comparison core
    now runs inside the baseline's `retro_run`, so it must hold both.
  - The batch child's progress line says when the comparison core is running, so the
    supervisor can blame the right core and record a failure at that frame instead of
    leaving the segment out. (Review 3.)
- **OpenGL:** each core that asks for it already gets its own private context
  (`player/src/gl.rs`). The Tetris decomp draws in software.
  - **A decomp that draws with OpenGL is entered inside the baseline's `retro_run`.**
    (Review 3.) Its context is made current while the baseline's is. On macOS leaving
    puts the earlier context back. Elsewhere leaving makes no context current, so the
    baseline would go on without one. Entering and leaving must nest on every platform.
    This is the case to test, not Mupen64Plus against Mupen64Plus, which never nests.
- **This reverses step 2 of `ISOLATION_PLAN.md`** (agreed 2026-11-10). Thread: leave
  `RemoteComparison` in the code, unused, so going back is cheap.
- **A question for the developer** (Review 3; not a change). The handle was narrowed to memory
  and registers by Review 2, 82 minutes before this decision. A handle that small is a
  flat copy of memory: 2 KB on the NES, 8 MB on the N64. Shared memory to the worker
  would carry it, the worker's copy of the baseline core would go, and the crash
  isolation, open problem 1 and open problem 2 would not be in question. The cost is a
  round trip per yield and that copy. PLAN.md lists "the handle reading through a pipe or
  shared memory" as dropped; whether that was weighed with the narrowed handle is not
  recorded.

### 10. Parity results and tools

Review 3: PLAN.md did not cover these. Each is work the design needs.

- **A failure records the yield and its hit number,** besides the segment, the frame and
  where the core synced. The sync point is likewise a yield and a hit number. Without
  them a failure cannot be re-run to the same place. `parity_failures` gains columns.
- **`dj parity --explain` must stop at the yield.** It looks for the baseline's last write
  to each differing address in the frames traced. The baseline runs on to the frame end
  after a mismatch, so a search to the end of the frame can find a later writer. dj notes
  the trace position when a yield fires.
- **A parity run stores the four counts and the share judged,** in place of passed,
  skipped and unchecked frames.
- **The MCP parity tools, `dj parity`'s output and the GUI's failure list** show the
  yield.

## What is not in the design

Each of these was considered and dropped. The history file has the reasoning. The last
six rows are Review 3's.

| Dropped | In its place |
| --- | --- |
| Stopping the baseline and returning from `retro_run` at a yield | The blocking call |
| dj telling the core to "step one tick" | The core runs itself |
| A guard on N64 yields for which overlay is loaded | The core answers "ignore" or "skip" |
| The core subscribing to yields | Every yield is passed |
| `tick = true` on a yield | `[tick]` as its own table |
| A tick's optional start address | `[tick] exec` lists where a tick ends |
| A tick-based baseline | Frames, with ticks as a display choice |
| A halt setting, or the core asking for a halt | Always stop on a mismatch or an error |
| dj comparing memory, and ignore rules in the config | The core gives the verdict. Section 6 is the exception |
| Checkpoints of the comparison core, and per-yield records in the cache | The core syncs itself from the baseline |
| dj copying timing-dependent values from the baseline at each yield | A patch for state that only looks like an input, such as the idle-loop generator. For true inputs, such as a retrace counter or a clock, the core reads them through the handle, at a yield placed where the game uses them |
| A new time limit on the call | None in the GUI. Batch commands keep their rule: no frame within 30 seconds ends the chain |
| The comparison core in a worker process, and the handle reading through a pipe or shared memory | The comparison core in dj's process. See the question in section 9 |
| A special "sync at the frame start when the game is waiting" path | The frame end is a yield like any other |
| dj recording each controller read and handing its bytes to the core, with a hook in each connector | A yield at the game's input routine; the core copies the buffer |
| Patch bytes written in the config, and patching RAM after boot | A patch file applied to the ROM before boot |
| Yields a connector provides by name (`CORE:nmi`) | The frame end is dj's own; anything else is an `exec` address |
| One "skip" for both a disregarded yield and an unjudged stretch | "Ignore" and "skip" |
| A yield over the cap as an error that stops playback | A warning; the yield is off until the frame ends |
| The core reporting "completed a tick" | Nothing, until dj has a use for the count |
| The frames since the last call, the count since the segment start, and the N64 thread, in the call | The current frame. The core reads the thread itself |
| Each connector recording yields and hashing RAM (section 6) | dj hashes in the function the connector calls |

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

1. **A decomp that corrupts memory before it crashes.** With the comparison core in dj's
   process (section 9), a buggy core can damage dj's own memory, or the baseline's
   through the handle. Saved edits cover a crash; they do not cover a backup written from
   damaged memory. Thought unlikely; no answer.
2. **A comparison core that draws with OpenGL, in dj's process.** Not tried. The case
   that matters is a decomp entered inside the baseline's `retro_run` (section 9). Also
   to test: switching between three contexts on macOS, and on Linux the existing
   workaround for a thread that cannot hold a GLX and an EGL context at once.
3. **The names** of the call and of the struct it returns.
4. **The N64 connector has no yields.** Its tick functions are stubs. The game runs on a
   coroutine whose stack may be too small for the call. Review: the connector can switch
   to the main coroutine to make the call, without returning from `retro_run`, so the
   game's stack stops mattering. Not checked: whether anything treats that switch as the
   end of the frame.
5. **Reading registers.** The connectors have no function for it, and the handle at a
   yield should offer one. On the N64 the core needs it to find the running thread.
6. **A load must not fire yields.** On Mupen64Plus a load ran 1,951 distinct instructions
   inside `retro_unserialize`.
7. **Where a yield goes in a spin loop.** It must be an instruction that runs once per
   wait. In Thwaite's boot, one loop ran its compare 2,693 times in a frame.
8. **dj's own tick count with no core loaded** counts every hit at the `[tick]` address,
   including other code at that address on the N64. A project should pick an address in
   code that is always loaded. The review suggests the yield also names the instruction
   expected at the address. Thread: one word is a weak test, because functions often
   start with the same instruction; add an optional check only if a project has no
   always-loaded address for its tick. The developer dropped the guard and has not reopened it.
9. **For the patch:** the file format; whether a game's overlays are stored compressed,
   which a byte patch cannot reach; the ROM offset of No Mercy's idle-loop call. The N64
   boot checksum is set aside.
10. **Two Mesen bugs on the roadmap** change counts, not RAM: the lag counter after a
    load, and a port keeping a device from an earlier segment.
11. **A rebuilt ROM that does not match, run in an emulator** (Review). It cannot use
    section 6: its addresses differ and its RAM cannot be hashed whole. It also cannot
    "run itself" to its own yield, because an emulator core only runs whole frames. No
    answer yet. How common the case is for dj's users is not known.
12. **Both sides' values at a yield inside a frame, for two emulators** (Review 3,
    section 6). A hash names the yield but not the bytes.
13. **The cost of the flags at a yield on the N64** (Review 3). Taking and clearing them
    means reading 8 MB of `RDRAM` flags and clearing those and the `ROM` region's. Today
    that happens once per frame and already halves playback speed (roadmap, "N64
    follow-ups"). A project with several judged yields per frame pays it at each one.
14. **What the handle's memory looks like on the N64** (Review 3). Mupen64Plus keeps RDRAM
    as host-order 32-bit words, and its connector hands out no bytes for that region. A
    decomp reading through the handle must know the order, or the handle must say.
15. **`ROADMAP.md` is out of date** (Review 3). "Planned before the release: parity at
    check points" describes the abandoned design, and the patch entry says parity "still
    needs an ignore rule", which this design leaves to the core.

## Build order

Review 3's proposal; Thread. PLAN.md moved the comparison core into the GUI's process
first. That gives up a shipped and tested protection before either spike has shown that
the design works. The N64 holds the most unknowns and is the reason for the design, so
its spike comes before the move too. Each step says what "done" looks like.

1. **A spike in the Mesen connector.** One yield at Thwaite's game wait and the blocking
   call. It needs no comparison core. Done when the yields and RAM hashes are the same
   across two runs and after loading a snapshot, state hashes are the same as without
   the yield, and the cost per yield is measured.
2. **A spike in the Mupen64Plus connector.** One yield at `match_tick` and the blocking
   call from the game's coroutine, or from the main one (open problem 4). Done when the
   yield count is the same from power-on and from a snapshot, a load fires none, and
   state hashes are the same as without the yield. If this cannot be made to work, stop
   and rethink before step 3.
3. **Move the comparison core into dj's process,** in the GUI and in `dj parity`, and
   make the crash note and the supervisor name it. Done when the existing comparison
   features and `tests/compare.rs` pass without a worker, and a core that crashes on
   purpose is named in the GUI's note and as a failure at its frame by `dj parity`.
4. **Two stub comparison cores.** One copies the baseline at every yield and always
   answers match. One diverges on purpose. Done when match, mismatch with both sides'
   values, skip, ignore, error, a declined sync, a failed sync and the stop at the frame
   boundary have each been seen, and a skipped stretch has been shown to credit no
   coverage.
5. **The Tetris decomp at a yield inside the frame.** (Review 3.) Give the Tetris project
   a `[tick]` at its game wait and have the decomp sync and judge there instead of at the
   frame end. It is the only real decomp at hand, and at the frame end it would exercise
   none of the new path. Its 15 lag frames test the limit on judging. Done when its 30
   segments pass, with the lag frames skipped or explained.
6. **The patch feature,** with the Mesen buffer change. Independent of the rest. Done
   when, with the patch, No Mercy's generator state does not change across frames that
   have no game-loop pass, and without the patch it does. (Review 3: PLAN.md's test was
   that a match replays to the same hash twice. The emulator already does that without
   the patch.)
7. **N64 yields in full.** `[tick]` at `match_tick` and a `[[yield]]` at `pads_read`.
   (Review 3: PLAN.md put `[tick]` at `pads_read`, against section 2a.) Done when both
   count the same from power-on and from a snapshot, and a stub core has copied the pad
   buffer at `pads_read`.
8. **Parity results and tools** (section 10). Done when a failure at a yield inside a
   frame can be re-run and explained from the command line.
9. **The frame-to-tick map and ticks on the timeline.** Can wait.

A real decomp of a game whose ticks cross frames is the last test, and the only one that
proves the design. None is at hand as a libretro core.

## What changed from PLAN.md

All from Review 3, and all unconfirmed.

Corrections of fact:

- Batch commands run the comparison core in a worker today, as the GUI does (What exists
  today, section 9).
- With no `[tick]`, no ticks are counted.
- Step 4's test for the patch could not fail.

Holes filled:

- "Skip" split into "skip" and "ignore", and flags cleared at a skip and at a sync
  (sections 2, 3).
- The cap is a warning, not an error (section 1).
- dj gathers the flags for the recorder and the live view before clearing them at a
  yield (section 2).
- How a decomp is driven, and which functions go (sections 2, 8).
- `dj_sync` answers synced, declined or failed (section 3).
- OpenGL contexts must nest (section 9).
- Stored parity runs and tick counts are keyed by the yield definitions (section 1).
- Failure records, `--explain`, and the tools (section 10).
- A crash in a decomp under `dj parity` (section 9).

Contradictions removed:

- Stepping on after a mismatch to look at the drift (section 4).
- "dj compares no memory" against section 6.
- The frame end as the connector's yield in one place and dj's in another (section 1).
- `[tick]` at `pads_read` in the build order.

Taken out:

- `CORE:name` yields.
- The frames since the last call, the count since the segment start, and the N64 thread.
- The "completed a tick" flag.
- Recording and hashing in each connector for section 6.
- BPS as the thread's choice of patch format.

Reordered:

- Both spikes before the move into dj's process; the Tetris decomp at a yield inside the
  frame added as a step.

Left for the developer:

- Whether the narrowed handle changes the case for the worker (section 9).
