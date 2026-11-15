> **Archived. Agents: do not read this file unless the developer tells you to.** It is an earlier revision or a long design discussion, kept for the record, and it contradicts the current design in places. The current design is [PLAN.md](../PLAN.md), with charts in [FLOW.md](../FLOW.md).

# Plan: parity at check points, not frame ends

Status: written 2026-11-11, nothing built. **Before the first release** (decided
2026-11-11): the comparison core API changes shape here, so it should change before
anyone writes a decomp against the old one. Comes from the Thwaite decomp
([THWAITE_DECOMP_PLAN.md](../../THWAITE_DECOMP_PLAN.md)), whose comparison core has to line its
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
([connectors.md](../../../docs/connectors.md#comparison-cores)). The copy never runs a frame and
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
([connectors.md](../../../docs/connectors.md#writing-one-for-another-core), step 4).

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

## Map comparison: power-on against snapshots, Thwaite (2026-11-11 22:43 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-11 22:43 EST.
> Threads that Andrew has not given permission to edit this section must not change it:
> not to correct it, not to mark items done, and not to reword it as the task progresses.
> Record progress, answers and disagreements in a new section below this one.

This is the check the previous section listed under "Added to Next". It ran from the
scratch harness, on a copy of the Thwaite project and its cache. No repo file was changed.

### What was compared

One map entry per frame: whether the frame starts in a wait, each tick start and end with
its cycle and address, executions of the `[tick]` address, controller strobes, and the
number of trace events.

- **Reference:** each of the 12 segments played from power-on.
- **Snapshots:** 152. Five are stored in the project's cache and fall inside a segment.
  The rest were taken every 100 frames and on each side of every lag frame.
- **Replay length:** to the segment's end from a stored snapshot, 300 frames from the
  others.

### Result

| Segments | Frames compared | Entries that differ |
| --- | --- | --- |
| The 11 without a mouse | 35,754 | 0 |
| "Mouse in port 2: click 1 Player" | 2,100 | 600 |

- **Ticks per frame from a loaded state match power-on,** for Thwaite on Mesen. This
  fills the "not compared directly" row of the previous section's table.
- **Mid-tick snapshots matched too.** Every lag frame and the frame after it was a
  starting point.
- **Saving doesn't disturb the run.** A pass with a save before every frame matched a
  pass with none, outside the mouse segment.
- **The 5 stored snapshots were byte-identical** to the state the power-on run reached at
  the same frame.
- **The cache's stored tick totals matched the trace** for all six segments that have
  ticks (37, 130, 845, 1800, 2400, 2906).

### The mouse segment: a leftover port device

Andrew notes that Thwaite's mouse support is an outlier among NES games. The finding is
about dj and the core, not ticks, and it is recorded here because the map check found it.

The segment plugs a mouse into port 2 with a `[DEVICE]` line at its frame 0.

- **Replaying it from its start in the same process played it without the mouse.** RAM,
  the full state and the map differed on all 400 frames.
- **Likely cause:** `retro_set_controller_port_device` in Mesen's libretro layer does
  nothing when the port already holds that device. It still remembers "mouse" from the
  first play, while the state loaded for the replay has none plugged in.
- **dj doesn't reset the port.** `devices_at_start` sets only the ports listed in
  `config.toml` and those an ancestor's `[DEVICE]` line changed. Port 2 keeps whatever
  the last segment left.
- **Other paths were fine.** In fresh processes, loading a mid-segment state matched with
  either order of load and device setup, and a sibling segment was unaffected by a
  leftover mouse.

Not pinned down:

- One later snapshot (frame 70) also played wrong straight after the bad replay, while
  the ones after it were right. The exact rule is unclear.
- This was reproduced through dj's library calls (`start_trunk`, `restore_inputs`,
  `step`), not in dj-gui. `dj run` plays each segment once per process and would not
  hit it.

If it reproduces in dj-gui, the fix is small: when dj enters a segment, set every port
explicitly, including ports the config doesn't list.

### Limits

- Thwaite on Mesen only.
- Snapshots that weren't stored were replayed for 300 frames, not to the segment's end.

## The same tests on Tetris (2026-11-11 23:02 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-11 23:02 EST.
> Threads that Andrew has not given permission to edit this section must not change it:
> not to correct it, not to mark items done, and not to reword it as the task progresses.
> Record progress, answers and disagreements in a new section below this one.

Andrew asked for the Thwaite tests to be repeated on the Tetris example. They ran from the
scratch harness on a copy of the project, without its cache, so stored snapshots were not
part of the comparison. No repo file was changed.

### How a tick was defined

Tetris has no `[tick]` in its config. The tests used a start and an end, as proposed in
"A tick with a start and an end":

- **End:** the `nop` just before each wait loop, at `$AA36`
  (`updateAudioWaitForNmiAndResetOamStaging`) and `$AA4C`
  (`updateAudioAndWaitForNmi`). It runs once per wait.
- **Start:** the instruction after the loop's `beq`, at `$AA3B` and `$AA51`.

The wait is `lda verticalBlankingInterval` / `beq`, and the NMI handler sets that
variable.

### The map from power-on against snapshots

| Measure | Result |
| --- | --- |
| Segments, frames | 30 segments, 24,392 frames |
| Snapshots replayed | 299: every 100 frames, plus each side of every lag frame |
| Frames compared | 72,918 |
| Map entries that differ | 0 |
| Entries that differ on a plain replay from power-on | 0 |
| Starts and ends out of order | 0 |
| Frames with two tick ends | 0 |
| Lag frames | 15 |
| Frames that start mid-tick | 13 |

- **Ticks per frame from a loaded state match power-on** on Tetris too.
- **The start-and-end definition checked itself.** Starts and ends alternated across every
  segment.

### What Tetris does differently from Thwaite

| | Thwaite | Tetris |
| --- | --- | --- |
| NMI handler | `inc nmis`, `rti` | Rendering, the frame counter, the random number, controller polling |
| Tick starts at cycle (median) | About 140 | 1,985 (range 1,785 to 4,056) |
| Tick ends at cycle | 1,471 to 27,466 | 2,170 to 27,563 |
| Controller strobes | One per frame, inside the tick | Two per frame, inside the NMI handler, while the main code waits |
| Strobes inside a tick | All | None of 48,750 |

- **A decomp's step for Tetris is "vblank work, then the tick".**
- **The input for a tick is read before the tick starts,** in the same frame. The
  per-frame input list in "Where the design stands", point 6, covers this.
- **On a lag frame the handler runs in the middle of a tick** and changes RAM there. The
  frame list says how many frame ends a tick had, not where in the tick they fell. That is
  already listed as open.

### Save states

- RAM matched on every frame after a load: saves at frames 0, 1, 333 and 900 of "Rotate
  all pieces", and at 0 and 1500 of the Mode B TAS, with up to 400 frames replayed.
- Saving twice gave the same bytes, and so did load then save.

**One thing not explained.** On root segments the full save state sometimes differed after
a load, on nearly every frame, while RAM matched on every frame.

- It showed on both Tetris roots and on Thwaite's boot segment. It never showed on a
  segment with a parent.
- It alternated between consecutive tests in one process: the first differed, the second
  matched, the third differed.
- So it depends on what the process did before, not on the save state. Which part of the
  state differs was not found.
- dj's state hashes cover RAM only and are unaffected.

### Syncing a second emulator from RAM alone

| Baseline at | Second emulator was | RAM afterwards |
| --- | --- | --- |
| "Rotate all pieces" frame 600 | Given the full save state (control) | Identical on all 1,200 frames |
| "Rotate all pieces" frame 600 | In play, at frame 599, 500, 344 or 300 | Identical on all 1,200 frames |
| "Watch demo" frame 3000 | In the demo, at frame 2999, 2744 or 2000 | Identical on all 3,000 frames |
| "Pause and unpause" frame 250 (playing) | Playing, at frame 50 | Identical on all 50 frames |
| "Pause and unpause" frame 250 (playing) | Paused, at frame 150 | Differed at once; 10 bytes still differed at the end |

This is the same pattern as Thwaite: RAM is enough when the receiver is in the right
loop, and not when it is in the wrong one.

### Confidence

For NES games in general, sync at a segment start moves from 50-60% to about 70%. Tetris
covers a game with real work in its NMI handler. No game with all of its logic in the
handler has been tested.

### Limits

- Mesen only.
- The second emulator is not a decomp, and it was synced at frame starts.
- Snapshots were replayed for 300 frames, not to the segment's end.
- The project's real decomp comparison core ("decomp (3D)") was not run.

## The lag counter after a load (2026-11-11 23:25 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-11 23:25 EST.
> Threads that Andrew has not given permission to edit this section must not change it:
> not to correct it, not to mark items done, and not to reword it as the task progresses.
> Record progress, answers and disagreements in a new section below this one.

This explains the item "One thing not explained" in "The same tests on Tetris": on root
segments the full save state sometimes differed after a load while RAM matched.

### What differs

- **One byte,** in the controller block of Mesen's save state, at `_lagCounter`. In the
  first test it was 4 on the straight run and 3 after the load.
- **RAM was equal** in every case, and so was `_pollCounter`.

### Why

Mesen's `ControlManager` counts a frame as lag when the game didn't read the controller
during it.

- Each frame's input poll (`UpdateInputState`) adds one to `_lagCounter` if the flag
  `_isLagging` is still set, and otherwise sets the flag.
- Any read of the controller ports (`ReadRAM`) clears the flag.
- `StreamState` saves and loads `_lagCounter` and `_pollCounter`, but not
  `_isLagging`.

So after a load, the flag holds whatever the last frame run in that process left, and
the first poll counts or doesn't count on that basis.

### Confirmed by experiment

The same Tetris boot state (frame 2 of "Boot, credits and legal") was loaded twice in one
process, after two different histories, then run for six frames with no input.

| Before the load | Lag counter after 6 frames | Poll counter | RAM |
| --- | --- | --- | --- |
| 3 frames of play (the controller is read every frame) | 2 | 8 | Same |
| 3 frames of boot (the controller is not read on the last) | 3 | 8 | Same |

The state right after each load was byte-identical to the one loaded.

### Why it looked the way it did

- **Root segments only:** only boot has frames where the game doesn't read the
  controller. Elsewhere the flag is clear at every frame end.
- **Alternating between tests:** early in Tetris's boot the game reads the controller on
  every other frame.
- **Thwaite's boot segment** showed it for the same reason.

### What it means for dj

- **Hashes, playback and parity are unaffected.** The game can't read the lag counter,
  and state hashes cover RAM only.
- **Comparing whole save states byte for byte is not a valid test** on this core. A stored
  snapshot can differ by this byte from a fresh one of the same frame and still be right.
- **Three pieces of state are now known to sit outside a Mesen save state:** this flag,
  the port devices remembered by its libretro layer, and the connector's counts.

### The fix

Add `_isLagging` to what `ControlManager::StreamState` saves and loads, in
`connectors/mesen/dj-mesen.patch`.

- It changes the save state's layout. Stored snapshots and `snapshot` blobs made with
  the current core would need a format version check, or to be made again.
- It should ride along with the next change that already needs `dj repin`.
- Low priority: nothing depends on the lag counter today.

It is recorded in ROADMAP.md under "Known bugs". The port-device finding from the Thwaite
mouse segment is not in the roadmap.

## WWF No Mercy: frames and ticks on the N64 (2026-11-11 23:45 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-11 23:45 EST.
> Threads that Andrew has not given permission to edit this section must not change it:
> not to correct it, not to mark items done, and not to reword it as the task progresses.
> Record progress, answers and disagreements in a new section below this one.

Andrew asked whether ticks line up with frames in WWF No Mercy the way they do in Thwaite
and Tetris. They don't. First, the NES wording: in both NES games a tick almost always
ends mid-frame. What is rare there is a tick that crosses a frame boundary (7 of 11,020
frames in Thwaite, 13 of 24,392 in Tetris).

### What was measured

Mupen64Plus (pure interpreter, GLideN64, HLE RSP), 6,000 frames from power-on with Start
and A pressed in turn every 30 frames. That reached the menus, not a match. The N64
connector has no trace and no ticks, so each frame was judged by its coverage flags:
whether an address ran in that frame.

| Address | Routine |
| --- | --- |
| `0x80005818` | `pads_read`, the first thing the game loop does |
| `0x80038460` | `osViSwapBuffer`, the display flip |
| `0x80036110` | `rng_main`, the random number routine |
| `0x80036CD0` | `osRecvMesg`, the wait every thread uses |

| Measure | Result |
| --- | --- |
| Frames where the game loop ran | 2,788 of 6,000 |
| Gap between loop runs | 2 frames in 2,772 of 2,787 cases; 1 frame 8 times; 3 or more 7 times |
| RAM bytes changed in a frame with no loop run | Always some; median about 12,000 |
| Frames where `rng_main` ran | 5,763 of 6,000, including frames with no loop run |
| Frames where `osRecvMesg` ran | All 6,000 |

### From the decomp's own notes

These are from `build/aki_n64_decomp` (`_re_docs`, `tools/trace/README.md`,
`tools/n64emu/akiemu.py`), not measured here.

- **Match logic runs at 30 ticks a second:** one `match_tick` (`0x80114514`) per pass of
  `match_frame` (`0x800D9AB0`), which is `pads_read`, the tick, then drawing.
- **It slows under load and never catches up.** Singles average 2.000 video interrupts a
  tick; a heavy four-wrestler scene averages 2.04 to 2.12, with single ticks up to 2.43.
- **A tick takes 0.2 to 0.87 of a frame.**
- **The idle thread (`0x800004D0`) calls `rng_main` in a busy loop,** at `0x80000570`.
  The generator's state follows spare CPU time.
- **The decomp pins it** by replacing that call (`0x0C00D844`) with a no-op, in its
  emulator's `--no-idle-rng` mode. The camera also draws from the generator outside the
  match logic, so its tools copy the generator's state from the trace every frame and
  leave it out of the comparison.
- **The match clock** at `0x8016B930` goes up by one per match tick.

### What it means for the design

- **A frame with no tick end is the normal case.** The list of frames since the last
  yield will usually hold two entries.
- **The frame-start shortcut does not hold.** On the NES, RAM at a frame start equals RAM
  at the last tick's end, so a decomp can sync there ("How a decomp syncs after a seek").
  Here RAM changes between ticks, so a decomp can only sync at the yield itself.
- **Parity needs the generator pinned, or its state copied or ignored.** ROADMAP.md now
  has an item for pinning it with a patch to the game's code in memory.
- **Ticks matter more here than on the NES.** A frame is not a step of the game.

### Limits

- No match was reached in this run.
- Coverage flags say whether an address ran in a frame, not how often or in what order.
- Which thread ran what is unknown, and is still the main open point for N64 yields.

## No Mercy video: black picture with framebuffer emulation on (2026-11-12 00:10 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 00:10 EST.
> Threads that Andrew has not given permission to edit this section must not change it:
> not to correct it, not to mark items done, and not to reword it as the task progresses.
> Record progress, answers and disagreements in a new section below this one.

Not about parity. It is recorded here because the gitignored project `examples/nomercy`
points at this section from its config.

**Symptom.** WWF No Mercy on the Mupen64Plus core (GLideN64) showed a black picture in
dj-gui and in frames captured through dj's library, on macOS. The log line
`unit 1 GLD_TEXTURE_INDEX_2D is unloadable…` appears with or without the problem and is
not the cause.

**What was found** (temporary logging in `player/src/gl.rs` and `libretro.rs`, since
removed; both files are unchanged):

- dj's side works. With `mupen64plus-EnableFBEmulation = "False"` the picture arrives,
  the right way up, through the same framebuffer and read-back.
- With framebuffer emulation on, the core's final step draws a quad into dj's framebuffer
  with sane state (no depth, scissor or blend; full viewport; a complete 640x640 RGBA8
  texture). The texture it samples is the colour buffer of the core's framebuffer 2, and
  that buffer is black.
- The picture is in another of the core's framebuffers (6). A plain blit from it into
  dj's framebuffer shows it, upside down.
- The texture name of framebuffer 2 is the newest in the context, which suggests the
  buffer the core presents is being recreated and is empty when shown.
- Making dj's colour attachment a texture, as RetroArch's is, changed nothing.
- A GL "invalid operation" error is raised once per frame while the core draws into its
  own buffers. Where was not found: the core calls OpenGL 1.1 functions directly, so dj
  cannot intercept them.

**Not found:** why the core presents an empty buffer. It was not tried in RetroArch, so
it is unknown whether this is the core on macOS in general or something about how dj
drives it (the pure interpreter, the HLE RSP, the save and load patch).

**What was changed:** `examples/nomercy/config.toml` sets
`mupen64plus-EnableFBEmulation = "False"`. That also stops the GPU copying its picture
into RDRAM, which CONNECTOR_PLAN.md had flagged as a possible source of hashes that
depend on the GPU.

**Open:** whether the connector should force framebuffer emulation off, as it forces the
interpreter and the RSP plugin.

## No Mercy video: root cause found (2026-11-12 00:50 EST)

**Root cause: a bug in the core (GLideN64 in the libretro fork), not in dj.** In
`GLideN64/src/Graphics/OpenGLContext/opengl_BufferedDrawer.cpp`, the constructor enables
vertex attribute 9 (`rectAttrib::barycoords`) in the rectangle VAO unconditionally, but
gives it a buffer pointer only `if (_glinfo.coverage)`. This fork hard-codes
`dual_source_blending = false` in `opengl_GLInfo.cpp`, so `coverage` is always false, and
the rectangle VAO always has an enabled attribute with no buffer. Apple's OpenGL draws
**nothing** for such a VAO, without a GL error. Other drivers apparently ignore the
unused attribute. (`UnbufferedDrawer` already guards this correctly; desktop GL always
uses `BufferedDrawer`.)

Every `gfxContext.drawRects` call is lost: the framebuffer-emulation present copy
(`copyTexturedRect` in `FrameBufferList::renderBuffer`), the overscan copy, and N64 fill
and texture rectangles.

**How it was narrowed** (logging in a scratch copy of the core; `build/` untouched):
- `renderBuffer` picks the right buffer (vi_origin 0x38fbc0 → buffer at 0x38f800, 480x240,
  scale 4/3, texture 640x640). VI gamma is off, so no post-processing pass. The core's
  "framebuffer 2" from the earlier notes is the overscan buffer (`m_overscan`, made at
  init), so it is the target of the first copy, not a recreated game buffer.
- The source texture holds the picture (from about frame 500). The draw's state was all
  correct: program linked and valid, uniforms, viewport, no scissor, depth, cull, blend or
  rasterizer discard, colour mask, draw buffer, texture complete, vertices in the VBO
  correct. Yet a red clear just before the draw survived it, so no fragments were written.
- Bisect at the draw: GLideN64's program with a fresh VAO drew the picture. A fresh VAO on
  GLideN64's own VBO with the same pointers drew. GLideN64's VAO drew nothing even with a
  trivial program. Listing that VAO's attributes showed attr 9 enabled with buffer 0.
- The one-line fix below, built in a scratch copy, makes the picture appear with
  `EnableFBEmulation = "True"`, with overscan on or off (frames 700 and 1500 correct and
  the right way up). The Mandelbrot test ROM, black with FB emulation on before, now shows.

**The FB-off workaround is wrong too.** With the shipped core and FB emulation off, the
picture appears (the present is a `glBlitFramebuffer`), but fill rectangles are still
dropped. At frame 2500 the background is never cleared, so earlier frames (the Jakks
logo) smear behind the scene. With the fixed core the same frame is clean. Recordings
made with the current core on macOS have wrong video either way. Hashes (RAM, SRAM) are not
affected by the dropped draws themselves.

**The fix** (for `connectors/mupen64plus/dj-mupen64plus.patch`, not applied):

```diff
--- a/GLideN64/src/Graphics/OpenGLContext/opengl_BufferedDrawer.cpp
+++ b/GLideN64/src/Graphics/OpenGLContext/opengl_BufferedDrawer.cpp
@@ -29,7 +29,7 @@
-	m_cachedAttribArray->enableVertexAttribArray(rectAttrib::barycoords, true);
+	m_cachedAttribArray->enableVertexAttribArray(rectAttrib::barycoords, _glinfo.coverage);
```

**Options:** no core option avoids the bug. It is in VAO setup and every desktop-GL run
takes this path, so the option sweep was not needed. The connector does not need to force
any option. Once the core carries the fix, `examples/nomercy/config.toml` can go back to
`EnableFBEmulation = "True"` if wanted. FB emulation's RDRAM copies are a separate question
for hash stability.

**Not settled:** where the once-per-frame GL "invalid operation" comes from. `glGetError`
right after the dropped draw reported none. RetroArch is not installed, so the bug was
not reproduced there, but it is the same core code. `SNESEMU.N64` and `N64NICCC.N64` are
black in every configuration, before and after the fix.

## No Mercy video: fix applied to the connector (2026-11-12 00:37 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 00:37 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

Andrew approved applying the fix from "No Mercy video: root cause found".

- **The patch.** `connectors/mupen64plus/dj-mupen64plus.patch` has a new first hunk for
  `GLideN64/src/Graphics/OpenGLContext/opengl_BufferedDrawer.cpp`: the rectangles'
  barycentric attribute is enabled only when `_glinfo.coverage` is true.
  `connectors/mupen64plus/README.md` describes it under "What the patch does".
- **The core was rebuilt** with `connectors/mupen64plus/build.sh`. Its build id is now
  `mupen64plus@15bada85+dj.8c54df2b` (was `…dj.184d5849`).
- **Checked with the rebuilt core,** No Mercy from power-on with no input,
  `EnableFBEmulation = "True"`: frames 700, 1500 and 2500 show the picture, the right way
  up. Frames 1500 and 2500 have a clean black background; with the old core and
  framebuffer emulation off, frame 1500 showed the Jakks logo smeared behind the scene.
- **`examples/nomercy/config.toml`** is back to `EnableFBEmulation = "True"`, as
  `dj init` wrote it, and records the new build id. The earlier section's "What was
  changed" no longer applies.

Not done:

- Not checked in dj-gui.
- Whether state hashes are the same with the old and new core was not compared.
- Any other N64 project recorded with the old build needs `dj repin`.
- The once-per-frame GL "invalid operation" is still unexplained.
- The fix has not been sent upstream to mupen64plus-libretro-nx.

## No Mercy: ticks against frames in a singles match and a tag match (2026-11-12 11:14 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 11:14 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

Andrew recorded two segments in `examples/nomercy` (gitignored), both children of
"navigate to main menu" (864 frames): a singles CPU vs CPU match ("start CPU vs CPU
exhibition", 2,136 frames) and a tag match with four CPU wrestlers ("4 CPU match", 1,800
frames). Both were replayed from power-on in a scratch harness and both reached their
recorded end hash. Frame numbers below count from power-on.

**How it was measured.** Per frame: which of eight addresses ran (from the connector's
coverage flags), the match clock, the random number state, and how many RAM bytes
changed. There is no trace on N64, so this shows whether something ran in a frame, not
when or how often. "Tick" means one run of `match_tick` (`0x80114514`), counted at its
call site `0x800D9DFC`; the tick has returned when `0x800D9E04` runs. A frame is one
`retro_run`.

### Results

| Measure | Singles (frames 2066 to 2999) | Tag, four CPUs (frames 2057 to 2663) |
| --- | --- | --- |
| Ticks | 467 in 934 frames | 303 in 607 frames |
| Frames between ticks | 2 in 464 of 466 gaps; one 3, one 1 | 2 in 300 of 302 gaps; one 3, one 1 |
| Ticks that started in one frame and returned in the next | 1 (0.2%) | 162 (53%) |
| Match variables changed on frames with no tick start | usually 5 bytes | usually 7 bytes; median 33 |
| Match variables changed on frames with a tick start | median 567 bytes | median 854 bytes |
| Random number state | changed every frame | changed every frame but 2 |
| Match clock steps | 437 (paused for 62 frames) | 303, one per tick |

- **The tick rate is the same in both modes:** one tick every two frames, with one hiccup
  each near the bell.
- **The tag match splits more than half its ticks across a frame boundary.** In the
  singles match that happened once. In the tag match the share falls over time: 36 of
  the first 50 ticks, 15 of the last full 50. The cause was not measured; more wrestlers
  active is the likely one.
- **So "ticks rarely end mid-frame" is false for this game.** It held for Thwaite, Tetris
  and the singles match. It does not hold for the tag match.
- **Where a tick falls inside a frame comes from the emulator's timing,** not only from
  the game. Mupen64Plus counts a fixed number of cycles per instruction. A different
  setting, another emulator or a console would put the boundary somewhere else. A decomp
  build has no such boundary at all.
- **Between ticks the match state is nearly still.** Most RAM that changes on a frame
  with no tick is in the main program's data, where the decomp's notes place the frame
  and graphics task buffers. An earlier section said RAM changes a lot between ticks;
  that was about all RAM, not match state.
- **The match clock is not a tick counter.** It steps once per tick while it runs, but in
  the singles match it stood still for 62 frames (2910 to 2972) while 31 ticks ran, and
  in both matches ticks run for 40 to 80 frames before it first moves.

### Loads and overlays

- **Overlay memory is rewritten twice on the way into a match.** Singles: frames 1591 and
  1905. Tag: frames 1570 and 1899. The first is before Andrew's "has now loaded arena"
  marker (1830); the second is after "skipping intro" (1876) and brings in the match
  code. `match_tick` first runs at 1984 (singles) and 1973 (tag).
- **Loads have no game-loop pass.** Stretches of 81, 172 and 209 frames in the singles
  run, and 76, 172 and 209 in the tag run.
- **The same address holds different code at different times.** In the singles run's
  menus, `0x80114514` was flagged in 84 frames with no call from the match loop: another
  overlay was loaded there.

### What this changes in the design

1. **Frame-end checks against a decomp are not usable in the tag match.** At more than
   half of the frame ends the game is in the middle of a tick. The check has to be at the
   yield. This was the plan already; the tag match is the first measured case that needs
   it.
2. **A seek into a tag match will often land mid-tick.** The "sync at the tick's finish"
   path in the flow chart is the common case there, not the rare one.
3. **A tick must be defined by code address,** not by the match clock.
4. **An N64 yield address needs a guard for which code is loaded,** for example the
   expected instruction word at that address.
5. **Frames stay as the timeline.** A tick-only timeline would have holes of up to 209
   frames during loads, and inputs and markers still need a position there.
6. **The controller read in the game loop (`pads_read`, `0x80005818`) is a better primary
   tick than `match_tick`.** It runs in menus and matches alike.

### Not known

- The order of events inside a frame, and whether a tick is interrupted or simply starts
  late.
- Why the tag match splits ticks, and whether six or eight wrestlers would drop the rate
  below one tick per two frames. No such drop was seen with four.
- Which few bytes of match state change between ticks.
- Whether any of this matches a console. Only this emulator, in its pure interpreter,
  was measured.

## Keeping a decomp in sync, from boot and from a seek (2026-11-12 12:53 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 12:53 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

This follows from the tag match result above. It is a design answer; none of it is built
or tested.

**The rule: keep the decomp in step by ticks only, and never line it up with a frame.**
Frame ends fall mid-tick more often than not in the tag match, and a decomp has no
frames.

### From boot

1. **Step by yield.** Each time the emulator reaches the tick address, dj has the decomp
   run to its next one, then the declared memory is compared.
2. **Feed inputs by controller read, not by frame.** dj records what each read returned
   in the emulator and gives the decomp the same values in the same order.
3. **Treat anything that depends on timing as an input.** No Mercy's random number state
   moves every frame from the idle loop, which a decomp cannot reproduce. Two options:
   - Pin it in both, with the roadmap's patch idea. The AKI decomp does this. Segments
     recorded unpinned will play differently.
   - Copy the value from the emulator at each yield. This is correct only if the idle
     loop never runs in the middle of a tick, which has not been measured.
4. **Loads cost nothing.** They have no game-loop passes, so the decomp waits at its
   yield.

Risk not ruled out: the game may read a clock or a retrace counter inside a tick. Each
such read would need the same treatment as the random number state.

### From an arbitrary seek

| | Copy state at a yield | The decomp's own checkpoints |
| --- | --- | --- |
| How | The emulator loads the frame snapshot and runs to the next tick start; dj copies RAM into the decomp there | The decomp restores its nearest earlier checkpoint and replays the recorded reads up to the target tick |
| Works for | Builds that keep game memory at the original addresses (a rebuilt ROM, a recompiled port) | Any build, including a native port with its own pointers |
| Cost | The partial tick before the sync point is not checked | Replay time; dj must store per-tick reads and copied values |
| After a mismatch | Can sync again and keep checking | Stays diverged from that tick on |

- **Recommended:** checkpoints as the general mechanism; a comparison core may declare
  that it also accepts a state copy. A native port cannot take raw N64 RAM, because that
  RAM is full of N64 pointers.
- **Recommended:** drop "sync at the frame start when the game is waiting" for N64. It
  saves one tick of checking in the singles match and is wrong half the time in the tag
  match. "Always sync at the next tick start" is one rule for both. This changes the
  "After a load or a jump" chart in `CHECKPOINT_PARITY_FLOW.md`, which has not been
  updated.

### What has to exist first

- The Mupen64Plus connector's tick hooks are stubs, so no N64 yield exists yet.
- An N64 yield needs the overlay guard, and a check that the game thread hit the address
  and not another thread.
- The frame-to-tick map has to hold per-tick controller reads and copied values, not only
  counts.

## Letting the comparison core run itself at each yield (2026-11-12 13:20 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 13:20 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

Andrew's idea: dj stops forcing a "step" on the decomp. A yield callback lets the decomp
run itself, so different yields (an N64 overlay's tick, one of Thwaite's mode loops) can
affect it differently. A comparison core is frame-based by default or yield-based, and a
project could choose whether the baseline is frame-based or tick-based. This is a design
answer; none of it is built or tested.

### The decomp runs itself: yes, with three limits

dj would stop saying "advance one tick". It says "the baseline reached yield X, here are
the inputs since the last call", and the comparison core decides how far to run before it
returns. dj no longer counts the core's ticks or assumes one core yield per baseline
yield.

- **It is still a step, one the core defines.** The call must return before the baseline
  continues, or determinism is lost. A decomp that never reaches its matching point would
  hang dj, so the call needs a timeout.
- **dj still does the comparing.** If each core judges for itself, every core
  reimplements comparison and dj cannot show which bytes differ in one uniform way. The
  core runs; dj then reads the declared memory through it.
- **dj still needs a position, for seeking and checkpoints.** The position can be "the
  Nth yield call delivered". The core must then depend only on the sequence of calls, and
  must offer save and restore between calls.

A gain: if the decomp arrives at a different yield than the baseline did, that is a clear
mismatch report in itself.

### Frame-based or yield-based cores: yes, as one mechanism

The frame end is a built-in yield that every baseline has. A core subscribes to the
yields it wants; the default subscription is the frame end only. A second emulator,
Thwaite and Tetris work on the default. No Mercy's decomp subscribes to its tick. One
code path, not two modes.

### A tick-based baseline: recommended against

- **The emulator is frame-driven underneath.** libretro runs, latches inputs and takes
  snapshots per frame, and the N64 connector can only save at interrupts.
- **Loads have no ticks.** No Mercy has stretches of up to 209 frames with no game-loop
  pass; inputs and markers still need positions there.
- **Where a tick sits in a frame comes from the emulator's timing** (the tag match), so
  tick positions are not a stable thing to store segments against.

A project could choose how the timeline is displayed: frames, or ticks from the
frame-to-tick map. Storage and seeking stay in frames.

Andrew has not yet said whether he accepts the recommendation against a tick-based
baseline.

## The yield response, the flow, and where yields are stored (2026-11-12 13:52 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 13:52 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

**Decided by Andrew:** the frame end is the default yield. He likes "frame/vblank as
default yield"; the name "frame end" is this thread's suggestion (see the end).

Andrew's ideas in this round:

- The comparison core returns a bit mask or a struct that gives context: "I have
  completed a tick with this yield", "I have seen parity", or "this is a partial tick,
  but I have lost parity for these reasons, halt playback".
- A yield is a blocking callback to dj. dj passes it to the comparison core if it is
  subscribed (or passes every yield and lets the core ignore some), handles the result,
  and then unblocks the emulator if all is well.
- Yields might be stored in the project or local config, and baselines might specify
  their own.

The rest is this thread's answer. None of it is built or tested.

### The response

A small struct, not only a bit mask: the states are mostly exclusive, and reasons need
room.

| Field | Meaning |
| --- | --- |
| Status | not judged, parity seen, parity lost, or cannot continue |
| Tick completed | the core finished a tick at this yield, as opposed to a partial one |
| Reasons | a list: address and length where the core knows them, otherwise text |

- **"Tick completed" is the most useful part.** It gives dj safe points for checkpoints
  and a tick count for the timeline, without dj defining what a tick is.
- **"Parity seen" from the core is shown as core-reported.** For a rebuilt ROM, dj can
  compare declared memory itself. For a native port it cannot, and has to trust the core.
  The UI should say which happened.
- **Halting is dj's decision, not the core's.** The core reports "parity lost" or "cannot
  continue"; a project or user setting decides whether playback stops. This differs from
  Andrew's wording, where the core asks for the halt.

### The flow

Andrew's flow matches the blocking-call proposal, with two limits:

- **The emulator is unblocked on a mismatch too.** It cannot be saved, seeked or shut
  down cleanly in the middle of a frame. On a halt, dj copies the declared memory at the
  yield for inspection, lets the frame finish, then pauses.
- **Subscribe; do not pass every yield.** A yield address in a hot loop fires thousands
  of times a frame (Thwaite's boot loop ran its compare 2,693 times in one frame). The
  connector arms only the yields that dj or the core asked for.

### Where yields are stored

- **Address yields go in the project config.** They are facts about the game, they are
  shared, and on N64 they carry the overlay guard. Local config is for machine paths.
- **Which yields a core subscribes to goes with the comparison's settings.**
- **Baselines offer built-in named yields.** The frame end is one. A connector could add
  others it knows natively, such as the NMI or a controller read, with no address.
- **Yield definitions must not feed state hashes,** or editing one would dirty the whole
  tree. They key only the tick map and parity results.

**Naming:** "frame end", not "vblank". It is the point where the emulator returns from a
frame, and the tag match showed that this is not a fixed moment in the game.

## Decisions on the five open questions, and patching the ROM before boot (2026-11-12 20:11 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 20:11 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

The five questions were the ones left open in the flow chart's "Still open" table. The
options and their pros and cons were given in the thread; this records where each ended.

| Question | Decision | Who |
| --- | --- | --- |
| 1. Who decides a halt | dj decides, from a project or user setting. The core only reports "parity lost" or "cannot continue" | Andrew agreed |
| 2. A tick-based baseline | No. Frames for storage and seeking; ticks as a display choice | Andrew agreed |
| 3. A time limit on the call into the comparison core | None. A core that hangs is the core's bug, not dj's | Andrew |
| 4. Where per-yield records are stored | The local cache database, written as segments play, under the same key as the frame-to-tick map | Andrew agreed |
| 5. No Mercy's random number state | Pin it, with a project-level patch applied to the ROM before boot | Andrew |

Notes on each:

1. A "break here" hint from the core, honoured only in the GUI, can be added later.
2. The frame-to-tick map should record controller reads, so segments could later be
   converted to be keyed by read. Revisit if someone wants a decomp to play a segment
   without the emulator.
3. The limit was on dj's call into the comparison core; a baseline that never returns
   from a frame is the hang dj already has. The thread suggested one guard, a whole-run
   timeout on the CLI that exits the process, so a CI run with a spinning decomp does not
   hang. Andrew has not answered that. Not checked: whether GUI playback runs off the UI
   thread. If it does not, a spinning core freezes the window.
4. The comparison core's own checkpoints go in the same cache, keyed by its build.
5. See below. "Copy the value from the emulator at each yield" stays in the design but is
   not built until a measurement shows the state holds still inside a tick.

### The patch: project level, applied to the ROM in memory before boot

Andrew's decisions:

- **A patch belongs to the project,** not to a decomp comparison. Its hash is kept in the
  project.
- **Changing it dirties the whole tree.** That is correct: the game plays differently.
- **Every emulator core gets it,** the baseline and any comparison core that is an
  emulator.
- **It is applied to the ROM before boot, in memory.** dj reads the ROM, changes the
  bytes, and hands the core the buffer. This replaces the roadmap's earlier idea of
  writing to RAM once the code has been copied there.
- **Mesen is patched in its connector to take the ROM from the buffer, and to reuse that
  buffer on a power cycle.**

What the thread found in the cores' source (read, not run):

| Core | Where it takes the ROM from today |
| --- | --- |
| Mupen64Plus-Next | The buffer dj passes |
| mGBA | The buffer when one is given, the path otherwise |
| Mesen | The path. It takes a buffer only through libretro's extended game info call, which dj does not answer |

- **Mesen's change is small.** In `retro_load_game`, the branch used with no extended
  game info sets only the path; it can also take the buffer and size from the same
  struct. Mesen copies the buffer. The path must still be passed: Mesen uses it to find
  HD packs and to name battery saves.
- **Why the power cycle matters.** `Console::ReloadRom` builds the ROM from the path
  again, which would bring back the unpatched ROM without any error. dj's reset is
  libretro's soft reset, which does not reload, so today's flow is safe.

Consequences the thread raised, which Andrew has not disputed:

- **Some segments need re-recording, not only new hashes.** Menu navigation should
  replay. A CPU match plays out differently once the generator is pinned.
- **dj cannot patch a decomp.** A native port has different addresses, so the decomp
  carries the same change in its own build. dj can pass the patch list to the comparison
  core so it can confirm it matches.
- **Parity still needs an ignore rule for the generator's state,** as the roadmap says.

Not checked:

- Whether Mupen64Plus enforces the N64 boot checksum, which covers the first megabyte of
  the ROM. If it does, dj must fix the checksum after patching.
- Whether No Mercy's overlays are stored compressed. A patch to compressed code is not a
  simple byte change. The idle loop is in the boot code and is not affected.
- The ROM file offset of the idle loop's call. The roadmap gives the RAM address
  `0x80000570`.

`CHECKPOINT_PARITY_FLOW.md` has not been updated for these decisions: it still shows a
time limit and lists these questions as open.

## After a patch changes: every segment is replayed (2026-11-12 20:23 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 20:23 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

This corrects one line of the section above, which said "some segments need
re-recording, not only new hashes" and listed it as not ruled on. Andrew has now ruled:

- **A patch with a different hash means every segment has to be replayed.** No
  exceptions: the patched bytes are in hashed memory, so every recorded hash is stale.
- **No extra confirmation.** Someone who applies a patch knows the consequences. There is
  nothing to "accept": the segment plays, and the user saves it or does not.
- **So dj needs no new flow for this.** It is the dirty-then-replay path that a changed
  parent segment already uses.

The thread's point that a replay cannot tell a segment that still does what was meant
from one that no longer does (both only show a changed hash) is true, and it is left to
the user who watches the replay.

`CHECKPOINT_PARITY_FLOW.md` was updated at the same time for the five decisions: the
time limit is gone from "Inside the call", and the "Still open" table lists only what is
still open.

## Yield config, the call, and patch files: decisions and answers (2026-11-12 21:27 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 21:27 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

This records two rounds: the yield config discussion, and Andrew's answers to the list of
what was left to decide. "Thread" marks a recommendation Andrew has not confirmed.

### Decided by Andrew

| Topic | Decision |
| --- | --- |
| The N64 guard on a yield | Dropped. The comparison core can see which overlay is loaded and disregard a yield |
| Subscribing to yields | Dropped. dj passes every yield; the core disregards what it does not want. There should not be many yields |
| Built-in yields | Written like an address: `exec = "CORE:name"` |
| `exec` | May be a list of addresses |
| The tick | `[tick]` stays its own table, beside `[[yield]]`. With no `[tick]`, the tick is the frame end, as now. This replaces the `tick = true` idea from the same discussion |
| The connector API | Changed in place. dj is not released, so there is no second API to keep |
| What the comparison core is handed | The yield's name; which address fired when `exec` is a list; whether parity checking is on; the frames since the last yield with their inputs and reads; read access to the baseline |
| State copy and checkpoints | A core declares which it supports |
| On parity loss | Playback stops. There is no reason to keep playing. A user, or an agent through the MCP, can step on by hand to look at the drift |
| Patches | A patch is a file the config points to, with its own offsets and bytes. Not a list of bytes in the config |
| The N64 boot checksum | Not a concern for now |

Notes:

- **The API number.** Andrew said "API 1"; the code has `DJ_API_VERSION = 2`. Thread:
  raise the number anyway, so that a core built before the change gets today's "rebuild
  the core" warning and not a crash.
- **Cost of passing every yield.** Everything is one process and one thread: a yield is
  a function call from the connector into dj and another into the core. Not measured,
  but it should be nanoseconds. The costly parts are dj's own work around a judged yield:
  taking and clearing coverage flags (the roadmap blames one read per frame, about 40 MB,
  for halving N64 playback speed) and any memory comparison. dj must not do either for a
  yield the core disregarded.
- **dj's own tick count with no core loaded.** With the guard gone, a `[tick]` address
  inside an overlay also counts hits from other code at that address (84 in No Mercy's
  menus). A project should pick a tick address in code that is always loaded.

### Answers to Andrew's questions

**Status values (thread).** dj already has three, in `player/src/compare.rs`:
`DJ_PARITY_MATCH`, `DJ_PARITY_MISMATCH` and `DJ_PARITY_SKIP`. Keep them and add one:

| Status | Meaning |
| --- | --- |
| match | The core checked and saw parity |
| mismatch | The core checked and parity is lost. Reasons follow |
| skip | The core did not judge this yield: it disregards it, the overlay is not the one it wants, or checking is off |
| error | The core cannot continue |

Plus one flag beside the status: the core completed a tick at this yield.

**What else the core could be handed (thread).** Beyond Andrew's list:

- How many times this yield has fired, since the segment start and since the last sync.
- On the N64, which thread hit the yield.
- The patch list, once at load, so the core can confirm it was built with the same
  change.
- Later, any values copied from the baseline. Not built first.

**Sync at the frame start.** Andrew is right: with no yields, the frame end is the only
place to sync, and it works today for Tetris against its decomp. The thread's removal of
that path was wrong as stated. The rule that covers both: **dj offers a sync at every
yield, the frame end included, and the core accepts where it can.** The flow chart
already has "the core declines here". A project with only the frame end syncs at a frame
boundary. A No Mercy decomp would decline the frame end and accept at its tick.

**What a checkpoint is.** A save state of the comparison core itself, taken by the core
at a tick boundary, which it can later return to. It only matters for a decomp that
cannot take a copy of the baseline's memory, such as a native port whose pointers and
layout differ. To seek, such a core would return to its own nearest save and replay the
recorded inputs from there. A decomp that keeps memory where the original does (as with
`dj_sync` today) never needs one. Thread: do not build this until a native port exists.

**What ignore rules are.** Bytes that differ between the baseline and a decomp without
it being a real mismatch. Two seen so far: 3 dead stack bytes below the stack pointer in
the Thwaite RAM-sync run, and a pinned random number state that the camera also uses.
The question was where such a list is written. Thread: nowhere in dj. The core returns
the verdict, as `dj_check_parity` does today, so the core keeps its own list. This also
withdraws the thread's idea that dj does the comparing for a rebuilt ROM.

**The tick start list.** Earlier Andrew suggested storing a tick as a start address and
an end address, and the plan recorded "a required end list and an optional start list
used only for counting". The question was whether to keep the start part. Thread: drop
it. `[tick] exec` is the list of places where a tick ends, and the core's "completed a
tick" flag covers the rest.

**How ROM patch files work.**

| Format | What it holds | Checks the ROM it is applied to | Limits |
| --- | --- | --- | --- |
| IPS | Records of offset, length, bytes | No | Offsets below 16 MB |
| UPS | The differences, XORed with the original | Yes, by CRC32 | Sizes must match the patch |
| BPS | Copy and insert commands | Yes: CRC32 of the source, the result and the patch | None that matter here |
| xdelta (VCDIFF) | Copy and insert commands | Optional | Needs a larger decoder |

Thread's recommendation:

- **BPS, referenced from the config:** `[[patch]]` with a `file`. The format's own
  check of the source ROM replaces the "expected bytes" field, and it catches a
  byte-swapped N64 dump. A patch file holds only the changed bytes, so it can be
  committed.
- **dj applies it to the ROM in memory.** The hash of the patch file goes into the setup
  id.
- **Cost:** a binary file is not readable in a diff, and a one-word change needs a tool
  to make it (Floating IPS is the usual one).
- Mesen already applies IPS, UPS and BPS files it finds beside a ROM. dj should not rely
  on that; it would bypass the setup id.

### Still to decide

- Whether Andrew accepts the four status values, the extra items handed to the core, and
  BPS as the patch format.
- The names of the call and its struct.
- Whether the CLI gets a stall watchdog, and whether the timeline can show ticks. Both
  can wait.
- How to proceed: a consolidated spec, stub comparison cores, and the build order.

`CHECKPOINT_PARITY_FLOW.md` has not been updated for this section. It still shows the
core subscribing, a halt setting, dj comparing memory, and "Is this yield a tick start?".

## Final round: statuses agreed, checkpoints dropped, and a correction about processes (2026-11-12 21:35 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-12 21:35 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

**This file is now history.** The design as it stands is in
[yield-parity/PLAN.md](PLAN.md), with its charts in
[yield-parity/FLOW.md](FLOW.md). Read those to build from; read this file
for why.

### Decided by Andrew in this round

- **The four statuses are agreed:** match, mismatch, skip, error, plus the "completed a
  tick" flag.
- **The extra items handed to the core are agreed:** the yield's counts, the thread on
  the N64, and the patch list at load.
- **Checkpoints are dropped.** A comparison core brings itself fully into step from a
  seek or a sync. It is not dj's business to manage a decomp's or a port's memory.

What follows from dropping checkpoints (thread):

- The "state copy or checkpoints" choice is gone. There is one way: dj offers a sync at
  each yield and the core accepts where it can.
- Decision 4 of the five open questions (per-yield records in the local cache) has
  nothing left to store. The records were for replaying into a checkpoint, and copied
  values are not being built. Only the frame-to-tick map remains, for display.

### A correction: the comparison core is not always in dj's process

The thread told Andrew that a yield costs "nanoseconds" because "everything runs in one
process on one thread". That is wrong for the GUI, and the thread should have checked
before saying it. From `ISOLATION_PLAN.md`, `player/src/compare.rs` and
`player/src/isolate.rs`:

| Where | How the comparison core runs today |
| --- | --- |
| The GUI | In a worker process (`RemoteComparison`). Requests are JSON lines on a pipe, with bytes after them. A call that takes over 10 seconds ends the worker |
| Batch commands (`dj run`, `clean`, `hydrate`) | Each chain plays in one worker with the baseline and the comparison core together. No frame within 30 seconds ends it |

Three things change because of this:

1. **The time limit question was already answered by existing code.** Andrew's "none, a
   hang is the core's bug" stands as "nothing new": the worker limits above stay. The
   thread's CLI stall watchdog already exists as the 30-second rule.
2. **A yield in the GUI is a round trip between processes,** not a function call. Not
   measured. A few per frame should be fine. A yield in a hot loop would not be, which
   the per-frame cap already treats as an error.
3. **The baseline handle is the real problem.** In the GUI the handle given to
   `dj_sync` and `dj_check_parity` points to a copy of the baseline inside the
   worker, refreshed by loading the baseline's save state before each call. At a yield
   the baseline is in the middle of a frame and cannot be saved. So the worker has no way
   to see the baseline at a yield with today's mechanism. This is open, and it is listed
   first under "Open problems" in the new plan.

### Files changed with this section

- `CHECKPOINT_PARITY_FLOW.md`: the "Current proposal" charts now show the final design.
- `ROADMAP.md`: the patch entry points to a patch file, not bytes in the config.
- New: `yield-parity/PLAN.md` and `yield-parity/FLOW.md`.

## A review by another thread, folded into the new plan (2026-11-13 11:20 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-13 11:20 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

Another thread, which studies ticks against frames in emulation generally, read
`yield-parity/PLAN.md` and `yield-parity/FLOW.md` (not this file, not the code) and
called the design well-founded, with eight points. Andrew asked for them to be folded in.

| Its point | What was done in `yield-parity/PLAN.md` |
| --- | --- |
| Timing inputs other than the controller: the patch suits the idle-loop generator, not true inputs such as a retrace counter | The "dropped" row now says the core reads true inputs through the handle. dj gets no copy feature |
| Controller reads are a prerequisite on the N64; pass the bytes the game received | The call hands over each read's bytes. A new build step puts reads before N64 yields |
| Evidence is lost on a mismatch, because the baseline runs to the frame end | Reasons carry both sides' values. This had been in an earlier round and fell out |
| Name the expected instruction at a yield, to fix dj's own tick count | Not adopted: Andrew dropped the guard. Recorded under open problem 8 with the thread's objection that one word is a weak test |
| A port cannot reproduce where an interrupt lands | Stated as a limit under "Sync". It applies to every lag frame |
| Move the baseline into the worker | Recorded as the review's pick under open problem 1, with its cost: it is the unbuilt GUI step of `ISOLATION_PLAN.md`. The build order now starts with batch commands, where the problem does not arise |
| Switch to the main coroutine to make the call on the N64 | Recorded under open problem 4. Not checked |
| The same-emulator comparison assumes the same ROM | Stated in section 6. A new open problem 11 covers a rebuilt ROM that does not match and runs in an emulator, which the design has no answer for |

The thread's own cautions about the review: it judged from two files; nothing is built;
the 53% figure comes from one emulator in one mode; and there is still no decomp to test
the design against.

## Input through a yield at the game's input routine (2026-11-13 14:43 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-13 14:43 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

The reviewing thread asked for dj to hand the comparison core the bytes of each
controller read. Andrew asked how dj would know them, and the thread read each
emulator's source (not run):

- **Mesen** and **mGBA** ask dj for input once per frame; the game's reads are served
  from that, and dj does not see them. Each has an easy place for a connector hook
  (`ControlManager::ReadRAM`; mGBA's `keysRead` callback).
- **Mupen64Plus-Next** asks dj when the game sends a controller command, so dj sees each
  read, though not the exact bytes or which command it was.
- **bsnes** asks when the controller is latched. This was read from the newer
  `libretro/bsnes-libretro`, not confirmed for the `bsnes_accuracy` core. No bsnes
  source is in the repo.

**Andrew's decision:** do not build a generic record. The project lists the instructions
that count as an input poll as yields. It is more legwork for the user and should work
better than a generic solution.

How the thread wrote it into `yield-parity/PLAN.md` (new section 2a): the yield goes
where the game's input routine has finished, and the decomp copies the game's controller
buffer from the baseline through the handle. dj records no reads; no connector needs a
hook; "controller reads" left the call, the open problems and the build order. Both flow
chart files had two box labels changed to match. The artifacts were not republished, at
Andrew's request, so they are one revision behind.

## A second review, and input as the core's business (2026-11-13 15:01 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-13 15:01 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

The reviewing thread read the revised `yield-parity/PLAN.md` and raised eight points and
three smaller ones. Andrew asked for them to be folded in.

| Its point | What was done in `yield-parity/PLAN.md` |
| --- | --- |
| Adding any yield removes the core's input | A misreading the wording invited. Section 2a now says dj plays input by frame to both cores, always |
| The input yield belongs after the raw bytes, not after the routine | Adopted in 2a |
| The core must infer timing-dependent branches from which yield fires | Stated in section 2 as part of what a core's author must do |
| `[tick]` and a `[[yield]]` at one address | An error on load |
| `[WRITE]` lines reach a decomp at the wrong logical moment | A write puts a decomp out of step; it syncs at the next yield |
| A value read through the handle is only right where a yield sits at its use | Stated in section 2, with its cost in extra yields |
| Narrow the handle at a yield to memory and registers | Adopted. The thread withdrew its claim that sending memory cannot satisfy the handle; that described today's contract. Open problem 1 now has this as the thread's pick |
| "Skip" hides too much | dj counts match, skip and mismatch per segment and shows the share judged |
| Assert `randomize_interrupt = 0` | Not adopted: it is hard-coded, and the pinned commit and build id cover it |
| The bsnes row is from another repository | Moved out of the table |
| The lag-frame limit is about judging | Moved to section 2, with narrower wording: a port cannot work out where an interrupt landed, and often cannot act on it when told |

**Andrew's position on input, same day:** dj plays input by frame as an emulator or a TAS
tool would, and the comparison core can do what it wants. How the game polls is not dj's
concern. The thread agrees. Section 2a is now that statement, plus an optional technique
for a core whose ticks span frames.

Checked for this round:

- The Tetris decomp core (`examples/tetris/.decomp/libretro/tetris_libretro.c`) reads
  input through libretro in each `retro_run`, and uses the handle only for
  `retro_get_memory_data` and `retro_get_memory_size`.
- The AKI decomp's notes say No Mercy's match loop calls `pads_read`, then one
  `match_tick`, counts no elapsed video interrupts, and uses `osGetTime` only at
  start-up. Not measured by this thread.

The thread's caution: each review adds duties for a comparison core's author to a design
that has never run. The artifacts were not republished and are two revisions behind.

## The comparison core moves into the GUI's process (2026-11-13 16:23 EST)

> **Read-only section.** Written by the meta-analysis thread on 2026-11-13 16:23 EST.
> Threads that Andrew has not given permission to edit this section must not change it.
> Record progress, answers and disagreements in a new section below this one.

The open problem that had headed the new plan was that a comparison core in a worker
process cannot see the baseline in the middle of a frame. The thread's pick had been a
handle that fetches memory on request, over the pipe.

**Andrew's decision:** run the comparison core in the GUI's process. It is easier, and
with unsaved state backed up he is less worried about a hard crash in a comparison core.
He also wants the crash note to name the comparison core.

- This reverses step 2 of `ISOLATION_PLAN.md`, agreed 2026-11-10. That file has not been
  edited.
- On OpenGL, Andrew noted the Tetris decomp draws in software. The thread checked
  `player/src/gl.rs`: each core that asks for OpenGL already gets its own private
  context, so a comparison core would too. Two GPU cores in the GUI's process has not
  been tried.
- The thread's remaining worry is a decomp that corrupts dj's memory before it crashes.
  Backups do not cover that.

In `yield-parity/PLAN.md`: new section 9; open problems 1, 2 and 5 replaced (memory
corruption, an OpenGL comparison core in-process, reading registers); the build order now
starts with the move and no longer has a separate "yields in the GUI" step. The
artifacts were not republished.
