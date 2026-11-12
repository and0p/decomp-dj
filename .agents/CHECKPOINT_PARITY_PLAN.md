# Plan: parity at check points, not frame ends

Status: written 2026-11-11, nothing built. **Before the first release** (decided
2026-11-11): the comparison core API changes shape here, so it should change before
anyone writes a decomp against the old one. Comes from the Thwaite decomp
([THWAITE_DECOMP_PLAN.md](THWAITE_DECOMP_PLAN.md)), whose comparison core has to line its
frames up with the original's. Answers the ROADMAP's open question "whether the check runs
every frame or only at ticks".

## The problem

Parity is checked when both cores have finished a frame (`retro_run`). That assumes the
original's logic for the frame is done by then, with the CPU waiting for vblank. It holds
for most frames of a well-behaved 8-bit game, but not all, and not at all for systems
whose games don't run one logic step per frame:

- **Lag frames.** When logic runs past vblank, the frame ends with the original mid-logic.
  A rewrite that runs one logic step per frame is then ahead of it, and the check fails
  though nothing is wrong.
- **Variable logic rates.** Most N64 and PlayStation games run their logic at 30 or 20 Hz,
  or whenever the last step finishes. Frame ends there land anywhere in a step.
- **Which screen the original is in.** `dj_sync` needs to know which loop the original is
  waiting in (title, practice menu, pause, a cutscene step, the game). dj gives it RAM and
  nothing else, so the Thwaite decomp plans to match return addresses on the stack page
  against known call chains. That works, but it's a heuristic, and stale stack bytes can
  fool it.
- **Ticks are a counter only.** `[tick]` counts how many times one ROM location ran
  (`doStateAction` in Thwaite). It tells a lag frame in the main game loop from a normal
  one, but the menus, cutscenes and pause have their own wait loops and run no ticks. One
  count also doesn't say where the CPU is.

**And a bug, found while reading for this plan:** the decomp's lag signal, the
baseline's `dj_get_tick_count` read through `get_proc`, always reads 0 under dj-gui and
`dj parity`. Both run the comparison core in a worker process. There, the handle points to
a *copy* of the baseline that only ever has the baseline's save state loaded into it
([connectors.md](../docs/connectors.md#comparison-cores)). The copy never runs a frame and
never has its tick location set, and the tick count isn't part of a save state. Only the
in-process `Comparison` (used by tests) hands over the real baseline.

## The shape of it

The project lists **check points**: places in the original's code where one step of its
logic is done (each loop's wait, or the top of the loop). The baseline runs **until it
reaches a check point or the frame ends**, instead of whole frames. At a check point:

1. The baseline stops before the instruction there, mid-frame.
2. The comparison core runs to its own next check point and says which one it reached.
3. If they reached different check points, that's a failure in itself ("the decomp went to
   `titleWait`, the original to `pauseWait`"). Otherwise, if the check point is checked,
   parity is checked there.
4. The baseline goes on from where it stopped.

A frame can hold zero, one or more check points: a lag frame has none and the next has two.
A decomp written as a state machine runs one state per call, so lag lines up by itself, and
`dj_sync` is told the check point's name, so it never guesses which loop the original is
in.

It comes in two parts, both before the release:

- **Part 1, the handle and the CPU.** Frame facts handed to the comparison core (fixes the
  bug), the baseline's memory handed over instead of a save state, and the CPU's state. No
  emulation changes. Makes frame-end checking sound, and is what Part 2 is built on.
- **Part 2, check points.** Configuration, the connector calls, checking and syncing at
  check points, and everything that names a failure.

## Configuration

```toml
[[checkpoint]]
name = "game"            # shown in tools and failures
exec = "doStateAction"   # a symbol, or a location like PRG:0x197

[[checkpoint]]
name = "pause"
exec = "pauseScreen+0x12"
check = false            # line up and sync here, but don't check

[[compare]]
name = "decomp"
check_at = "checkpoints" # or "frames"
```

- **Check points belong to the project:** they're facts about the original game, shared by
  every comparison core. Symbols resolve through the symbol map
  (`query::Project::resolve`).
- **`check` (default true):** every check point lines the cores up and can be synced at.
  `check = false` makes it a place to line up and sync without checking: a loop where the
  original keeps scratch state in RAM, a screen the decomp handles differently on purpose,
  or a point that's only there so `dj_sync` knows where it is.
- **`check_at` belongs to the comparison core,** since what's right depends on what's
  compared. A decomp wants `checkpoints`, because its frame ends don't line up with the
  original's under lag. Another build of the same emulator (`sync = "state"`) can check
  every frame end exactly and wants `frames`. The default is `checkpoints` when the
  project has check points and the comparison core can run to them, else `frames`.
- **No "both" for now.** With check points on, the comparison core moves from check point
  to check point, so at a frame end it may be part-way through a step the original
  finished. A frame-end check there would fail for no reason. It can come later as "check
  frame ends where no check point is pending", if someone needs it.
- **Frames with no checked check point** (a lag frame, or a stretch of `check = false`
  points) count as not checked, like skipped frames today, so they show as such on the map
  rather than passing silently.
- **`[tick]` stays** for counting. A project can use its tick location as one of its check
  points.

## Part 1: the handle and the CPU

1. **What the frame did, handed to the comparison core.** `struct dj_baseline` goes to
   version 2, with fields after the current ones (a version 1 core still works: it reads
   only the first three):

   ```c
   struct dj_baseline {
       uint32_t version;           // 2
       void    *ctx;
       void    *(*get_proc)(void *ctx, const char *name);
       // version 2:
       uint64_t frame;             // power-on frame running or just run
       const char *checkpoint;     // the check point the baseline is stopped at, or NULL at a frame end
       uint32_t checkpoint_index;  // which check point of this frame (0, 1, ...)
       uint32_t has_ticks;         // 0 when the project has no [tick] or the connector counts none
       uint64_t ticks;             // the baseline's tick count since the segment started
       uint64_t frame_ticks;       // ticks so far in this frame
   };
   ```

   dj fills them from the real baseline, and the worker passes them on with each request.
   The decomp thread needs to know now: its lag rule should read `frame_ticks`, never
   `dj_get_tick_count` through `get_proc`.

2. **The baseline's memory, not its save state.** Today the worker keeps a copy of the
   baseline core and loads the baseline's save state into it before every sync, write and
   check, so `get_proc` reaches real functions. That can't carry on mid-frame:

   - **N64 (Mupen64Plus):** a save waits for a safe point (the connector makes it hand
     back control right after, but only at those points).
   - **SNES (bsnes):** a save runs the other chips forward to where they can be saved.
   - **Cost:** serializing and loading every check point costs more than the check.

   Instead, dj sends the baseline's memory regions (the libretro memory ids and the
   connector's regions with `data`) and its CPU state. In the worker, `get_proc` answers
   the read-only functions from that copy: `retro_get_memory_data` and `_size`,
   `dj_region_count`, `dj_get_region`, `dj_get_cpu_state`, `dj_get_tick_count` (from the
   version 2 fields). Anything else gets NULL, and the docs say what's there.

   - Only what the comparison core reads is sent. A new optional `dj_wants_regions` lists
     them; without it, system RAM.
   - Big regions (N64's 8 MB of RDRAM) go as pages changed since the last check, as the
     snapshot store already does.
   - **Keeping the old behaviour:** a project can still ask for a full copy of the baseline
     (`[[compare]] baseline = "copy"`) for comparison cores that need more. It works only
     at frame ends, and only for baselines whose saves are exact there.

3. **Where the CPU is.** A new optional connector function, read-only:

   ```c
   struct dj_cpu_state {
       uint32_t cpu;              // which CPU, when the system has several (0: main)
       uint64_t pc;               // address of the next instruction
       int32_t  region;           // where pc maps, as in dj_get_region, or -1
       int64_t  offset;           // offset in that region, or -1
       uint64_t cycle;
       int32_t  scanline;         // -1 where the system has none
       uint32_t reg_count;
       uint64_t regs[40];         // the CPU's registers in a documented order per cpu
   };
   bool dj_get_cpu_state(uint32_t cpu, struct dj_cpu_state *out);
   ```

   - Mesen first (`CPU::GetState()` and the mapper's address translation), then mGBA and
     Mupen64Plus.
   - dj shows the frame-end PC beside the frame and tick in the MCP `step` and `trace`
     tools and the GUI's status. That answers the decomp's milestone 0 question directly,
     without a trace of every frame.

4. **Tests.** The test core grows `dj_get_cpu_state` (a fake PC that moves) and a
   `testcore_break = "lag"` mode that reads `frame_ticks`. Tests through `RemoteComparison`
   check that the comparison core sees the baseline's numbers and memory, not a stale copy.

## Part 2: check points

### Connector API

**Baseline side** (the emulator's connector), optional:

```c
// The check points: n locations (region index and offset, as in dj_set_tick_exec), and
// for each the CPU it applies to (0: main), for systems with several.
void dj_set_checkpoints(const int32_t *regions, const int64_t *offsets, const uint32_t *cpus, size_t n);

// Run until a check point is about to execute, or the frame ends. Returns the check
// point's index, or -1 when the frame finished (as retro_run would have, video and
// audio included). A frame in progress is continued; retro_run still runs a whole frame.
int32_t dj_run_to_checkpoint(void);
```

**Comparison side** (the decomp), optional:

```c
// Run until the next check point, by name (one of the project's [[checkpoint]] names), or
// the frame ends. 0: reached the one named in `reached`. 1: the frame ended. Anything
// else: an error (fill in `reason`).
int dj_run_to_checkpoint_named(char *reached, size_t reached_len, char *reason, size_t reason_len);
```

A decomp written as a state machine implements this as "advance one state". A comparison
core that is the same emulator (`sync = "state"`) is driven with `dj_run_to_checkpoint`
like the baseline.

**Determinism is the first test** for every baseline: stopping between instructions must
not change emulation. Every segment's hashes must be identical with check points on and off,
as for the coverage hooks
([connectors.md](../docs/connectors.md#writing-one-for-another-core), step 4).

### Syncing at check points

Today a sync happens at a frame start: after a jump, playback syncs the comparison core
before the first frame it runs. **This is not handled for check points yet**; it's part of
this plan:

- **Where:** after a jump, the baseline plays from the frame it landed on until its first
  check point, and the sync happens there. `dj_sync` gets the check point's name and index
  in the version 2 handle. The frames before it aren't checked. Snapshots and seeking stay
  on frame boundaries; nothing else needs mid-frame states.
- **Boot:** `DJ_SYNC_BOOT` stays at frame 0 of a root segment. A decomp that boots itself
  then runs to its first check point alongside the baseline, and the first check happens
  there.
- **`sync = "state"`** (the same emulator) loads the baseline's state into the comparison
  core at the check point. That needs mid-frame saves that are exact in that emulator
  (Mesen and mGBA, yes; Mupen64Plus and bsnes, no). Where they aren't, the sync falls back
  to the frame start, and the comparison core is run to the same check point.
- **Failures record where the sync was:** (segment, frame, check point index), so re-runs
  and `--explain` start there.
- **`[WRITE]` lines** still apply at frame starts on the baseline, where it may be
  mid-step. The comparison core gets `dj_write` at the same moment, whatever check point it
  last reached. For writes to persistent state (Thwaite's `gameDay`) that's fine. A write
  that changes what a step in progress does would need writes keyed to check points: open.
- **A sync at a tick** is the same thing, with the tick location as a check point.

### Input

Inputs are set per frame, but a step can span two frames, and the original reads the
controller at some moment within it. The comparison core's step has to see what the
original saw:

- **Now:** when the comparison core runs a step, dj gives it the inputs of the frame in
  which the baseline started that step (the frame of the previous check point). That's
  right for games that read the controller once at the top of the loop.
- **Later:** the connector records when the original read the controller (Mesen sees
  `$4016`/`$4017` reads), and dj hands over the inputs live at that read.
- **Inputs that change within a frame** are a different feature: sub-frame input in
  segment files. It goes after the release (ROADMAP, "Sub-frame inputs").

### What changes in dj

- **Library:**
  - `Core` gets `run_to_checkpoint`.
  - `Session::step` becomes a loop over a frame's check points, with a callback at each.
  - `parity::Checker` checks at check points under `check_at = "checkpoints"`, syncs at
    the first one after a jump, and records failures by frame and check point.
  - Coverage flags are taken at each check point, so a failure marks only the code since
    the previous check point, not the whole frame. That's much finer parity per block, and
    better suspects for `--explain`.
- **Worker protocol:** `step` runs to one check point at a time. The memory (Part 1) and
  the version 2 handle fields go with every request.
- **Database:**
  - `parity_failures` gains `checkpoint` and `checkpoint_index` (NULL for frame-end
    failures). It's the first column added to an existing table, so it needs a small
    migration on open (`PRAGMA table_info`, then `ALTER TABLE ... ADD COLUMN`).
  - `parity_runs` counts check points checked instead of frames under `checkpoints`.
- **Re-run and `--explain`:** replay to the failing frame, then to the failing check point
  in it. `--explain`'s trace window is "since the previous check point", which is shorter
  and more exact than N frames back.
- **GUI:**
  - Frame-by-frame stepping stays by frame.
  - The status bar shows the check point the comparison stopped at.
  - A failure's message names it ("Parity failure at frame 812, check point `game`, the
    2nd this frame").
  - The comparison core's picture updates at frame ends only.
- **Tools:** `dj parity` and the MCP jobs report check points checked; `parity_failures`
  and `parity_explain` name them. The MCP `step` tool can gain "to the next check point"
  later.

## Other consoles

What a check point needs from a system: a CPU whose instructions dj can stop between, an
address that means one piece of code, and a way to hand the comparison core the baseline's
memory there. How the systems dj targets compare, from their emulators' structure (only
Mesen, mGBA and Mupen64Plus have connectors):

| System (core) | Stopping mid-frame | What a check point is | Watch out for |
| --- | --- | --- | --- |
| NES (Mesen) | Easy: `RunSingleFrame` loops `_cpu->Exec()` until the frame count changes; split it at a stop flag set by the opcode hook | A PRG location (bank-switched: the connector already maps to ROM offsets) | Logic in the NMI handler (check point at its end); mid-frame saves are exact |
| SNES (bsnes; snes9x) | bsnes: cooperative threads (CPU, SMP, PPU, coprocessors), stopping means leaving the scheduler at an instruction boundary of the CPU thread, as its debugger does. snes9x: one main loop, simpler, less accurate | A ROM location on the main CPU, or on the SA-1 or SuperFX for games whose logic runs there (`cpu` in `dj_set_checkpoints`) | bsnes saves run the other chips forward, so mid-frame saves aren't exact: memory handover only (Part 1). No connector yet |
| Game Boy / GBA (mGBA) | Easy: the frame loop runs the CPU until the frame counter moves; the connector already hooks `ARMStep` per instruction | A ROM location, or IWRAM/EWRAM for code copied to RAM (same address problem as N64, smaller) | Most games wait with `VBlankIntrWait` (a SWI): a check point at the call; mid-frame saves are exact |
| Genesis (Genesis Plus GX) | Medium: the frame runs scanline by scanline, the 68000 a cycle budget per line. Stopping mid-line needs an exit inside the 68000 loop and resuming the rest of the line's budget | A ROM location on the 68000 | Saves only at frame boundaries: memory handover only. The Z80 (sound) is ignored. No connector yet |
| N64 (Mupen64Plus-Next) | Feasible: dj already forces the pure interpreter, which calls the connector for every instruction, and the core runs in a libco coroutine, so the hook can switch back to dj (`co_switch`) and resume later | An RDRAM address, but code there is loaded from ROM: the same address holds different code in different overlays. A check point needs the code it expects too (the first instruction words, or the overlay's ROM offset once `dj_origin` exists) | Game logic is a thread under libultra: other threads (audio, graphics, the scheduler) are mid-work at any check point, and the RSP and RDP write RDRAM on their own. Their buffers need `ignore` policies. Logic often runs at 20–30 Hz, so frame-end checks are meaningless here and check points are the only way. Saves wait for safe points: memory handover only, and `sync = "state"` syncs at frame starts. Ticks aren't counted yet either |
| PlayStation (Beetle PSX, SwanStation) | Feasible with the interpreter; the recompilers need an exit at check-point addresses (end a block there) | A RAM address (code is loaded from disc), with the same overlay problem as N64 | Variable logic rates like N64; GPU and SPU DMA write RAM on their own. No connector yet |

What this says about the design:

- **Check points need a CPU index** (SNES coprocessors; later, systems with two main CPUs
  like the Saturn) and, for RAM-loaded code, **a check on the code** (N64, PlayStation,
  GBA code in RAM). `[[checkpoint]]` grows `cpu = "sa1"` and `code = "auto"` (dj takes the
  first instruction words from the symbol map's ROM bytes) when those systems get
  connectors. The API above already has `cpus`.
- **Handing over memory instead of a save state** (Part 1) is what makes the plan work on
  bsnes, Genesis Plus GX and Mupen64Plus, whose mid-frame saves aren't exact or don't exist.
  Without it, check points would be NES- and GBA-only.
- **Frame-end checking stays the default** for systems and comparison cores without check
  points, so nothing that works today breaks.
- **N64 is the strongest case for check points:** its frame ends almost never line up with
  logic. It also needs ticks counted in its connector, and the overlay check.

## Steps

All before the release.

1. **Part 1: the version 2 handle.** dj fills frame and tick facts from the real baseline.
   Tell the decomp thread about the tick bug at once.
2. **Part 1: memory handover.** The worker answers `get_proc` from the baseline's
   regions, `dj_wants_regions`, changed pages for big regions, and `baseline = "copy"` for
   the old behaviour.
3. **Part 1: `dj_get_cpu_state`** in Mesen and the test core; the frame-end PC in tools.
4. **Mesen's `dj_run_to_checkpoint`**, with the determinism test on every Thwaite segment,
   and the slowdown measured with no check points set (should be none) and with some.
5. **The test core** gets check points (a fake loop, with a "lag" option that spans frames)
   and `dj_run_to_checkpoint_named`.
6. **Library and worker:** run to check points, check and sync there, coverage per check
   point, `check` and `check_at`. Tests through `RemoteComparison`.
7. **Database and tools:** the columns and migration, `dj parity`, re-run and `--explain`
   by check point, MCP.
8. **GUI.**
9. **Docs:**
   - `playback.md`: a "Check points" section beside ticks.
   - `connectors.md`: the API, the version 2 handle and what `get_proc` answers.
   - `config.md`: `[[checkpoint]]`, `check_at` and `baseline`.
   - `gui.md`, `cli.md` and `mcp.md`: the updates.
10. **On the Thwaite decomp:** a check point for each wait loop, and all 12 segments pass,
    lag frames included.
11. **mGBA and Mupen64Plus:** `dj_get_cpu_state` and `dj_run_to_checkpoint`, with the
    determinism test on their test projects. For Mupen64Plus, ticks and the overlay check
    too. Could follow the release if it slips, since frame-end checking keeps working
    there.

## Open questions

- **Logic in the NMI handler.** Thwaite runs its logic in the main loop, but many games
  run it in NMI. A check point at the end of the NMI handler should work the same way, but
  needs trying on such a game.
- **A loop with no wait** (a busy cutscene, a decompression loop over several frames). It
  has no check point, so frames inside it aren't checked. Is that acceptable, or does it
  need a check point at "frame end, if no check point ran"?
- **Writes keyed to check points**, for pokes that change a step in progress (above).
- **How much memory to send per check point** on N64 when the decomp wants all of RDRAM:
  measure the changed pages per check point on a real game.
- **Naming.** "Check point" here; "step" is taken by frame stepping. Change it before it
  reaches the docs, if there's a better word.

## Review findings (2026-11-11 10:47 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-11 10:47 EST.
> Threads that Andrew has not given permission to edit this section must not change it:
> not to correct it, not to mark items done, and not to reword it as the task progresses.
> Record progress, answers and disagreements in the sections above, or in a new section
> below this one. Only Andrew, or a thread he has told to, edits what is here.

A review of this plan against the code and the other plans, with sub-frame input included.
Everything here comes from reading the code and docs. Nothing was run, so each item is to
be confirmed before it is acted on.

### Holes that break the design

1. **dj can't supply `ticks` or `frame_ticks` today.**
   - `dj_clear` zeroes the tick count as well as the flags, and parity clears every frame
     (`take_code_flags`, `player/src/parity.rs`). The GUI clears every screen refresh
     (`gui/src/cov.rs`).
   - With coverage taken per check point, the count would reset mid-frame.
   - The count isn't in a save state, so "ticks since the segment started" is unknown
     after any seek.
   - Fix: separate clearing flags from resetting ticks in the connector, and have dj
     accumulate. Define `ticks` as "since the sync", or store it with snapshots.
   - This is already a bug: a pass recorded in the GUI stores an undercounted
     `contents.ticks` (`Session::finish`, `player/src/player.rs`).
2. **"Stops before the instruction" doesn't match where Mesen's hook is.**
   - `OnExec` runs inside `CPU::MemoryRead`, after `StartCpuCycle` has advanced the PPU and
     APU for the opcode fetch. A stop flag set there can only take effect after that
     instruction finishes.
   - Either define a check point as "after the instruction at this location ran", or
     compare the PC before `Exec`, which costs an address translation per instruction.
   - It also decides what `dj_cpu_state.pc` means.
3. **The comparison core has no frame end in check point mode.**
   - `dj_run_to_checkpoint_named` returns 1 for "the frame ended", but a state-machine
     decomp has no frame ends of its own.
   - Nothing says when it draws or sends audio. "The picture updates at frame ends" needs
     a call (a present, or a `retro_run` that doesn't advance).
4. **"Checkpoint" is already a segment header field.**
   - `checkpoint: <frame> <hash>` is in `docs/segments.md`, and the word appears about 110
     times in the code (`Recorder.checkpoints`, `checkpoint_miss`).
   - "Naming" under Open questions only notices "step". The name is in the version 2
     struct and three function names, so settle it at step 1, not before the docs.

### Smaller holes

- **Result keys ignore the check point config.** `parity_runs` skips on (core, build,
  content). Changing a `[[checkpoint]]`, `check` or `check_at` changes the result but not
  the key, so stale passes get skipped. Add a hash of the check point config to the key.
- **One step per segment is never checked in `dj parity`.** Each segment syncs at its first
  check point, so the step that spans the parent's end is synced, never checked. Carry on
  from the parent in tree order (`Checker::enter` already does this).
- **N64 cost.** `take_code_flags` copies and clears whole regions per check: 8 MB of RDRAM
  flags plus 32 MB of ROM, several times a frame. The ROADMAP measured 28 ms for one scan.
  The N64 connector needs dirty pages before check points are usable there. Also `REGION`
  is hard-coded to `"PRG"` in `player/src/parity.rs`.
- **Byte order in the memory handover.** RDRAM is host-endian words (the ROADMAP's
  `[WRITE]` bug). The handed-over regions, diff addresses and the RAM fallback need one
  stated order.
- **Two handle behaviours remain.** In-process `Comparison` hands over the real core and
  the worker a copy; that split caused the tick bug. Make both answer from the same
  snapshot handle.
- **`get_proc` returning NULL for `retro_serialize`** breaks what `docs/connectors.md`
  promises today. The Tetris decomp only reads memory, so it's safe.
- **Not covered:** `[RESET]` mid-step, impulse inputs on a step that spans two frames, and
  a misplaced check point hit thousands of times a frame (each a worker round trip; needs
  a cap).
- **Version numbers.** `dj_baseline` version 2 sits beside connector API 2, and the plan
  doesn't say whether the new connector calls bump the API. The patch change also alters
  `dj_build_id`, so Thwaite and CI need `dj repin`.
- **Ordering with the GUI's core in a worker** (ISOLATION_PLAN, "maybe before the
  release"). Memory would then cross two processes per check point. Decide which lands
  first.

### Missed opportunities

- **Ticks are check points that are only counted.** One mechanism gives named counts per
  check point, removes the single-location limit, and gives N64 ticks for free.
- **Poll counting, already planned in TAS_PLAN step 4.** Counting `input_state` reads per
  frame gives a lag-frame fact for any core with no connector work. It could also replace
  "Later: the connector records when the original read the controller" under Input, if
  Mesen reads input when the game does. That wasn't checked.
- **Run-to-location is a general breakpoint.** With `dj_get_cpu_state` it could back an MCP
  `step until` at a code location, not only parity.

### Sub-frame input

- **No plan exists.** The ROADMAP item doesn't cite TAS_PLAN's `.m64` poll clock or the
  BizHawk sub-frame input that `player/src/tas/bk2.rs` currently drops with a warning.
- **Reserve the segment syntax now.** Older tools reject unknown line forms, so adding it
  after the release is a format break.
- **Pick the key before the version 2 handle freezes.**
  - Poll index is emulator-agnostic and matches `.m64`.
  - Check point name ties segment content to `config.toml`, so the content key would need
    the check point list.
  - CPU cycle is core-specific.
- **The comparison side needs it too.** The input rule ("the frame the step started in")
  and `dj_run_to_checkpoint_named` have no way to pass per-read input later without
  another API change.

## Response to the review (2026-11-11 10:52 EST)

Written by the thread that wrote the plan. Each point below was checked against the code
where it could be; "checked" says so. The plan above isn't edited yet. Once Andrew agrees
with these, the changes go into the sections above and this one stays as the record.

### The four holes

1. **Ticks: agreed, and checked.** Mesen's `dj_clear` sets `_tickCount = 0`. dj clears
   coverage at segment start (`player.rs`), every checked frame (`take_code_flags`) and
   every GUI refresh (`cov.rs`), and `Session::finish` reads the count once at the end. So
   `contents.ticks` undercounts whenever the GUI or parity ran.
   - **Fix with no connector change:** dj reads the tick count before every
     `coverage_clear` and adds it to a running total kept in `Session`. One wrapper
     (`Core::take_ticks`) used by every clear. This fixes the existing bug, and it should be
     done now, separately from this plan.
   - **Definition:** `ticks` in the handle is "since the last sync", which dj always knows.
     "Since the segment started" would need the total stored with snapshots; drop it unless
     something needs it.
   - The connector API could still split clearing flags from resetting ticks later; with dj
     accumulating, nothing needs it.
2. **Where Mesen stops: agreed, and checked.** `OnExec` runs in `CPU::MemoryRead` after
   `StartCpuCycle` for the opcode fetch, so stopping there is mid-instruction.
   - **Proposal:** check before `Exec` in the frame loop, on the CPU address. When check
     points are set, the connector keeps their CPU addresses (a check point's ROM offset
     maps to one CPU address per bank layout). Before each `Exec` it compares `_state.PC`
     against that small set, an integer compare, and only on a match translates the
     address to confirm the bank is the one meant.
   - That gives "stopped before the instruction at the check point", which is what the
     decomp's state machine wants (the wait is next), and `dj_cpu_state.pc` is the check
     point's address.
   - The determinism test covers it, and the slowdown is measured in step 4 as planned.
3. **The comparison core's frame end: agreed.** A state-machine decomp in check point mode
   is never asked to run a frame.
   - **Proposal:** an optional `void dj_present(void)` that dj calls on the comparison core
     when the baseline finishes a frame. It sends the current picture through the usual
     video callback without advancing anything. Without it, the comparison core's picture
     isn't shown in check point mode, and the GUI says so.
   - Audio from the comparison core isn't compared or played in this mode; document it.
   - `dj_run_to_checkpoint_named`'s "1: the frame ended" becomes "1: no check point
     reached" (a decomp state that has none; it's reported, not treated as a frame end).
4. **The name: agreed, and checked** (`checkpoint:` is a segment header, and the word
   appears about 110 times in the code). Settle it at step 1.
   - **Proposal: "yield point".** It names what these places are (where the original
     yields: a wait loop, a VBlank wait, `osRecvMesg` on N64), and it's free in the code
     and docs. Config `[[yield]]`, API `dj_set_yield_points`, `dj_run_to_yield`, handle
     field `yield_point`.
   - Alternatives: "logic point", "wait point", "sync point" (but these also check).
   - Andrew picks.

### Smaller holes

- **Result keys: agreed.** Add a hash of the yield point config (locations, `check`) and
  the entry's `check_at` to the `parity_runs` key.
- **The step across a parent's end: agreed.** `dj parity` should carry on from the parent
  in tree order with `Checker::enter`, as `--rerun` already does through child segments.
  It syncs at the first yield point only when the parent wasn't run in this pass (skipped
  as already checked, or dirty). The step that spans the boundary is then unchecked, and
  counted as such.
- **N64 cost and `REGION = "PRG"`: agreed.** Yield points on N64 wait for dirty pages in
  its connector (step 11, which can follow the release). `REGION` becomes the code region
  of the core in use, as part of step 6, since the GBA and N64 already have other names.
- **Byte order: agreed.** The handed-over regions are in the CPU's own byte order, as the
  connector API already says for `data`, and dj converts RDRAM's host-endian words before
  sending. Diff addresses and the RAM fallback use the same order. This also fixes the
  `[WRITE]` bug the ROADMAP lists, if the same conversion is shared.
- **Two handle behaviours: agreed.** In-process `Comparison` should answer `get_proc` from
  the same snapshot of memory and CPU state as the worker does, so tests see exactly what
  dj-gui does. The tests move to that in step 2.
- **`get_proc` and `retro_serialize`: agreed, intended.** That is why `baseline = "copy"`
  exists. `connectors.md` changes in step 9. Before the release, no one outside depends on
  it; the Thwaite and Tetris decomps only read memory.
- **Not covered, agreed:**
  - **`[RESET]` part-way through a step:** treat it as a jump. The comparison core is
    synced again at the next yield point (or booted, if the reset is a power-on).
  - **Impulse inputs on a step that spans two frames:** under the "frame the step started
    in" rule, a one-frame press in the second frame is lost. That's what poll timing
    (below) fixes; until then, list it as a known limit.
  - **A yield point hit too often:** a cap per frame (64, say). Past it, that yield point
    is turned off for the rest of the run, and a warning names it. That's probably a wrong
    address, not a game.
- **Version numbers: agreed.** The new connector calls make connector API 3 (`dj_api_version`).
  The `dj_baseline` struct keeps its own version (2). The patch changes `dj_build_id`, so
  step 4 ends with `dj repin` on Thwaite and the CI's pinned build.
- **Order with the GUI's core in a worker:** yield points first. The GUI's core stays in
  process for the release (decided 2026-11-10). If it moves later, the baseline's worker
  can send regions straight to the comparison worker instead of through the GUI.

### Missed opportunities

- **Ticks as counted yield points: agreed, and worth doing in this plan.** A yield point
  with `check = false, count = true` is a tick. dj keeps a count per named point.
  `[tick] exec = ...` stays in config as shorthand for one counted point named `tick`, so
  existing projects keep working. That gives N64 ticks once its connector stops at yield
  points, and more than one tick location per project.
- **Poll counting: checked, and it doesn't give the read time on Mesen.**
  `LibretroKeyManager::RefreshState` calls `input_poll` and reads every port's
  `input_state` once per frame, not when the game reads `$4016`. So counting libretro
  polls says nothing about lag on Mesen; it does for `.m64` and cores that poll on demand.
  - On Mesen, what's needed is the game's reads of `$4016`/`$4017`, counted by the
    connector's existing `MemoryRead` hook. That's cheap, and it's the "later" under Input.
  - Both are worth having, as different facts: libretro polls (any core, no connector) and
    controller reads (connector).
- **Run to a location as a general breakpoint: agreed.** An MCP `step` option "until code
  at X runs" (and the GUI's later equivalent) on top of `dj_run_to_yield`. After this plan,
  not in it.

### Sub-frame input

- **Agreed that the syntax must be reserved before the release,** since older tools reject
  unknown line forms. Proposal for segment lines: `812.2 P1 A` means frame 812, from the
  game's third read of the controller in that frame (`.0` is the first, and a plain `812`
  stays "the whole frame").
- **Agreed on the key: the poll index.** It doesn't depend on the emulator, it matches
  `.m64` and BizHawk's sub-frame input (`player/src/tas/bk2.rs` drops it with a warning
  today), and it doesn't tie segment content to `config.toml`.
- **On the comparison side, no API change should be needed.** The comparison core reads
  inputs through dj's own `input_state` callback, so dj can answer each read with the
  input for that read's index, counting the comparison core's reads as it counts the
  baseline's. What that needs is that the decomp reads input when the original does, once
  per read, which a faithful decomp does anyway. Freezing the version 2 handle is safe for
  this.
- **The ROADMAP item** should cite TAS_PLAN's `.m64` poll clock and the BizHawk warning, and
  say the syntax is reserved before the release while the feature follows it.

### What changes in the plan, if agreed

- **Step 0 (new, now):** dj accumulates ticks around every coverage clear. This fixes the
  `contents.ticks` bug and is independent of the rest.
- **Step 1:** settle the name, which renames everything above.
- **Step 4:** Mesen checks `_state.PC` before `Exec`; ends with `dj repin`.
- **Steps 5 and 6:** `dj_present`, the cap per yield point, counted yield points and
  `[tick]` as shorthand, `REGION` per core, the config hash in the `parity_runs` key, and
  `dj parity` carrying on from parents.
- **Step 7:** the format reserves `frame.read` input lines (parsed, refused with "not
  supported yet" until sub-frame input exists).
- **New after the plan:** counting controller reads in the Mesen connector, and the MCP
  "step until code at X runs".

**Decided 2026-11-11:** the name is "yield" (Andrew). The rename above and the other
changes wait for his go-ahead.

## Review of the response (2026-11-11 11:12 EST)

Written by the meta-analysis thread, about "Response to the review" above. The response
checked the claims against the code, and most of it can be accepted as written. The points
below are the ones to settle before the plan is edited. Mesen's source wasn't re-read for
this: the Mesen details rest on the response's reading and on how the core is structured.

### To settle first

1. **A counted-only point must not stop or line up.**
   - The response makes a tick "a yield point with `check = false, count = true`". But
     every yield point lines the cores up, and reaching different ones is a failure.
   - Thwaite's tick (`doStateAction`) is inside the game loop, not at a wait, so the decomp
     would have to report reaching it.
   - Counting needs to be a third kind: counted in the connector, never stopped at, never
     matched against the comparison core.
2. **`dj_present` should be a frame-end call, not only a picture.**
   - Many games do real work in the NMI handler every frame, lag or not: a frame counter,
     OAM and palette upload, sound.
   - A decomp that only advances at yield points has nowhere to do that work.
   - One optional call when the baseline's frame ends (`dj_frame_end`, say) covers the
     per-frame work and the picture. It also answers "Logic in the NMI handler" under Open
     questions.
3. **"yield" is a reserved word in Rust.**
   - A field or variable named `yield` needs `r#yield`, and the config table needs a serde
     rename. It is also a keyword in Python, which the project's scripts use.
   - The name is fine for the docs and the C API (`dj_run_to_yield`). In Rust, use
     `yield_point` throughout, and decide that at step 1.
4. **Over the cap, fail the run instead of turning the point off.**
   - Lining up depends on both cores seeing the same sequence of points. Dropping one
     mid-run changes what every later check means.
   - An error that names the point is safer. The response itself says a point hit that
     often is probably a wrong address.
5. **Connector API 3 strands the other two connectors.**
   - dj ignores a connector whose version isn't its own.
   - mGBA and Mupen64Plus get their yield points in step 11, which may follow the release,
     so they would stop working at step 4.
   - Either dj accepts versions 2 and 3, or both connectors get a version-only rebuild at
     step 4.
6. **"The game's third read" needs a definition per system.**
   - On the NES, one controller read is a strobe of `$4016` followed by eight reads, so the
     index should count strobes, not reads.
   - Missing rule: what a `frame.read` line does on a frame with fewer reads than its
     index. A lag frame has none.
   - Mesen needs more than counting. It takes input once per frame, so sub-frame input
     there also needs the connector to ask for input again at the strobe.

### Smaller notes

- **The PC compare before `Exec`** is the right fix, and being read-only it can't disturb
  emulation. Three details:
  - Resuming needs a skip-once flag, so it doesn't stop again at the same PC.
  - The work `retro_run` does before and after the frame loop must run once per frame, not
    once per call.
  - On mappers with movable banks, one ROM offset can sit at several CPU addresses, so the
    set needs all of them.
- **Byte order.** Converting RDRAM for the handover is fine, but keep `[hash]` regions out
  of it. They hash the host-order bytes today, and changing that would invalidate every
  N64 segment's hashes.
- **Step 0** (dj accumulating ticks around every clear) is better than the connector change
  the review proposed. dj must reset its total on every jump, since the connector's count
  carries on across a loaded state.

## Response to the review of the response (2026-11-11 11:46 EST)

Written by the thread that wrote the plan. All six points are agreed; two of them improve
the design. Point 5 was checked against the code; the rest is judgement. Nothing above is
edited yet: these answers, and the agreed parts of the first response, go into the plan
when Andrew says so.

1. **Counted-only points: agreed.** The first response was wrong here. There are three
   kinds of point, and only yields stop the baseline:

   | Kind | Stops the baseline | Cores line up | Checked |
   | --- | --- | --- | --- |
   | Yield (`check = true`, the default) | yes | yes | yes |
   | Yield (`check = false`) | yes | yes | no |
   | Counter | no | no | no; counted in the connector only |

   Config: `[[yield]]` and `[[counter]]`. `[tick] exec = ...` stays as shorthand for one
   counter named `tick`. N64 gets ticks from counters, without stopping.
2. **`dj_frame_end` instead of `dj_present`: agreed.** One optional call when the
   baseline's frame ends covers the per-frame work (what the NMI handler does: frame
   counter, OAM upload, sound) and the picture. It also settles "Logic in the NMI handler"
   under Open questions. **Ordering rule:** dj calls it after any yield that falls in the
   same frame, so the decomp's order matches the original's (logic, then vblank).
3. **`yield` in Rust: agreed.** "Yield" in the docs, the config (`[[yield]]`) and the C API
   (`dj_run_to_yield`); `yield_point` / `YieldPoint` in Rust, with a serde rename for the
   table. Python only sees it as a string key, so nothing changes there.
4. **Over the cap, fail: agreed.** The run fails with an error naming the yield and its
   count for the frame. Turning a point off would quietly change what every later check
   means, and a point hit that often is misconfigured anyway.
5. **Connector API 3: agreed, and checked.** `player/src/libretro.rs` warns and plays
   without coverage unless `dj_api_version()` is exactly 2, so mGBA and Mupen64Plus would
   lose coverage at step 4. **Fix:** dj accepts 2 or 3. A version 2 connector means no
   yields, no counters past `[tick]` and no CPU state, as a missing optional function does
   today. That's better than version-only rebuilds, because it also keeps cores built before
   the change working.
6. **Sub-frame reads: agreed on all three.**
   - **The index is defined per system** as the moment the game latches the controller.
     On the NES that is a strobe of `$4016`, not each of the eight reads after it.
   - **A frame with fewer reads than a line's index** (a lag frame has none): the line
     carries to the next read that happens, and playback warns if it's never used by the
     end of the segment. That's what TAS tools do with lag; dropping it silently would be
     worse.
   - **Mesen needs to fetch input again at each strobe,** not only count. That's an
     emulation change, on only while a segment has sub-frame lines, so existing hashes
     don't change. It comes after the release; before it, only the syntax is reserved.

**Smaller notes: all agreed.**

- **Resuming at a yield** sets a skip-once flag, so the same PC doesn't stop again.
- **`retro_run`'s work before and after the frame loop** runs once per frame; the split into
  begin, run and end gives that.
- **The stop set holds every CPU address** that a yield's ROM offset is mapped at, for
  mappers with movable banks.
- **Byte order:** the conversion applies only to the regions handed to the comparison core,
  diff addresses and the RAM fallback. Never to `[hash]` regions, whose hashes stay over the
  host-order bytes.
- **Step 0:** on every jump, dj resets its tick total and reads the connector's count once
  to set the baseline, since that count carries on across a loaded state.

No further round of review looks needed: these answers are consistent with each other and
with the agreed parts of the first response.

## Second review of the responses (2026-11-11 11:57 EST)

Written by the meta-analysis thread, about "Response to the review of the response" above.
Another round is needed. In agreeing with the six points, that response makes four new
design decisions that haven't been reviewed, and one of them is wrong. By its own account
only point 5 was checked against code. For this review Mesen's source was read in
`build/mesen-src`, so the Mesen facts here are checked, not recalled.

### Asked of the plan's thread: one consolidated API section

**Before anything is merged into the sections above, write the whole API once, in one
section, and have it reviewed as a whole.** The reason for doing this work before the
release is that the API freezes. Today its final shape exists only as amendments spread
over four sections, and some of them conflict:

- The first response changed what return code 1 of the comparison core's run call means.
- `dj_present` became `dj_frame_end`, and this review changes it again (below).
- `count = true` on a yield became `[[counter]]`.
- `check_at = "checkpoints"` has no value under the new name.

The section should hold, in their final form and with final names:

- **The version 2 `dj_baseline` struct,** every field, including where named counters'
  counts go.
- **Every connector function,** baseline side and comparison side, with its signature and
  return codes, and which connector API version it belongs to.
- **Which call runs when.** A table of `retro_run`, `dj_run`, `dj_run_to_yield` and the
  frame-end call, for each mode: plain play, frame-end checking, yield checking, and
  `sync = "state"`.
- **The config keys:** `[[yield]]`, `[[counter]]`, `[tick]`, `check`, `check_at` and its
  values, `baseline`.
- **The reserved segment syntax** for sub-frame input, and what a version 1 tool does when
  it meets it.
- **What `get_proc` answers** in the worker and in process.

### Wrong: the `dj_frame_end` ordering rule

The response says dj calls it "after any yield that falls in the same frame, so the
decomp's order matches the original's (logic, then vblank)". That doesn't hold. The
`dj_frame_end` suggested in "Review of the response" shares the flaw.

- **Where Mesen's frame ends:** at scanline 240 (`PPU::ProcessScanlineFirstCycle`:
  `SendFrame`, `_frameCount++`), and the NMI fires on scanline 241. So within one
  `retro_run` the order is NMI handler, then the wait exits (the yield), then logic.
- **The decomp runs late.** It runs a step only when the baseline reaches the yield that
  ends it. The original ran that step's logic in the previous frame, then the NMI, then
  reached the yield.
- **So the order is reversed.** The decomp gets `dj_frame_end` at the end of the previous
  frame, before it has run the logic the original ran before that NMI.
- **What breaks:** anything the NMI handler consumes from logic, such as a queued VRAM
  update or an OAM buffer. At the check, the original's queue is flushed and the decomp's
  is full.
- **A better shape:** `dj_run_to_yield` is told how many frame ends passed since the last
  yield, and the decomp does its vblank work itself, after its logic. A frame-end call then
  only presents the picture.

### Unreviewed decisions with problems

- **`[[counter]]` has nowhere to report.**
  - The version 2 handle has one `ticks`, `frame_ticks` and `has_ticks`. Named counters
    need per-name counts, and the connector needs calls to set and read several. Neither
    is specified.
  - "N64 gets ticks from counters" isn't free. That connector's tick functions are stubs,
    and it still needs the overlay code check. The "for free" in "Review findings" assumed
    counters and yields shared one mechanism, which point 1 removed.
- **"The line carries to the next read"** makes a line's effect depend on emulation timing,
  not on its frame number. "That's what TAS tools do" is asserted, not checked. Only the
  syntax needs settling before the release, so leave the lag rule undecided.
- **The Mesen mode switch for sub-frame input** is unnecessary. Mesen takes input once per
  frame at one scanline (`PPU.cpp`, `UpdateInputState`). Fetching again at a strobe returns
  the same value when input doesn't change within the frame, so it can be always on.

### Still open after four rounds

- **The rename isn't finished.** `check_at = "checkpoints"` has no new value, and the
  edits to THWAITE_DECOMP_PLAN.md and the ROADMAP aren't listed.
- **Scope grew with no triage.** At least ten items were added (step 0, counters, the
  frame-end call, the version range, the format reservation and others). Only the API and
  format shape must precede the release. Say which of the rest can follow it.
- **No cheap end-to-end test.** Mesen against itself (`sync = "state"`) through yields on
  all 12 Thwaite segments would prove the stop, resume and line-up logic before any decomp
  exists.
- **The input rule is unchanged.** Presses lost on a step that spans two frames are still
  "a known limit", and Thwaite's yield placement depends on that rule.

### What held up

- **Checking the PC before `Exec` is sound.** Mesen takes interrupts at the end of
  `CPU::Exec`, so the PC there is the true next instruction.
- **Mesen reads input once per frame,** as the response said.
- **dj requires connector version exactly 2** (`player/src/libretro.rs`), so accepting 2
  or 3 is right.
- **An opportunity:** Mesen already tracks lag itself (`_isLagging` and `_lagCounter` in
  `ControlManager`). The connector could expose that instead of counting `$4016` reads.

## Consolidated API, for review (2026-11-11 12:03 EST)

Written by the plan's thread, as "Second review of the responses" asks: the whole API in
its final form, in one place. It takes in everything agreed in the sections above, with the
second review's corrections. **Where it differs from any earlier section, this one is
meant.** Once it's accepted, it replaces Part 1, Part 2 and "Configuration" above, and the
review sections stay as the record.

Facts checked for it in `build/mesen-src` and dj's code:

- **Mesen's frame order:** the frame ends at scanline 240 (`SendFrame`, `_frameCount++`
  in `PPU::ProcessScanlineFirstCycle`), and NMI fires at 241, at the start of the next
  `retro_run`.
- **Mesen's input:** taken once per frame, in `ControlManager::UpdateInputState`, which also
  bumps `_lagCounter` and `_pollCounter`. Both are saved in the save state.
- **Mesen's lag flag:** `_isLagging` is set when input is taken and cleared by any read of
  `$4016`/`$4017` (`ControlManager::ReadRAM`).
- **dj's version check:** dj needs connector API exactly 2 today (`player/src/libretro.rs`).
- **Segment format 1** rejects a frame number that isn't an integer (`parse_body_line`).

### Terms

- **Yield:** a place in the original's code where one step of its logic is done and it
  starts to wait (the first instruction of a wait loop, a `VBlankIntrWait` call,
  `osRecvMesg`). Not the top of the loop after the wait. The baseline stops *before*
  executing it.
- **Step:** what the original runs between two yields. The decomp's state machine runs one
  step per call.
- **Frame end:** `retro_run` returning, or `dj_run_to_yield` returning -1.
- **Counter:** a location that's only counted. It never stops anything and is never matched
  between the cores. `[tick]` is the one counter built now.

### Versions

| What | Version | Compatibility |
| --- | --- | --- |
| Connector API (`dj_api_version`) | **3** | dj accepts 2 and 3. A version 2 connector has none of the version 3 functions, as if each optional one were missing |
| `struct dj_baseline` (`version` field) | **2** | Version 2 adds fields at the end; a comparison core written for version 1 reads the first three as before |
| Segment format (`dj-segment N`) | stays **1** | 2 is reserved for sub-frame input lines (below). dj 0.1 already refuses a newer format by name |
| `config.toml` | `format = 1` | New tables and keys are added here, before the release, so no format change |

A connector built with the API 3 patch reports a new `dj_build_id`. Thwaite and CI's
pinned build need `dj repin` when Mesen's patch changes.

### Configuration

```toml
[[yield]]
name = "game"            # unique; shown in tools, failures and the handle
exec = "gameLoopWait"    # a symbol (with +offset) or a location like PRG:0x1A3
check = true             # default; false: line up and sync here, don't check

[tick]
exec = "doStateAction"   # unchanged: the counter named "tick"

[[compare]]
name = "decomp"
check_at = "yields"      # or "frames"
baseline = "memory"      # or "copy"
```

- **`[[yield]]`:** project-wide, since yields are facts about the original. A name or
  location given twice is an error. Keys reserved for later and refused now with "not
  supported yet", so their meaning can't drift: `cpu` (SNES coprocessors) and `code` (the
  overlay check for code loaded into RAM).
- **`[[counter]]`:** reserved. dj 0.1 refuses it with "not supported yet". Counters don't
  change playback, so adding them later needs no format change.
- **`check_at`:** `"yields"` checks at checked yields, `"frames"` at frame ends. The
  default is `"yields"` when the project has yields and the comparison core has
  `dj_advance_to_yield`, else `"frames"`. `"yields"` without either is an error naming
  what's missing. "Both" doesn't exist.
- **`baseline`:** `"memory"` (the default) hands the comparison core the baseline's memory
  and CPU state. `"copy"` keeps today's full copy of the baseline core, with its save state
  loaded, so `get_proc` answers any function. `"copy"` works only with
  `check_at = "frames"`; with `"yields"` it's an error.

### The handle: `struct dj_baseline` version 2

```c
struct dj_counter {
    const char *name;
    uint64_t    total;        // since the last sync
    uint64_t    this_frame;   // in the frame running now
};

struct dj_baseline {
    uint32_t version;         // 2
    void    *ctx;
    void    *(*get_proc)(void *ctx, const char *name);
    // version 2:
    uint64_t    frame;        // the power-on frame running now (a yield) or just run (a frame end)
    const char *yield;        // the yield the baseline is stopped at; NULL at a frame end or frame start
    uint32_t    yield_index;  // which yield of this frame (0, 1, ...)
    uint32_t    frame_ends;   // frame ends since the previous yield (see below)
    uint32_t    counter_count;
    const struct dj_counter *counters;  // counter_count entries; "tick" first when [tick] is set
};
```

- **Valid** only for the length of the call it's passed to, like the version 1 handle.
- **`frame_ends`:** how many times the baseline's frame ended between the previous yield and
  this one. Normally 1: the frame end (and NMI) that ended the wait. More than 1 means the
  step ran past vblank (lag), and each extra frame end fell during the step's logic. The
  decomp does its own vblank work from this: the first before the step's logic, any others
  as a lagging original would (usually the guarded part of the NMI only). This replaces
  `dj_frame_end`. At a sync it's 0.
- **Counters:** only `tick` exists until `[[counter]]` is built, and `counter_count` is 0
  when the project has no `[tick]`. dj counts them from the connector's count, accumulated
  around every `dj_clear`. It resets them at every sync, since the connector's count isn't
  part of a save state.
- **Read `counters`, not `dj_get_tick_count` through `get_proc`,** which isn't answered.

### Baseline-side connector functions

Unchanged from API 2: `dj_api_version` (now 3), `dj_region_count`, `dj_get_region`,
`dj_clear`, `dj_set_tick_exec`, `dj_get_tick_count`, the trace functions, `dj_forced_options`,
`dj_build_id`. dj stops relying on the tick count surviving a clear (it accumulates).

New in API 3, all optional:

```c
// Where the CPU is. cpu 0 is the main CPU (the only one until [[yield]] cpu exists).
// False when that CPU doesn't exist.
struct dj_cpu_state {
    uint64_t pc;              // the next instruction's address
    int32_t  region;          // where pc maps, as in dj_get_region, or -1
    int64_t  offset;          // offset in that region, or -1
    uint64_t cycle;
    int32_t  scanline;        // -1 where the system has none
    uint32_t reg_count;
    uint64_t regs[40];        // in the order connectors.md gives per cpu ("6502": A X Y SP P)
};
bool dj_get_cpu_state(uint32_t cpu, struct dj_cpu_state *out);

// The yields: n locations as in dj_set_tick_exec, each on a CPU (0 for now). Replaces any
// earlier set; n = 0 turns them off. The connector stops at every CPU address the
// location is mapped at (a bank mapped twice counts twice).
void dj_set_yields(const int32_t *regions, const int64_t *offsets, const uint32_t *cpus, size_t n);

// Run until the next instruction is at a yield, or the frame ends. Returns the yield's
// index, or -1 when the frame ended (video and audio sent, as retro_run). Continues a frame
// in progress, and doesn't stop again at the yield it last stopped at before executing it.
// retro_run still runs a whole frame and ignores yields.
int32_t dj_run_to_yield(void);

// Whether the game has read the controller in the frame running now (Mesen: not
// _isLagging). Reset when the frame's input is taken.
bool dj_frame_read_input(void);
```

Reserved names (API 4 or later, not built): `dj_set_counters`, `dj_get_counts`.

**Mesen:** `RunSingleFrame` splits into begin (the frame count and model update), run (the
`Exec` loop, stopping when `_state.PC` is in the yield set and the skip-once flag is clear)
and end (`EndFrame` and the rest), so a frame's own work runs once whatever the number of
calls. The PC compare is before `Exec`, which is where Mesen has already taken interrupts,
so the PC is the true next instruction.

### Comparison-side connector functions

Unchanged: `dj_sync`, `dj_write`, `dj_check_parity`, `dj_parity_diffs`, `dj_run`. Each one
that takes the handle gets version 2. `dj_sync` keeps its `frame` and `flags` arguments and
finds the yield in the handle.

New, optional:

```c
// Run one step: to the next yield, given what the baseline did (handle: yield, frame_ends,
// counters). 0: reached a yield, its [[yield]] name written to `reached`. 1: reached no
// yield (a state the decomp has none for): dj counts the step as unchecked and syncs
// again at the next yield. Anything else: an error, with `reason`.
int dj_advance_to_yield(const struct dj_baseline *baseline, char *reached, size_t reached_len,
                        char *reason, size_t reason_len);

// Send the current picture through the video callback without advancing anything. dj
// calls it at each of the baseline's frame ends under check_at = "yields". Without it the
// comparison core's picture isn't shown in that mode.
void dj_present(void);

// The regions the comparison core reads through the handle, by connector or libretro
// name, one per line ("RAM\nSRAM\n"). Without it: system RAM.
const char *dj_wants_regions(void);
```

**Reaching a different yield than the baseline** is a failure: "the decomp reached
`titleWait`, the original `pauseWait`". **A yield hit more than 64 times in one frame** on
the baseline ends the run with an error naming it and the count; it's misconfigured.

### Which call runs when

| | Plain play | `check_at = "frames"` | `check_at = "yields"` | `sync = "state"` with yields |
| --- | --- | --- | --- | --- |
| **Baseline, per frame** | `retro_run` | `retro_run` | `dj_run_to_yield` until -1 | `dj_run_to_yield` until -1 |
| **Comparison, per frame** | none | `dj_run` if present, else `retro_run` | at each yield: `dj_advance_to_yield`; at the frame end: `dj_present` | its own `dj_run_to_yield`, to the same yield index |
| **Check** | none | after the frame: `dj_check_parity` (or the RAM compare) | at each yield with `check = true` | at each checked yield |
| **Sync** | none | at a frame start, after a jump | at the first yield after a jump | at the first yield, by save state where the emulator's mid-frame states are exact (Mesen, mGBA), else at the frame start |
| **`[WRITE]`** | baseline, at the frame start | both, at the frame start (`dj_write`) | both, at the frame start, wherever the comparison core is | as frames |
| **Coverage taken** | segment end (GUI: each refresh) | each frame | each yield and frame end | each yield and frame end |

`[RESET]` is a jump in every mode: the comparison core syncs again at the next yield (or
frame start). `DJ_SYNC_BOOT` stays at frame 0 of a root segment in every mode.

### Inputs to the comparison core under `"yields"`

dj sets them before each `dj_advance_to_yield`. They're the inputs of the frame in which
the baseline read the controller during this step: the first frame since the previous
yield where `dj_frame_read_input` was true. If no frame read it (the connector lacks the
function, or the step read nothing), it's the first frame that began after the previous
yield. A step that read in two frames gets the first one's; that's a known limit, listed in
the docs.

### What `get_proc` answers

| Function | `baseline = "memory"` (worker and in process alike) | `baseline = "copy"` |
| --- | --- | --- |
| `retro_get_memory_data`, `retro_get_memory_size` | from the snapshot sent with the call | the copy's |
| `dj_region_count`, `dj_get_region` | from the snapshot (`data` filled for the regions sent; `flags` NULL) | the copy's |
| `dj_get_cpu_state` | from the snapshot (main CPU) | the copy's |
| anything else (`retro_serialize`, `dj_get_tick_count`, ...) | NULL | the copy's |

- **The snapshot:** the regions in `dj_wants_regions` (or system RAM) and the CPU state,
  taken from the real baseline at the moment of the call.
  - Regions over 64 KB go as the pages changed since the last call.
  - Bytes are in the CPU's own order, which means converting N64 RDRAM's host-endian words.
    The same conversion applies to diff addresses and the RAM fallback, and never to
    `[hash]` regions, which keep hashing host-order bytes.
- **In-process `Comparison`** (the tests) answers from the same kind of snapshot, so tests
  see what dj-gui and `dj parity` see.

### Reserved segment syntax

```
812.2 P1 JOYPAD A 1
```

- **Meaning:** frame 812, from the game's third controller read in that frame (`.0` is the
  first). A plain `812` stays "the whole frame".
- **A controller read is defined per system:** the moment the game latches the controller.
  On the NES that's a strobe of `$4016`, not each of the eight reads that follow.
- **Format:** only in segment format 2. dj 0.1 refuses a format 2 file as it does any newer
  format. In a format 1 file, `812.2` is refused with "sub-frame input lines need segment
  format 2" (today it's a parse error; the message is the only change).
- **Undecided until the feature is built:** what a line does when the frame has fewer reads
  than its index (a lag frame has none), and how Mesen fetches input again at a strobe.
  `UpdateInputState` can't simply be called again, since it bumps `_lagCounter` and
  `_pollCounter`, which are in the save state.

### Results (not frozen, listed for completeness)

- `parity_failures` gains `yield` and `yield_index` (NULL for frame-end failures), with a
  migration on open.
- The `parity_runs` key gains a hash of the `[[yield]]` table and the entry's `check_at`.
- `dj parity` carries on from the parent in tree order (`Checker::enter`). It syncs at the
  first yield only for a segment whose parent wasn't run in this pass.

### Renames and edits this implies

- **This plan:** "check point" becomes "yield" throughout once this section is accepted.
  `check_at = "checkpoints"` becomes `"yields"`; `dj_set_checkpoints`, `dj_run_to_checkpoint`
  and `dj_run_to_checkpoint_named` become `dj_set_yields`, `dj_run_to_yield` and
  `dj_advance_to_yield`; `dj_frame_end` is dropped for `frame_ends` and `dj_present`.
- **Rust:** `yield_point` / `YieldPoint`, with a serde rename for `[[yield]]`.
- **THWAITE_DECOMP_PLAN.md:**
  - the lag rule reads `frame_ends` (and the `tick` counter), not `dj_get_tick_count`;
  - `dj_sync` uses the handle's `yield`, and stack matching becomes a fallback for frame
    starts;
  - the frame-end section describes yields at each wait loop's start and the vblank work
    driven by `frame_ends`.
- **ROADMAP:**
  - the release row's wording;
  - the open question "every frame or only at ticks" answered;
  - the sub-frame input entry cites TAS_PLAN's `.m64` poll clock and the BizHawk warning
    in `player/src/tas/bk2.rs`, and says the syntax is reserved before the release.

### Before the release, and after

| Before | After |
| --- | --- |
| Step 0: dj accumulates ticks around every clear (fixes `contents.ticks`); reset at every jump | `[[counter]]`, `dj_set_counters`, `dj_get_counts` |
| This API and the reserved syntax, frozen | Yields in mGBA and Mupen64Plus (with N64 ticks and the overlay check) |
| dj accepts connector API 2 and 3 | Sub-frame input itself |
| Handle version 2, the memory snapshot, `get_proc` as above, `dj_get_cpu_state` | The MCP "step until code at X runs" |
| Mesen: `dj_get_cpu_state`, `dj_set_yields`, `dj_run_to_yield`, `dj_frame_read_input`; the determinism test; `dj repin` | |
| **Mesen against Mesen** (`sync = "state"`) through yields on all 12 Thwaite segments: proves stopping, resuming and lining up before any decomp exists | |
| The test core: yields, a lag mode spanning frames, `dj_advance_to_yield`, `dj_present` | |
| dj: checking and syncing at yields, the results changes, the GUI, the docs | |

## Critique of the consolidated API (2026-11-11 12:55 EST)

Written by the meta-analysis thread, about "Consolidated API, for review" above. That
section is the best statement of the design so far, and it fixes the vblank ordering
properly. It isn't ready to freeze: two rules are wrong as written, four things are
unspecified, and a core definition changed without being flagged. The Mesen and Thwaite
facts here were checked in `build/mesen-src` and `examples/thwaite/build`.

### Wrong as written

- **The input rule picks the wrong frame on every normal step.**
  - The rule: the first frame since the previous yield where `dj_frame_read_input` was
    true.
  - That function is a per-frame flag. Mesen clears `_isLagging` on any `$4016` read
    (`ControlManager::ReadRAM`) and only sets it again when the next frame's input is
    taken.
  - With the yield at the wait's start, the game has already read the controller earlier
    in that same frame. So the frame containing the previous yield reports "read", and the
    decomp gets inputs one frame early.
  - The fix is a read count sampled at each yield and frame end, so dj sees reads after
    the yield. The same count is the sub-frame read index.
- **The `check_at` default contradicts the Mesen-against-Mesen test.**
  - `"yields"` requires the comparison core to have `dj_advance_to_yield`, and is an error
    without it.
  - An emulator used with `sync = "state"` has `dj_run_to_yield`, not
    `dj_advance_to_yield`. So the fourth column of "Which call runs when" and the release
    test can't be configured.
  - Nothing says dj calls `dj_set_yields` on the comparison core in that mode either.

### Unspecified

- **Stop state lives outside the save state.** The "frame in progress" and skip-once flags
  are connector globals, and Mesen's `retro_unserialize` doesn't touch them. Four cases
  need rules:
  - A failure stops playback with the baseline mid-frame, then the user seeks.
  - `retro_run`, `retro_serialize` or a state hash is asked for while a frame is in
    progress.
  - A `sync = "state"` core loads a state stopped at a yield, and would stop there again
    at once.
  - `[RESET]`.
- **A spin loop that branches to its own first instruction.** "The first instruction of a
  wait loop" works for Thwaite, where `lda nmis` sits outside the loop (`main.s`). For
  `loop: lda flag / beq loop` the yield is hit every iteration, and skip-once only covers
  the first. That trips the cap of 64 on a correct config.
- **Coverage between a yield and the frame end.** Flags are taken at both. The ones taken
  at a frame end belong to the next yield's check, and nothing says they're carried over.
- **`"RAM"` is ambiguous in `dj_wants_regions`.** It's both a libretro memory name
  (`region_memory_id`) and a Mesen connector region, and the connector's `RAM` has no
  `data`.

### Risks in freezing it

- **Frozen after one emulator.** mGBA and Mupen64Plus yields come after the release, so the
  API freezes having been built only on the simplest core. "Other consoles" calls mGBA
  easy; a prototype there before the freeze would test the API.
- **`dj_set_yields` takes parallel arrays.** Any new condition needs a new function. The
  N64 will need one: `osRecvMesg` is called by every thread, so a yield there needs a
  thread filter, which `cpu` and `code` don't cover.
- **The cap of 64 is fixed.** An N64 game could exceed it legitimately.
- **Lag is still approximate.** With `frame_ends` above 1, the decomp can't know where in
  its logic the NMI fell. List it as a limit, instead of "as a lagging original would".

### Process

- **The yield moved without notice.** It went from just past the wait's exit (the reasoning
  in THWAITE_DECOMP_PLAN.md) to the wait's first instruction. That's the right call, since
  it's what fixes the ordering, but it's the largest change in the section and isn't
  flagged.
- **No list of differences.** "Where it differs from any earlier section, this one is
  meant" needs one. Also new and unflagged: `dj_advance_to_yield` takes the handle,
  `dj_frame_read_input` exists, and counters are deferred.

### What held up

- `frame_ends` in the handle, with the yield at the wait's start, gives the right order of
  logic and vblank work.
- Sub-frame input in segment format 2 is cleaner than reserving syntax in format 1.
- `get_proc` answering the same way in the worker and in process.
- The before-and-after table, with Mesen against Mesen as a release test.
- dj warns about unknown top-level config keys and ignores them (`check_versions`,
  `player/src/collection.rs`), so adding `[[yield]]` needs no format change.

## Proposed API, first pass (2026-11-11 12:55 EST)

Written by the meta-analysis thread, as a first pass at an API that closes the gaps in the
critique above. It starts from "Consolidated API, for review" and keeps what held up
there. Where the two differ, the differences are listed at the end. Nothing here has been
built or run. The Mesen notes say what the source suggests, and the determinism test
decides.

### Six rules the API follows

Each one removes a class of problem instead of handling its cases.

1. **The baseline is mid-frame only inside dj's own step loop.** Before dj saves a state,
   hashes, seeks, stops playback or calls `retro_run`, it finishes the frame: it calls
   `dj_run_to_yield` until the frame ends, ignoring the yields. This holds after a parity
   failure at a yield too.
   - *Why:* snapshots, hashes, seeking and the GUI keep meaning what they mean today. The
     four "stop state" cases in the critique can't happen.
2. **No save state is ever taken or loaded mid-frame.** `sync = "state"` syncs at a frame
   start, then both emulators run to yields in step. Only `dj_sync` cores sync at a yield,
   and they do it from memory.
   - *Why:* the same emulator, started from the same frame-start state, reaches the same
     yields. Mid-frame states were only needed to sync it at a yield, and they aren't
     exact on Mupen64Plus or bsnes.
3. **The connector keeps no stop state between calls.** `dj_run_to_yield` always runs at
   least one instruction before it can stop, so no "skip once" flag exists.
   - *Why:* nothing outside the save state decides where the next stop is.
4. **Counts, not flags.** Input reads and point hits are ever-increasing counts. dj samples
   them and works with the differences.
   - *Why:* a per-frame flag can't say whether a read came before or after a yield in the
     same frame. A count can, and nothing has to reset it at the right moment.
5. **One list of points, as structs with a size.** Yields and counters are the same thing
   to a connector: a location it watches. Each entry starts with its own size.
   - *Why:* a new condition (an N64 thread, an overlay check) is a new field, not a new
     function, and counters need no later API version.
6. **Every name says whose it is.** A region name is the connector's when the connector has
   a region of that name with `data`. Otherwise it is a libretro memory name.

### Configuration

```toml
[[yield]]
name = "game"            # unique; shown in tools, failures and the handle
exec = "gameLoopWait"    # a symbol (with +offset) or a location like PRG:0x1A3
check = true             # default; false: line up and sync here, don't check
rearm = "always"         # default; or "interrupt" (below)
max_per_frame = 64       # default; more than this in one frame is an error

[tick]
exec = "doStateAction"   # unchanged: the counter named "tick"

[[compare]]
name = "decomp"
check_at = "yields"      # or "frames"
baseline = "memory"      # or "copy"
```

- **`rearm`:** when a yield may stop the baseline again after stopping it once.
  - `"always"`: every time it's reached. Right when the yield's instruction runs once per
    wait, as in Thwaite, where `lda nmis` sits before the loop.
  - `"interrupt"`: not again until the CPU has taken an interrupt. Right for a spin loop
    that branches back to its own first instruction (`loop: lda flag / beq loop`), where
    only an interrupt ends the wait.
- **`max_per_frame`:** per yield, so an N64 project can raise it for one busy yield without
  losing the check on the others.
- **`check_at` default:** `"yields"` when the project has yields and the comparison core
  can follow them, else `"frames"`. A core can follow them when it has
  `dj_advance_to_yield`, or when its entry has `sync = "state"` and it has
  `dj_run_to_yield`.
- **`baseline = "copy"`** works only with `check_at = "frames"`, as before.
- **`[[counter]]`** stays reserved in config. The connector API below already carries
  counters, so building it later changes dj only.

### Baseline-side connector functions (API 3)

Unchanged from API 2: `dj_api_version` (now 3), `dj_region_count`, `dj_get_region`,
`dj_clear`, `dj_set_tick_exec`, `dj_get_tick_count`, the trace functions,
`dj_forced_options`, `dj_build_id`. dj accepts versions 2 and 3.

New in API 3, all optional:

```c
#define DJ_POINT_YIELD     0   // stops dj_run_to_yield
#define DJ_POINT_COUNTER   1   // only counted
#define DJ_REARM_ALWAYS    0
#define DJ_REARM_INTERRUPT 1

struct dj_point {
    uint32_t size;      // sizeof(struct dj_point) as the caller built it
    uint32_t kind;      // DJ_POINT_*
    int32_t  region;    // as in dj_get_region
    uint32_t cpu;       // 0: the main CPU
    int64_t  offset;    // in that region
    uint32_t rearm;     // DJ_REARM_*, for yields
    uint32_t reserved;  // 0
};

// The points to watch. Replaces any earlier list, zeroes every count and arms every
// yield; n = 0 turns them off. False, with nothing set, when the connector can't honour
// one (a kind, cpu, region or non-zero field it doesn't know).
bool dj_set_points(const struct dj_point *points, size_t n);

// How many times each point's instruction has run since dj_set_points. Not changed by
// dj_clear. Fills up to n counts.
void dj_get_counts(uint64_t *counts, size_t n);

// Runs at least one instruction, then on until the next instruction is at an armed
// yield, or the frame ends. Returns the yield's index in the list, or -1 when the frame
// ended (video and audio sent, as retro_run). retro_run ignores points and runs a whole
// frame.
int32_t dj_run_to_yield(void);

// How many times the game has latched the controller since the game was loaded or a
// state was last loaded. Per system: on the NES, a strobe of $4016.
uint64_t dj_input_reads(void);

struct dj_cpu_state {
    uint32_t size;      // sizeof(struct dj_cpu_state) as the caller built it
    uint32_t reg_count;
    uint64_t pc;        // the next instruction's address
    int32_t  region;    // where pc maps, as in dj_get_region, or -1
    int32_t  scanline;  // -1 where the system has none
    int64_t  offset;    // offset in that region, or -1
    uint64_t cycle;
    uint64_t regs[40];  // in the order connectors.md gives per cpu ("6502": A X Y SP P)
};
bool dj_get_cpu_state(uint32_t cpu, struct dj_cpu_state *out);
```

- **Why `dj_set_points` returns a bool.** A connector that doesn't know a field refuses the
  whole list, and dj says which yield it couldn't set. The alternative is a yield that
  silently never stops.
- **Why counts survive `dj_clear`.** It fixes the tick bug at its source for API 3
  connectors. dj still accumulates around clears for API 2's `dj_get_tick_count`.
- **Arming after a load.** dj calls `dj_set_points` again after every `retro_unserialize`
  and `retro_reset`, which arms every yield. So the connector needs no hook in those
  functions, and dj starts its count differences again from zero.
- **`dj_input_reads` works without yields.** dj samples it at every frame end in every
  mode, which gives lag frames for the GUI's ruler and for TAS_PLAN step 4.
- **Mesen.** Each call notes the PPU's frame count on entry and returns -1 when it
  changes, then does the frame's closing work (`EndFrame` and what follows in
  `RunSingleFrame`). The yield test is on `_state.PC` before `Exec`, after the first
  instruction of the call. To confirm with the determinism test: that the frame's opening
  work (`UpdateNesModel`) can run on every call.

### The handle: `struct dj_baseline` version 2

As in "Consolidated API, for review", unchanged: `frame`, `yield`, `yield_index`,
`frame_ends`, `counter_count` and `counters`. Two additions to what it says:

- **`counters` come from `dj_get_counts`** on an API 3 connector, and from the accumulated
  tick count on an API 2 one.
- **Lag is a stated limit.** With `frame_ends` above 1, the handle can't say where in the
  step's logic the extra frame ends fell. A decomp can't reproduce an NMI handler whose
  work the rest of that step then reads.

### Comparison-side connector functions

`dj_sync`, `dj_write`, `dj_check_parity`, `dj_parity_diffs`, `dj_run`,
`dj_advance_to_yield` and `dj_present` are as in "Consolidated API, for review". One
change:

```c
// The regions the comparison core reads through the handle, one name per line
// ("RAM\nSRAM\n"). Without it: libretro system RAM.
const char *dj_wants_regions(void);
```

- **Resolving a name (rule 6).** dj takes the connector's region of that name when it has
  `data`, and the comparison core reads it with `dj_get_region`. Otherwise the name is a
  libretro memory (`RAM`, `SRAM`, `VRAM`, `RTC`), read with `retro_get_memory_data`.
  With Mesen, `RAM` is the libretro system RAM, since the connector's `RAM` has no `data`.
- **A name neither side has** fails the load, naming it.

### Which call runs when

| | Plain play | `check_at = "frames"` | `"yields"`, a `dj_sync` core | `"yields"`, `sync = "state"` |
| --- | --- | --- | --- | --- |
| **Baseline, per frame** | `retro_run` | `retro_run` | `dj_run_to_yield` until -1 | `dj_run_to_yield` until -1 |
| **Comparison** | none | `dj_run` if present, else `retro_run` | at each yield: `dj_advance_to_yield`; at the frame end: `dj_present` | `dj_run_to_yield`, each time the baseline's returns |
| **Lined up when** | | | the names match | both return the same index, or both -1 |
| **Check** | none | after the frame | at each yield with `check = true` | at each yield with `check = true` |
| **Sync, after a jump** | none | at the frame start | at the first yield | at the frame start (rule 2) |
| **Points set on** | the baseline, if `[tick]` | the baseline, if `[tick]` | the baseline | both cores, the same list |
| **After a failure** | | stop | finish the baseline's frame, then stop (rule 1) | the same |

- **`[WRITE]`** goes to both at the frame start, as today.
- **`[RESET]`** is a jump in every mode.

### Inputs to the comparison core under `"yields"`

dj samples `dj_input_reads` at every yield and frame end. Before each
`dj_advance_to_yield` it sets the comparison core's inputs to those of one frame:

1. **The frame in which the count first rose after the previous yield.** That is the frame
   whose input the game read during this step.
2. **If it didn't rise** (the step read nothing), the frame the step ended in.
3. **Without `dj_input_reads`,** the first frame that began after the previous yield.

- **Why this is right for Mesen.** Mesen takes libretro input once per frame, and a read
  later in that frame sees that frame's input. The count rising in frame N means the game
  saw frame N's lines.
- **Limit:** a step that reads in two frames gets the first one's inputs.
- **Sub-frame input later** uses the same count: `812.2` is the read where the count is 2
  above its value at the start of frame 812.

### Coverage and results

- **Flags are taken at every yield and every frame end, and carried.** What's taken at a
  frame end is OR'd into the set for the next yield. A check at a yield owns everything
  the baseline ran since the previous yield.
- **A frame end with no yield after it** in the segment (its last frames) leaves flags
  that belong to no check. They count as not checked.
- **A failure records** the frame and the yield's index in it. A re-run plays to that
  frame, then to that index.
- The `parity_runs` key, the `parity_failures` columns and `dj parity` carrying on from
  parents are as in "Consolidated API, for review".

### Where this differs from "Consolidated API, for review"

| There | Here | Why |
| --- | --- | --- |
| `dj_set_yields` with three arrays | `dj_set_points` with structs, kinds and sizes | Extends without a new function; counters need no API 4 |
| `dj_frame_read_input`, a per-frame flag | `dj_input_reads`, a count | The flag gives inputs one frame early |
| A skip-once flag in the connector | "Runs at least one instruction" | No state outside the save state |
| `sync = "state"` syncs at a yield by mid-frame save state | It syncs at the frame start | Exact on every emulator; no mid-frame states |
| The baseline stays mid-frame after a failure | dj finishes the frame first | Seeking, saving and hashing stay on frame boundaries |
| `check_at = "yields"` needs `dj_advance_to_yield` | Or `sync = "state"` with `dj_run_to_yield` | Mesen against Mesen can be configured |
| A fixed cap of 64 | `max_per_frame` per yield | An N64 yield may exceed it legitimately |
| No rule for spin loops | `rearm = "interrupt"` | A correct config no longer trips the cap |
| `"RAM"` ambiguous in `dj_wants_regions` | Rule 6 | One answer per name |
| Coverage at a frame end unassigned | Carried to the next yield | Each check owns the code since the last one |
| `dj_cpu_state` without a size | With one | Registers or fields can be added |

### Open in this proposal

- **`rearm = "interrupt"` on a second emulator.** "The CPU has taken an interrupt" is clear
  on the 6502. On the GBA the wait is usually `VBlankIntrWait`, which needs no rearm. On
  the N64 the wait is a thread blocking, where "interrupt" may be the wrong word. Try it in
  mGBA before the freeze.
- **`max_per_frame` across lag.** A count per frame assumes frames are the unit. For a
  yield reached many times in a long step, a count per step may fit better.
- **Whether `dj_set_points` after every load costs anything** on a connector with large
  regions. It shouldn't, since it sets a short list.
- **Finishing the frame after a failure** means the baseline's picture and RAM views show
  the frame's end, not the yield. The failure's diffs are taken at the yield, so nothing is
  lost, but the GUI should say which moment it shows.

## Response to the critique and the proposed API (2026-11-11 16:48 EST)

Written by the plan's thread, about "Critique of the consolidated API" and "Proposed API,
first pass". The critique is right on every point, and the proposal improves on
"Consolidated API, for review" in most of its eleven rows: counts instead of flags,
extensible structs, finishing the frame after a failure, no mid-frame save states. It has
four faults and some gaps. Mesen's input timing was checked in `build/mesen-src`; the rest
is from reading the proposal.

### Faults

1. **`rearm = "interrupt"` breaks rule 3.** Whether a yield is armed is stop state outside
   the save state, and it changes behaviour after a load:
   - **In continuous play:** a spin loop's yield is disarmed when the wait starts, the
     frame ends (scanline 240) while it spins, and the NMI rearms it.
   - **After a seek to that frame start:** `dj_set_points` arms every yield, so the baseline
     stops in the spin loop at once, before the NMI.
   - **The result:** the yields seen after a jump differ from those in continuous play, and
     failures are keyed by which yield of the frame.
   - **Fix: a yield is entry into a range.** `exec` plus `size` (the wait loop's bytes). It
     fires when the PC enters the range from outside. The connector needs only the
     previous instruction's PC, which it sets from the current PC after a load or
     `dj_set_points`. That is derived from the save state, so rule 3 holds.
   - A one-instruction range is the old behaviour. Spin loops need no special case, and a
     correct config can't trip the per-frame cap. `rearm` goes.
2. **Lining up under `sync = "state"` is too strict.** "Both return the same index, or both
   -1" makes the cores cross frame ends together. A comparison emulator with other settings
   (overclocking, say) won't lag where the baseline does, so it reaches a yield while the
   baseline reaches a frame end, and that reads as out of line though nothing is wrong.
   - **Fix:** use the decomp's model. The comparison emulator runs until it returns a yield,
     crossing frame ends as needed, and only the sequence of yields must match.
   - **Inputs:** dj sets the comparison emulator's inputs at each of its own frame starts.
3. **Structs with a size in each entry have no defined stride.** An array whose entries may
   differ in size can't be walked. Either every entry has the same `size`, which is the
   stride, or the stride is an argument:
   `dj_set_points(const void *points, size_t stride, size_t n)`. The second is clearer.
4. **Two indexes share a name.**
   - `dj_run_to_yield` returns a position in the points list, which includes counters.
   - The handle's `yield_index` and the failure records mean "which yield of this frame".
   - Name them apart (`point` and `nth_this_frame`, say).

### Gaps

- **The input rule holds on Mesen, with one stated limit (checked).** Mesen takes input at
  scanline 241 (`EmulationSettings::_inputPollScanline = 241`). The frame ends at 240, so
  input is taken just inside the new `retro_run`, before the NMI, and the count rising in
  frame N means frame N's inputs were seen. The exception: a read during the rest of
  scanline 240 (about 113 CPU cycles) sees the previous frame's input. The game is normally
  waiting then; list it as a limit.
- **`--explain` under rule 1** takes its trace at the yield, before dj finishes the frame,
  or the trace includes code that ran after the failure.
- **Ticks on an API 3 connector** are set through `dj_set_points` as a counter, not
  `dj_set_tick_exec`, so there aren't two mechanisms. `dj_set_tick_exec` stays for API 2
  connectors.
- **Counting `$4016` strobes needs the write hook.** A strobe is a write, and Mesen's
  `ControlManager::WriteRAM` only forwards it to the devices. The connector's existing
  hook in `CPU::MemoryWrite` counts the 1→0 write. Cheap, and worth saying, since the
  proposal only says "on the NES, a strobe".
- **The N64 thread filter isn't in the struct.** Adding fields later works, since an older
  connector refuses an entry it doesn't know. Reserve the field's meaning now anyway, since
  the critique raised it.
- **`max_per_frame`:** with entry into a range, the cap only catches misconfigured yields.
  Per yield is fine; the "per step" question can be dropped.

### The freeze is smaller than both threads assume

- **The baseline-side API has one kind of user:** dj's own connectors (`connectors/mesen`,
  `mgba`, `mupen64plus`), which ship in this repo and are patched along with dj.
- **What people outside write against:**
  - the comparison side (`dj_sync`, `dj_advance_to_yield`, the handle, `dj_wants_regions`);
  - the config (`[[yield]]`, `[[compare]]`);
  - the segment format.
- **So only those freeze for the release.** The baseline-side calls (`dj_set_points`,
  `dj_run_to_yield`, `dj_input_reads`, `dj_get_cpu_state`) stay versioned with dj and can
  change once mGBA and Mupen64Plus are tried. That answers "Frozen after one emulator"
  without an mGBA prototype before the release. The range semantics and N64 threads get
  settled when those connectors are built.
- **The docs say so:** `connectors.md` marks the baseline-side functions as dj-internal and
  subject to change. The comparison-side ones are the stable API.

### Where this lands

Accept "Proposed API, first pass" with these changes:

- entry into a range in place of `rearm`;
- lining up by yields only under `sync = "state"`;
- a stride argument to `dj_set_points`;
- separate names for the list position and the yield's number in its frame;
- ticks through `dj_set_points` on API 3;
- the scanline 240 input limit stated;
- `--explain` tracing to the yield;
- the stable comparison side and the dj-internal baseline side.

One short round of review on these, then the plan's sections above are rewritten from the
result.

## Review of the response to the proposed API (2026-11-11 16:51 EST)

Written by the meta-analysis thread, about "Response to the critique and the proposed API"
above. That response is the strongest so far. Its four faults in "Proposed API, first
pass" are all fair, and `rearm = "interrupt"` breaking that proposal's own rule 3 is the
best catch in this exchange. Two of its fixes have problems, and its argument that less
needs freezing contradicts itself. Its Mesen claims were checked in `build/mesen-src` and
hold: input is taken at scanline 241 (`_inputPollScanline`), and
`ControlManager::WriteRAM` only forwards to the devices.

### Fixes that don't hold

- **"Entry into a range" fires on every interrupt during the wait.**
  - The NMI handler runs outside the range, and its return lands back inside. That is an
    entry from outside, so the yield fires again.
  - The decomp is then asked to run a whole step while the original is still about to
    finish its wait. That happens every frame, and more often with IRQs.
  - It needs an exception for returning from an interrupt, which brings back per-CPU
    meaning: RTI on the 6502, a BIOS return on the GBA, a thread switch on the N64.
  - **A simpler alternative with no state:** keep one-instruction yields, and let `exec` be
    a list of locations. The yield goes on an instruction that runs once per wait, as
    Thwaite's `lda nmis` already does. Where several paths jump into one wait, the list
    names each. Recommended for the release; ranges can come later if a game needs them.
- **Lining up "by yields only" under `sync = "state"` leaves inputs undefined.**
  - If the comparison emulator crosses frame ends on its own, its frame count drifts from
    the baseline's. "Inputs at each of its own frame starts" doesn't say which frame's
    lines, and segment lines are keyed by the baseline's frames.
  - It also weakens the release test. Strict lockstep is what proves that stopping and
    resuming don't disturb emulation. The loose rule would hide a frame-boundary bug.
  - **Recommended:** keep lockstep for the release, and send emulators with different
    timing to `check_at = "frames"`.

### The freeze argument contradicts itself

- **`dj_get_cpu_state` is called dj-internal, but decomps call it.** It is answered through
  `get_proc`, along with `dj_get_region`. So `struct dj_cpu_state` and `struct dj_region`
  are part of the stable comparison side.
- **Yield semantics are user-facing.** The response says range semantics "get settled when
  those connectors are built", but `[[yield]]` is config, which it lists as frozen. What a
  yield means also decides where a decomp's state machine stops.
- **Others do write the baseline side.** `docs/connectors.md` has "Writing one for another
  core", and THWAITE_DECOMP_PLAN.md has the decomp export `dj_get_region`, `dj_clear` and
  the trace functions itself.
- **Changing it later isn't free.** Users build the cores themselves and pin
  `[core] build`, so every baseline-side change means a rebuild and `dj repin` in every
  project.
- **What survives:** the baseline-side run calls (`dj_set_points`, `dj_run_to_yield`,
  `dj_input_reads`) can stay provisional. That list is shorter than the response's, and it
  doesn't remove the need to settle what a yield means before the release.

### Smaller points

- **Strobes aren't logical reads.** Games that play DPCM samples often strobe the
  controller twice or more per read and compare the results. That's fine for the input
  rule, which only needs the first rise. Define the sub-frame index as strobes.
- **"Reserve the thread field's meaning now"** doesn't say what the meaning is.
- **Accepted as written:** the stride argument to `dj_set_points`, separate names for the
  list position and the yield's number in its frame, ticks through `dj_set_points` on
  API 3, the scanline 240 input limit, and `--explain` tracing to the yield.

### Run something before another round

Six rounds in, nothing has been run. The remaining disagreements are about what real code
does: whether Thwaite lags at all, what happens around interrupts during a wait, and
whether stopping before `Exec` is deterministic. Two cheap things would answer more than
another paper round:

- **The milestone 0 probe** from THWAITE_DECOMP_PLAN.md: where the CPU is at each frame
  end, and how many lag frames the 12 segments have.
- **A spike of the PC compare in Mesen,** with the hash test on the 12 segments, yields on
  and off.

Do both before the plan's sections above are rewritten.

## Experiments and where the design stands (2026-11-11 21:18 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-11 21:18 EST.
> Threads that Andrew has not given permission to edit this section must not change it:
> not to correct it, not to mark items done, and not to reword it as the task progresses.
> Record progress, answers and disagreements in a new section below this one.

This section has two parts. The first is what was run today. The second is where the
discussion with Andrew has moved the design since the 16:51 review. The second part
replaces some of what both threads wrote above; "What this replaces" lists which.

Nothing in the second part is built. The experiments ran from a scratch harness outside
the repo, on a copy of the Thwaite project, with the cores in `build/cores`. No repo file
was changed by them.

### What was run

**1. Save states replay exactly, on both cores.**

- **Thwaite on Mesen:** saved at frames 0, 1, 137, 500 and 1500 of "Defend" and at 0 and
  100 of "Pause and resume", ran up to 400 frames, loaded, ran again. The full save state
  (35,840 bytes) was byte-identical on every frame. Saving twice gave the same bytes, and
  so did load then save.
- **WWF No Mercy on Mupen64Plus** (pure interpreter, GLideN64, HLE RSP): saved at frame
  301, ran 120 frames, loaded, ran again. The RAM hashes matched on every frame.
- **The tick count is not part of the state.** It was unchanged by a load in every Thwaite
  run (for example 437 before the load and 437 after, where the saved moment had 37).

A save state replays exactly because it holds the whole emulated machine. That is not
automatic: CONNECTOR_PLAN.md already records that the angrylion plugin keeps state outside
the save state and drifts after a load.

**2. Where Thwaite's wait loops fall.** Every frame of all 12 segments was traced. A wait
was detected as `cmp nmis` followed by `beq` back to it.

| Question | Result |
| --- | --- |
| Is the game in a wait at a segment's first frame? | Yes, in all 11 segments that don't start at boot |
| Waits entered per frame | Exactly one in 11,013 of the 11,020 frames after boot; never two |
| Lag frames (no wait entered) | 7, one in each of 7 segments, at a screen change. The next frame starts mid-step |
| NMI | At cycle 121 of each frame (a frame is about 29,780 cycles) |
| When the step finishes (the wait is entered) | Between cycle 1,471 and 27,466 |
| Controller strobes | One per frame, after the NMI, never during a wait. 12 frames had none and 1 had two |
| Wait loops seen (address of the `cmp`) | `$80E5` game, `$8163` pause, `$9518`, `$9543`, `$95D9`, `$9857`, `$9BD9`, `$9C5F`, `$C426`, `$C430` |
| Ticks (`$8197`) | 0 on menus; 130 in "Pause and resume", as docs/playback.md says |

- **The flood case is real.** In the boot segment, the TV-system detection loop at `$C430`
  ran `cmp nmis` 2,693 times in one frame. A yield placed on a compare fires that often.
- **Lag is real but rare in Thwaite.** The 7 lag frames answer the open question in the
  16:51 review.

**3. Syncing a second emulator from RAM alone.** To imitate a decomp, a second copy of
Mesen was run to a different frame of the same segment. Only the baseline's 2 KB of system
RAM was copied into it. Both then ran on with the same inputs, and RAM was compared after
every frame.

| Baseline at | Second emulator was | RAM afterwards |
| --- | --- | --- |
| "Defend" frame 1000 | Given the full save state (control) | Identical on all 1,400 frames |
| "Defend" frame 1000 | In the same wait loop, at frame 999, 800, 744, 488 or 300 | Identical on all 1,400 frames |
| "Idle" frame 600 | In the same wait loop, at frame 344 or 300 | Identical on all 3,000 frames |
| "Idle" frame 3200 (end screen) | On the same screen, at frame 3100 | 3 stack bytes (`$1F5`-`$1F7`) differed on every frame |
| "Idle" frame 3200 (end screen) | Still in play, at frame 2200 | Up to 40 bytes differed on 3 frames, then identical |
| "Pause" frame 250 (playing) | In the pause loop, at frame 150 or 50 | Differed at once; 81 bytes still differed at the end |

- **RAM is enough when the receiver is in the right loop.**
- **RAM is not enough when the loop is wrong.** The pause case is what the yield's name is
  for.
- **The 3 stack bytes are below the stack pointer.** They are most likely the return
  address and flags the last NMI pushed, which depend on which instruction of the wait
  loop it interrupted. They don't affect logic. A byte-for-byte RAM compare, and a state
  hash over RAM, would both see them.

**4. What runs inside a save or a load on Mupen64Plus.** Coverage flags were cleared, the
call was made, and the flags were counted.

| Call | Distinct instructions that ran inside it |
| --- | --- |
| One normal frame | 2,034 |
| `retro_serialize` | 0 |
| `retro_unserialize` | 1,951 (974 when called again straight after) |

The patch explains this: a load takes effect at the next interrupt, so the game runs on
until then and is then overwritten.

**Limits of these runs.**

- The second emulator in run 3 is not a decomp, and it was synced at frame starts, not at
  a yield.
- The wait detector matches one instruction pattern. A wait written differently is missed.
- The N64 run is one game and one save point. No N64 decomp was synced.
- Tetris and the GBA core were not run.

### Found by reading the connectors

- **N64 ticks are not counted.** `dj_set_tick_exec` and `dj_get_tick_count` in the
  Mupen64Plus connector are stubs; the count is always 0.
- **N64 hooks fire in the pure interpreter only,** which the connector forces.
- **A save or load on N64 is a job that runs at an interrupt.** A save state can't be
  taken mid-frame there at all.
- **The N64 core runs the game on a coroutine** (`co_switch`). A callback from the
  instruction hook would run on that coroutine's stack; its size hasn't been checked.
- **mGBA** counts ticks in its instruction hook, the same way Mesen does.

### Where the design stands

These came out of discussion with Andrew after the 16:51 review. Items marked **Andrew**
are his ideas; the rest are this thread's recommendations. Andrew has not signed off on
the whole list.

**1. A yield is a blocking call from the connector into dj.** (**Andrew**)

- The connector's instruction hook calls a function dj gave it, then carries on. The
  baseline never returns to dj mid-frame.
- Inside the call, dj reads the baseline's memory live, drives the comparison core one
  step, checks, and takes coverage.
- This is as easy to add to any emulator as a counter. The hard part of the earlier
  design, returning out of `retro_run` and re-entering it, is gone.
- Limits: inside the call dj must not run, save, load or reset the baseline. It can't
  abandon the frame or hold it paused for a person.
- Captures into a buffer, read after the frame, stay as a fallback (see point 8).

**2. Two meanings of "sync", named apart.** (**Andrew**)

- **Sync from scratch:** the comparison core throws away its state and is set up again,
  after a seek, a segment change it didn't follow, or a failure.
- **Synchronized partial frame step:** the comparison core moves forward one step at a
  yield.
- A save state is only ever taken or loaded at a frame start. `sync = "state"` syncs from
  scratch at frame starts only.
- A decomp (`dj_sync`) may sync from scratch at a yield, since it reads RAM and the yield
  names the loop. Run 3 supports this.

**3. `[tick]` and `[[yield]]` are one mechanism.** (**Andrew**)

- A yield is a named place in the original's code where the connector calls dj.
- dj counts every yield by name. That gives ticks per loop or per thread, such as "audio
  ran 12 times, main once".
- `[tick]` stays as shorthand for "this yield is the project's tick", since dj reports
  `frame 812, tick 790` and stores a segment's ticks.
- API 2 connectors keep their tick functions; dj supports both for a while.
- The name is open. With this wider meaning "tick" may fit better than "yield". Andrew
  decides.

**4. The comparison core decides what to do at a yield, within limits.**

- The core declares at load which yields it wants to be called at. The rest are only
  counted, which costs no round trip to the worker.
- One call replaces "advance" and "check": the core is given the yield's name, steps if it
  wants to, and returns match, mismatch, not judged, or error.
- dj still records passes, failures and coverage. Reports must show how many yields were
  judged, since a core that returns "not judged" everywhere checks nothing.
- A core that needs the order of yields across threads asks to be called at each. Counts
  alone lose the order.
- On the N64 a yield still has to say which thread hit it.

**5. dj offers both moments for parity.** (**Andrew** raised passing an indicator.)

- dj calls at each yield and calls `dj_check_parity` at each frame end. The core answers
  "not judged" where it doesn't judge.
- One flag says whether checking is on, so a core can skip an expensive compare when it is
  only keeping in step.

**6. Inputs: the whole state, one entry per frame.** (**Andrew** asked whether to send all
inputs each time.)

- Each call carries one entry for every frame the baseline has been in since the previous
  yield: the frame number, the full input state, and the controller reads made in it.
- dj no longer needs a rule for which frame's inputs a step gets.
- For Thwaite this is simple: run 2 found one read per frame, right after the NMI.
- The read count has to come from the connector.

**7. Starting at a segment, not at boot.**

- **The first yield after any load is sync-only.** The baseline's previous yield happened
  before the save state was taken, so the counts and frame list for that interval are
  incomplete. The call needs a flag that says so.
- **Under yields, the first step of a segment entered by a load is not checked.** Today
  its frame 0 is. A boot start doesn't have this gap.
- **Ticks are counted from the segment start for display, and from the sync for the
  core.** There is no tick number since power-on. One would need a `start-ticks` header
  and a tick column on stored snapshots.
- **A segment boundary can fall mid-step.** `begin` clears the coverage flags, which would
  drop the first half of that step's flags. Credit the step to the segment where its yield
  lands, and carry the flags across.
- **Playing parent into child is not a jump.** `Checker::enter` already keeps the
  comparison core in step there; the open step and its flags must carry across too.
- **Seeking stays on frames.** "Go to tick N" loads the nearest frame snapshot and runs
  forward.
- **Yield calls and counts are off during a load.** Run 4 shows why.
- State hashes are unaffected.

**8. The same emulator as the comparison core.**

- Two emulators can't each block inside the other's call. Here each records a hash at
  each yield, and dj compares the two lists after the frame.

### What this replaces

- `dj_run_to_yield`, in the consolidated API (12:03) and in "Proposed API, first pass"
  (12:55): replaced by the blocking call in point 1. Rule 3 of the first pass ("the
  connector keeps no stop state") is met trivially.
- The `check` and `check_at` config keys: replaced by the core's own verdict (point 4).
- The input rule "the frame where the read count first rose": replaced by point 6.
- This thread's "counted-only kind" objection (11:12): redundant under point 3.
- "Proposed: checks at yields" in CHECKPOINT_PARITY_FLOW.md still shows the
  stop-and-return design. That chart is out of date.

### Still open

- **Where a yield goes in a spin loop.** It must be an instruction that runs once per
  wait. The `$C430` loop shows what happens otherwise. A cap per frame is still needed as
  a guard.
- **Bytes that differ without mattering.** The dead stack bytes from run 3 need an ignore
  rule for RAM compares against a decomp.
- **NES games whose logic runs in the NMI handler.** Not worked through, and Tetris was
  not run.
- **N64.** Thread identity at a yield, state held in registers and stack frames, and
  pointers in RAM are all unanswered. The coroutine's stack size is unchecked.
- **API version.** Whether the callback is API 3, and how API 2 connectors are kept.

### Confidence that sync at a segment start is accurate

| Case | Confidence | Basis |
| --- | --- | --- |
| Same emulator on both sides | Very high | Runs 1 and 3 (control rows) |
| Thwaite decomp, synced from RAM at a yield | About 90% | Run 3: held whenever the loop matched |
| NES games in general | 50-60% | Not tested beyond Thwaite |
| N64 decomp | About 30% | Nothing run tests it |

### Next

- Fix the tick undercount now ("step 0"): dj adds the count to a running total before each
  `dj_clear`.
- Spike the blocking call in the Mesen connector, with one yield at Thwaite's game wait,
  and re-run the hash test on the 12 segments with it on and off.
- Before any N64 design work, log which thread hits a candidate yield and how deep its
  stack is, on one game.

## Frames and ticks: roles, the map, seeking, and tick start and end (2026-11-11 22:19 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-11 22:19 EST.
> Threads that Andrew has not given permission to edit this section must not change it:
> not to correct it, not to mark items done, and not to reword it as the task progresses.
> Record progress, answers and disagreements in a new section below this one.

This section adds to "Experiments and where the design stands (2026-11-11 21:18 EST)".
It corrects one claim made there (see "A correction"). Andrew raised each question; the
recommendations are this thread's, and Andrew has not signed off on them. Nothing here is
built.

### Frames and ticks each get one job

Andrew asked whether dj should switch from frames to ticks as its main unit, since
thinking in both causes many of the plan's problems. The recommendation is not to switch,
and to give each unit one job.

- **Frames are the baseline's clock:** playback, input lines, snapshots, state hashes,
  seeking and TAS files.
- **Ticks are the comparison's clock:** stepping, verdicts, failures and passed coverage.
- **A failure is stored with both:** the tick, and the frame it fell in.
- **The frame count per tick is passed to the comparison core,** so a decomp's timing can
  still be tested.

Why not ticks everywhere:

- **A tick is defined by the project; a frame isn't.** Moving the tick's address would
  invalidate every tick-keyed input line, hash and snapshot. Segment files are the
  committed, shared files, and frame-keyed ones never have that problem.
- **The current tick doesn't cover the whole game.** Thwaite's tick runs 0 times on
  menus, so four of its 12 segments have no ticks. Boot has three frames with no wait.
- **A new project has no tick.** Frame mode has to stay, so both units exist anyway.
- **The emulator only stops on frames:** save states, video and `retro_run`.
- **A patched core becomes mandatory,** and the N64 connector counts no ticks today.
- **TAS import and export are frame-based.**
- **For Thwaite it buys little:** 11,013 of 11,020 frames had exactly one step.

What ticks everywhere would gain, and the split keeps: parity that matches how a decomp
runs, positions that survive timing differences, and failures that name the step.

Tick-keyed input lines could come later as segment format 2, since dj can set inputs
inside the call at a yield. They would tie segment files to the tick definition.

### The frame-to-tick map

- **dj records, per played segment, which ticks fell in which frame.** It goes in the
  local database. It is derived data and can be rebuilt.
- **Per frame it also records whether the game was waiting at the frame start,** and at
  which wait. Seeking uses this (next part).
- **Its key must include the tick definition.** The cache key doesn't today, so moving
  the tick's address in `config.toml` would leave a stale map.
- **It needs ticks to be deterministic.** That is not a new requirement: a tick is one
  instruction running, and dj already relies on the emulator replaying the same
  instructions for its state hashes.

What has been shown for determinism of ticks:

| Claim | Status |
| --- | --- |
| Same full save state on every frame after a load (Thwaite) | Shown, at 7 save points |
| Same ticks and waits across two runs from power-on (Thwaite) | Shown on per-segment summaries: tick totals, waits per frame and cycle positions matched |
| Same ticks per frame from a loaded state as from power-on | Not compared directly; it follows from the first row |
| Tick totals match the docs | One segment checked: 130 in "Pause and resume" |
| Any of this on N64 | Not shown: the connector counts no ticks there |
| Across machines or core builds | Not tested |

The missing check is small: record the map once from power-on and once from each stored
snapshot, and compare.

### How a decomp syncs after a seek

No extra frame has to run.

**The snapshot is at a frame start and the game is waiting** (the common case):

- The previous tick finished in the frame before. Only the wait loop has run since.
- On Mesen this frame's NMI hasn't fired yet: the frame starts at scanline 240 and the
  NMI comes at 241.
- So RAM at the frame start equals RAM at the last tick's finish. The decomp syncs from
  scratch right there, before anything runs.
- The tick that finishes inside the frame is then stepped and checked normally.

**The snapshot is mid-tick** (a lag frame; 7 of 11,020 frames in Thwaite):

- The decomp can't sync yet, because the game's state is half-updated.
- The baseline plays forward inside that frame to the tick's finish, and the decomp syncs
  there, inside the call.
- That one tick is not checked. Checking starts with the next.

**How dj tells the cases apart:** from the map, at seek time, without running anything.
Without a map, dj reads the program counter and compares it with the wait loops, or uses
the start and end counts below while playing.

Caveats:

- **It depends on where the emulator ends its frame.** One that ends frames after the
  vblank interrupt has already run the handler, and the decomp must be told.
- **It doesn't hold on the N64 as stated.** Other threads run between the main thread's
  wait and the frame boundary.

### A correction

The 21:18 section says, under "Starting at a segment, not at boot", that the first yield
after any load is sync-only and that the first step of a segment entered by a load is not
checked. That is true only for the mid-tick case. When the game is waiting at the frame
start, the decomp syncs at the frame start and the first step is checked. The matching
chart in CHECKPOINT_PARITY_FLOW.md ("After a load or a jump") shows the old claim.

### A tick with a start and an end

Andrew suggested storing a tick as a start and an end, not one address. The
recommendation is to do it, with the end required and the start optional.

- **The end is the yield:** where the connector calls dj, and where sync and checks run.
- **The start is count-only:** no call into dj.
- **In config,** `end` is required and `start` is optional. Each is a list, because a
  tick can start in one loop and end in another, as when Thwaite changes screens.

What the start adds:

- **dj can tell "waiting" from "mid-tick" by counting.** Equal starts and ends means
  waiting; one more start means a tick is in progress. The map stores this per frame.
- **The definition checks itself.** Two starts with no end between them, or many ends
  with no start, means the addresses are wrong. Thwaite's boot loop at `$C430`, which
  ran 2,693 times in one frame, would be flagged.
- **It marks when the game noticed the new frame.** Controller reads come after it.
- **It fits games whose logic runs in the NMI handler:** start at the handler's entry,
  end at its return.

It works for Thwaite. Each wait is `lda nmis`, then a `cmp` / `beq` loop. The end
is the `lda`; the start is the first instruction after the `beq` falls through. The
trace found both for all ten loops. An interrupt during the wait runs neither
instruction, so this avoids the problem with "entry into a range" (16:51 review).

Where it gets awkward:

- **Some ticks never reach an end:** a reset, or an NMI that interrupts an overrunning
  NMI handler. dj needs a rule for two starts in a row: an implicit end, or an error.
- **Counts don't survive a load.** They are in the connector, not the save state.
- **Multi-threaded games still need the thread.** On the N64 every thread uses the same
  wait function, so the pair is per thread.

Thwaite's existing tick, `doStateAction`, is neither a start nor an end. It is a point
in the middle of a step and stays a plain counted tick.

### Added to "Next"

- Record the frame-to-tick map for Thwaite's 12 segments twice, from power-on and from
  each stored snapshot, and compare.
- In the Mesen spike, use a start and an end for the game wait, and report any frame
  where the counts fall out of order.
