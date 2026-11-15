# Plan: parity by breakpoints

Written 2026-11-15 15:20 EST, from a conversation with the developer that day. The
developer asked for the design to be looked at afresh, with the comparison core doing
more and dj less. Nothing here is built.

Revisions, all 2026-11-15:

- 15:41 EST: a `[WRITE]` line may name an instruction address (section 3).
- 16:28 EST: rewritten after a critical pass and the developer's answers. New: the jump
  and console-reset events, pre-roll (section 6), the handle's shape, and stricter rules
  for crediting coverage. The file now stands alone.
- 16:57 EST: a second critical pass, and [STATES.md](STATES.md) written to check the
  rules case by case. Changed: what "unjudged" claims, the jump at an exact frame, play
  and step after a mismatch, pre-roll of one tick, how a connector matches addresses,
  and the build order.
- 17:30 EST: the developer's decisions on the open questions. `dj_handle_event` and its
  struct are written out (section 2). A write goes through its own function,
  `dj_handle_write` (section 3). Playback pauses only at the first mismatch of a run.
  Pre-roll is zero unless the `[[compare]]` entry sets it. "Unjudged" carries a reason.
  The tail of a segment expects no verdict. The worker is turned down.

How to read it:

- **This is the current design.** Charts are in [FLOW.md](FLOW.md).
- **[STATES.md](STATES.md) walks every case through these rules:** each state dj and the
  core can be in, each moment between frames, and each verdict in each place. Read it
  before changing a rule here, and add the case there.
- **"Developer"** marks what the developer proposed or chose.
- **"Thread"** marks a recommendation the developer has not confirmed.
- **Everything in [archive/](archive/) is older:** two earlier plans, their charts, and a
  long design discussion. Agents: do not read those files unless the developer tells you
  to. This file does not depend on them.

## The goal

Compare a decomp against the original game at moments that mean the same thing in both.
Today dj compares once per frame, at the frame end. That does not work when a frame ends
in the middle of the game's logic step.

## The idea

The comparison core is a debugger client. It says where it wants breakpoints in the
original game. dj runs the baseline and calls the core each time one is hit, with the
baseline stopped and readable. The core runs itself, compares, and answers with a
verdict. dj plays, delivers events, and records verdicts. It does not manage the
comparison core's state.

## Terms

| Term | Meaning |
| --- | --- |
| Baseline | The original game, running in an emulator core with a dj connector |
| Comparison core | A decomp or port, built as a libretro core, that is compared against the baseline. A project lists its comparison cores in `[[compare]]` entries in `config.toml` (`docs/config.md`) |
| Frame | One `retro_run` of the baseline. Segments, inputs, snapshots and seeking are in frames |
| Tick | One step of the game's logic. The project's `[tick]` names where one ends |
| Breakpoint | An address in the original game where the comparison core wants to be called |
| Event | One call from dj to the comparison core |
| Verdict | What the core answers to an event |
| Jump | The baseline's state was replaced: a seek, a snapshot, a power-on. Also the name of the event that says so |
| Live | The comparison core has copied the baseline since the last jump. Only the core knows this |
| Pre-roll | Frames dj plays before the place the user asked for, so the core can go live first |
| `dj parity` | The command-line command that checks segments against a comparison core without the GUI. CI runs it, and so do the MCP server's parity jobs |

## What exists today

- **One check per frame.** After each frame dj runs the comparison core's frame, then
  calls its `dj_check_parity` or compares system RAM (`player/src/compare.rs`).
- **dj syncs the core** after every jump with `dj_sync`, and tracks whether it is in step
  (`parity::Checker`).
- **The comparison core runs in a worker process,** in the GUI and in `dj parity`.
- **`dj parity` jumps to each segment.** It loads the parent's end state and checks from
  frame 0 (`player/src/parity_cmd.rs`).
- **`[tick] exec`** names one address that the connector counts. `dj_clear` zeroes the
  count, so stored tick counts are too low whenever flags were cleared.
- **Coverage flags are taken once per frame** during a parity check, and credited to that
  frame's verdict.
- **A `[RESET]` line** is a soft reset. dj calls `retro_reset` on both cores.
- **A `[WRITE]` line** is applied between frames. A comparison core follows it through
  `dj_write`, or is synced again.

## The design

### 1. Breakpoints

The comparison core provides them, through a function dj calls once after the game is
loaded. (Developer: a core is either coded to handle a breakpoint or it is not, so the
list belongs with the code. Thread: the shape below.)

```c
struct dj_breakpoint {
    uint32_t    id;       // the core's own number, handed back in the event
    const char *region;   // the baseline connector's region: "PRG", "RDRAM"
    uint64_t    offset;
    const char *name;     // for dj's messages only
};

// Returns how many there are. The list stays valid while the game is loaded.
size_t dj_breakpoints(const struct dj_breakpoint **out);
```

- **dj checks each region against the baseline's connector** when the core loads, and
  refuses the core with a message if one is unknown.
- **A core without the function gets no break events.** It still gets the others. That is
  the Tetris decomp today.
- **There is nothing in the project config.** Two comparison cores for one game each
  bring their own list.
- **A ROM revision is the core's business.** It is asked after the game is loaded and can
  choose its table then.
- **The list is fixed at load.** An N64 core registers every breakpoint it may want and
  answers "pending" where one does not apply, for example when another overlay sits at
  that address. If that proves too noisy, an event could return "my list changed" and dj
  would ask again. Not in the design until a decomp needs it.
- **A breakpoint hit more than a cap in one frame is switched off until the frame ends,**
  with one warning that names it. It does not stop playback.
- **A load fires no breakpoints.** On Mupen64Plus a load runs instructions inside
  `retro_unserialize`; dj ignores hits during it.
- **`[tick]` is unchanged in the config,** and is not part of parity. dj arms it as a
  breakpoint of its own and counts the hits itself, with or without a comparison core.
  `dj_clear` no longer touches a tick count.

### 2. Events

One function in the comparison core, `dj_handle_event`, takes every event but a write. A
write has its own function (section 3).

| Event | When | What it carries |
| --- | --- | --- |
| jump | Between frames, before the first other event after the baseline's state was replaced. Also after the core was loaded, switched on, halted, or the user asked for a fresh copy | Why: a fresh power-on, a replaced state, or "nothing was replaced, copy again" |
| console reset | Between frames: a `[RESET]` line was applied to the baseline | Nothing more |
| break | The baseline hit one of the core's breakpoints. The baseline is inside `retro_run` and waits | The breakpoint's `id` |
| frame end | The baseline's `retro_run` returned | Nothing more |

Every event also carries a segment and a frame, whether this is pre-roll (section 6),
and a handle.

```c
enum dj_event_kind  { DJ_EVENT_JUMP = 1, DJ_EVENT_CONSOLE_RESET, DJ_EVENT_BREAK,
                      DJ_EVENT_FRAME_END };
enum dj_jump_reason { DJ_JUMP_POWER_ON = 1, DJ_JUMP_STATE, DJ_JUMP_COPY_AGAIN };
enum dj_verdict     { DJ_PENDING = 0, DJ_MATCH, DJ_MISMATCH, DJ_UNJUDGED, DJ_HALT };

#define DJ_PREROLL 1u       // in `flags`: this frame is pre-roll

struct dj_event {
    uint32_t size;          // sizeof(struct dj_event), so fields can be added later
    uint32_t kind;          // enum dj_event_kind
    uint32_t flags;         // DJ_PREROLL
    uint32_t breakpoint;    // DJ_EVENT_BREAK: the core's own id
    uint32_t jump_reason;   // DJ_EVENT_JUMP: enum dj_jump_reason
    const char *segment;    // the segment's id
    uint64_t    frame;
    const struct dj_baseline *baseline;   // read the baseline; valid during the call
};

// Returns an enum dj_verdict. `reason` is text for unjudged, mismatch and halt.
int dj_handle_event(const struct dj_event *ev, char *reason, size_t reason_len);
```

- **The name and the struct are the developer's choice** (2026-11-15), from the thread's
  suggestion.
- **"Pending" is 0,** so a core that does not act on an event returns 0.

- **Which frame an event carries.** Between frames (jump, console reset, a write with no
  address): the frame about to run. Inside a frame (break, a write with an address): the
  frame running. Frame end: the frame just run. Between frames the events come in the
  order jump, console reset, write.
- **"Jump" is not "console reset".** (Developer.) A `[RESET]` line is a soft reset of
  the console. The game has logic for it, and so should the decomp. dj applies it to the
  baseline with `retro_reset`, as now, and tells the core. dj does not call
  `retro_reset` on a core that takes events.
  - **A core parked at a frame boundary runs its own reset path and stays in parity.**
  - **A core that judges at a tick cannot.** It is parked at its last tick while the
    baseline was part-way through the next, so its state is not the one the game reset
    from. It treats the event as a jump. So does a core with no reset path yet.
- **dj sends one jump however many loads came before it.** Dragging the ruler loads many
  states; the core hears of it once, before the next frame that runs.
- **The jump arrives with the baseline between frames and readable.** A core that judges
  at frame ends copies the baseline there and then, and the frame that follows is
  checked, as today. A core that judges at a tick waits for its tick breakpoint.
- **A parent segment playing into its child is no event.** Nothing was replaced. If the
  parent's last frame ended in the middle of a tick, the core is parked at its last
  breakpoint and stays there until the child is picked and plays. Its held coverage
  (section 5) goes with it. A jump is owed only when dj loads a state.
- **The GUI's "Sync now" sends a jump** with the reason "copy again".
- **dj does not sync the core.** After a jump the core answers "pending" until it
  reaches an event where it can copy what it needs. Which event that is, is its own
  choice.
- **A write is the core's to handle,** through its own function (section 3).
- **The call has no time limit in the GUI.** `dj parity` keeps its rule: no frame within
  30 seconds ends the chain.
- **dj does not call `retro_run` on a core that takes events.** The core runs itself
  inside the event call, for as long as it decides.

**The handle** is the `baseline` field of the event: what a core uses to read the
original game during a call. (Thread: the definition below.)

```c
struct dj_baseline {
    uint32_t version;
    void    *ctx;
    // A memory region by the name [WRITE] and [hash] use: "RAM", "SRAM", "VRAM", "RTC".
    // NULL if the baseline has none. Read only.
    const uint8_t *(*memory)(void *ctx, const char *name, size_t *size);
    // The CPU's registers, in a layout fixed per instruction set. Returns the bytes
    // written, 0 if the connector cannot say.
    size_t (*registers)(void *ctx, void *out, size_t cap);
};
```

- **It reads memory and registers, and nothing else.** The baseline must not be run,
  saved, loaded or reset through it. Today's `get_proc`, which gives any function of the
  baseline by name, goes.
- **Two sets of names are in play, as today.** Breakpoints and instruction addresses use
  the connector's code regions (`PRG`, `RDRAM`). Memory uses the names `[WRITE]` and
  `[hash]` use.
- **The memory pointer is into the baseline** (section 11), so nothing is copied unless
  the core copies it. Nothing stops a core writing through it; a write would change the
  baseline's state hashes without any error.
- **It is good only for the length of the call.**

### 3. Writes at an instruction address

Asked for by the developer. A `[WRITE]` line may end with an instruction address:

```
812 [WRITE] RAM 0x0451 63 @PRG:0x0197
```

- **Why:** a write between frames lands wherever the emulator ends its frame, which can
  be the middle of a tick. It is repeatable, but a decomp cannot apply it at the same
  moment in its own logic. A write at an instruction lands at a known place in the
  game's code.
- **A line with no address is applied between frames, as now** (Developer). It needs no
  connector. Many cheat tools work this way.
- **A line with an address is armed when its frame starts.** dj adds the address to the
  connector's breakpoints for that frame. The first time the baseline executes it, dj
  writes the bytes with the baseline stopped there, then disarms it.
- **If the address is not reached in that frame, the write is dropped with a warning in
  the console** that names the line (Developer: a warning, for now). Playback goes on.
  The write is not applied and the comparison core is not told. It does not stay
  armed into later frames: a save state taken in between would not say that a write was
  still waiting, and every stored snapshot would have to carry that list. The fix for
  the author is to move the line to a frame where the address runs.
  - **A dropped write is still repeatable,** so hashes hold. But the segment then does
    not do what its line says, and only the console shows it. The developer has set this
    aside as an edge case for later.
- **The comparison core is told at that same moment,** inside the frame. When one
  address is an armed write's and also a breakpoint of the core's, the write comes
  first, so the core sees it before it compares.
- **A core with a breakpoint at the write's address can apply it exactly.** It is parked
  there, so it needs no fresh copy and the frame stays checked.
- **A comparison core that is the same emulator gets the write at the same instruction.**
  dj arms the address in its connector too (section 10).
- **The address is written out in the line.** There are no named addresses in the
  project config and no `@tick` (Developer: maybe later). The cache key and the recorded
  hashes follow the line's text, so they already follow the address
  (`cache::body_hash`). Names can be added later without changing how these lines play.

**How a comparison core gets a write.** Through its own struct and function (Developer),
for both kinds of line:

```c
struct dj_write {
    uint32_t size;          // sizeof(struct dj_write)
    uint32_t flags;         // DJ_PREROLL
    const char *segment;
    uint64_t    frame;
    const struct dj_baseline *baseline;
    const char    *region;      // what was written: "RAM", "SRAM"
    uint64_t       offset;
    const uint8_t *bytes;
    size_t         len;
    const char    *at_region;   // the instruction address, or NULL if the line had none
    uint64_t       at_offset;
};

// Optional. Returns an enum dj_verdict, as dj_handle_event does.
int dj_handle_write(const struct dj_write *w, char *reason, size_t reason_len);
```

- **dj has already written the baseline** when it calls.
- **The core does not register for writes.** Every `[WRITE]` line is sent to it. The
  segment decides where writes happen.
- **A core parked at the moment the write lands applies the bytes** to its own state and
  answers "pending". That is a core at a frame boundary for a line with no address, or a
  core with a breakpoint at the line's instruction address.
- **A core parked anywhere else goes stale** and copies the baseline again at its next
  good event, as after a jump. A core that cannot cope answers "halt".
- **A core without the function cannot follow writes.** dj then owes it a jump, with the
  reason "copy again", before the next frame. For a line with an address, dj also treats
  the rest of that frame as unjudged: the core's state is wrong from the write on.
- **Why its own function:** the event struct stays small, and following writes is
  optional.

What it costs:

- **A segment with such a line needs a baseline connector.** Without one dj refuses the
  segment and says why.
- **It adds a form to the segment format.** Today's parser already rejects it, which is
  what an older tool should do. dj is not released, so this is the time.
- **dj writes the baseline's memory in the middle of a frame,** which it has not done
  before. It should be safe for RAM on Mesen, and on Mupen64Plus's interpreter, which dj
  already forces. Each connector needs checking (open problem 8).
- **On the N64, other code at that address in another overlay triggers the write.** The
  same caveat as `[tick]`.

### 4. Verdicts

| Verdict | Meaning | What dj does |
| --- | --- | --- |
| pending | Nothing to say, and nothing unvouched for | Nothing |
| match | What ran since my last verdict has parity | Credits the held coverage (section 5) |
| mismatch | It does not. Reasons follow | The first of a run: records a failure and pauses playback at the frame boundary. Later ones in the run: shown, nothing more. The core stays live |
| unjudged | I cannot vouch for what ran since my last verdict. A reason may follow | Drops the held coverage. Given inside a frame, it drops that frame's too |
| halt | I cannot meaningfully continue. A reason follows | Records a failure. Stops playback at the frame boundary. Sends no more events until a jump |

- **The event at which a core copies the baseline answers "unjudged",** not "pending".
  What ran between the jump and the copy was compared with nothing. With "pending" the
  first match afterwards would credit it.
- **"Halt" is separate from "mismatch"** (Developer). A mismatch may put itself right; a
  core that has hit an unimplemented function, or has lost its footing, has one safe way
  to stop. It replaces today's failed `dj_run`, and covers a copy the core cannot make.
- **After a mismatch the core keeps getting events.** What it does next is its own
  choice: carry on from its own state, or copy the baseline again.
- **A run of mismatches is one failure** (Developer). A mismatch may put itself right,
  so only the first of a run counts: the one whose last verdict before it was not a
  mismatch. Pending verdicts in between do not count.
  - **The first is recorded, and playback pauses** at the frame boundary.
  - **Play or step then carries on as normal playback.** The core keeps its own state.
    Later mismatches in the run are shown but not kept, blame no code, and do not pause.
    Drift can be watched while playing.
  - **If the core matches again and later mismatches, that is a new failure** and a new
    pause.
  - **"Sync now" makes the core copy the baseline again,** when the user wants that.
  - **`dj parity` stops checking a segment at its first failure,** as now.
  - **The last verdict is "none" after a jump and at the end of pre-roll.** Verdicts in
    pre-roll are ignored, and must not hide the first mismatch after it.
- **After a halt, play or step sends a jump** with the reason "copy again" and loads
  nothing. A halted core gets no other event first.
- **A verdict given between frames claims nothing ahead.** It can only speak for what has
  run. STATES.md, section 3, has every verdict in every place.
- **A reason, as text,** comes back in the buffer the call is given, for unjudged,
  mismatch and halt (Developer: unjudged gives one too). The tools can then say why
  frames were skipped, such as an overlay that is not decompiled yet.
- **What differed,** for a mismatch: a list of an address and a length where the core
  knows them, each with both sides' values. The struct is today's `dj_parity_diff`, read
  through today's `dj_parity_diffs`. dj reads it before the baseline moves on.
- **The core gives every verdict.** dj compares no memory and keeps no list of bytes to
  ignore.
- **With parity checking off, dj discards match, mismatch and unjudged.** A halt still
  stops playback. The core is not told.

### 5. Coverage

dj takes the connector's flags once per frame, at the frame end, as it does today. It
never clears them in the middle of a frame.

- **Each frame's flags go onto a held pile,** before the frame-end event is sent.
- **A match credits the pile** to the build's passed set and empties it. Inside a frame,
  that is the frames before this one.
- **"Unjudged" empties the pile.** Given inside a frame, it also discards that frame's
  flags when they come.
- **A recorded mismatch, or a halt, gives the failure the pile.** Given inside a frame,
  the failure also gets that frame's flags when they come.
- **A verdict between frames does not touch the frame ahead.** A frame-end core that
  copies the baseline on a jump answers "unjudged" there, and the frame that follows is
  still credited by its own match.
- **A jump empties the pile.**
- **"Pending" leaves the pile alone.**
- **Pre-roll frames are never put on the pile** (section 6).
- **Turning parity checking on clears the connector's flags,** so the first frame does
  not carry what ran before.

For a core that judges at every frame end this is exactly today's behaviour. For a core
that judges at a breakpoint inside a frame, credit is by whole frames, and every error is
on the safe side:

- A match credits the code a frame ran before the breakpoint one verdict late.
- A failure blames the whole frame it arrives in, and the frames since the last match.
- "Unjudged" discards the whole frame it arrives in.

In exchange, the recorder and the GUI's live view go on reading flags after each frame,
unchanged, and the N64 pays for its 8 MB of flags once per frame, as now.

**What a parity run stores does not change shape.** `parity_runs` counts frames: passed
(credited by a match), skipped (dropped by "unjudged"), unchecked (dropped by a jump, or
still held at the segment's end when the last verdict was not a match). The share judged
is passed over all three. A core
that answers "pending" for ever shows as frames unchecked.

**The tail of a segment expects no verdict** (Developer). A tick core's last match in a
segment leaves the final frame or two on the pile: the baseline is part-way through a
tick that the segment does not finish. When a segment ends with the last verdict a
match, `dj parity` does not count those frames as unchecked and does not report them.
Their code is credited to nothing, so code that ran only there shows as not checked on
the coverage map.

### 6. Pre-roll

After a jump, a core that judges at a tick cannot go live until its next tick. Without
help, the first frames of every segment that `dj parity` checks would go unchecked. So
when dj jumps somewhere to check parity, it starts the baseline earlier and plays up to
the place asked for. (Developer.)

- **When:** any jump made while checking parity. `dj parity` jumps to every segment's
  start. In the GUI, a seek or a selected segment is a jump.
- **Not when playing straight on.** A parent that carries on into its child (the GUI's
  "Next", into the sticky or only child) is not a jump and needs none.
- **Not for a root segment's start.** There is nothing before power-on.
- **Not for "copy again".** Sync now, play after a halt, and switching the comparison on
  load nothing, so there is nowhere earlier to start from.

**Pre-roll is zero unless the comparison core's `[[compare]]` entry sets `preroll = N`**
(Developer). A core that can copy the baseline between frames needs none: the average
NES game can be picked up from RAM at a frame boundary and followed frame by frame. A
core that cannot is a special case that needs setting up anyway, and its author sets the
number then: enough frames to be sure of reaching the core's copy point, such as one
full tick. dj does not work the number out from ticks.

Call the place asked for T. The jump event is sent at the exact frame J, N frames before
T, and the frames from J to T are the pre-roll. J stops at a root segment's start. It may
fall in the parent or further up.

Where the baseline starts is a separate matter:

- **dj loads the nearest stored snapshot at or before J,** an ancestor's included, or
  failing that the parent's start, which is always available.
- **It plays from there to J with no events sent.**
- **The jump is sent at J whatever was loaded.** So a failure re-runs the same way on a
  machine with other snapshots, or none (section 9).

During pre-roll:

- **Every event says it is pre-roll.** The core can skip its comparison. The GUI can show
  "pre-roll".
- **dj ignores every verdict except halt.** Parity before the place asked for is not what
  was asked about (Developer). No coverage is held for those frames.
- **A halt in pre-roll** is recorded as a failure at the frame it happened in. The place
  asked for is not checked. `dj parity` reports that segment as not checked, with the
  halt's reason, and counts it as failed.

What it costs: in the GUI the pre-roll frames run unseen before anything is shown. On
the N64 that is about a second for 60 frames.


### 7. Input

Unchanged. dj plays input by frame to both cores through libretro (Developer,
2026-11-13). How the game polls input is the comparison core's business. dj records no
controller reads and no connector needs a hook for them.

### 8. The picture

- **The core draws when it chooses** (Developer asked; Thread's answer). It calls the
  ordinary libretro video callback whenever it has finished a picture, during any event.
  A core that can draw from a copied state may do so right after copying.
- **dj shows the latest picture when the baseline's frame ends.** If the core finished
  none in that frame the old one stays; if it finished two the last wins.
- **dj never asks for a picture.** A port can only draw when its own logic gets there.
- **The picture is for display.** No verdict depends on it.
- **A core may take as long as it needs** (Developer: one with advanced graphics should
  be able to, when stepping one frame at a time). The event call blocks, so the step
  ends when the core returns. The GUI's watchdog writes its "stopped responding" note
  after 10 seconds and removes it when frames come back.
- **dj says whether a picture is wanted,** through libretro's own
  `GET_AUDIO_VIDEO_ENABLE`, answered per core from the capture flag dj already sets.
  Today dj always answers yes (`player/src/libretro.rs`). A core with costly graphics can
  then skip drawing during a seek, pre-roll, or `dj parity`. It may ignore the answer.
- **While playing, a slow comparison core slows both,** because the baseline waits for
  it. If the baseline should keep its speed, the GUI can ask for a picture less often.
  That is a GUI setting, not part of the contract.
- **After a jump the old picture is shown faded** until the core has drawn a new one.

### 9. Failures and re-running them

- **A failure records** the segment and frame, the breakpoint's name or "frame end", the
  reasons, the coverage (section 5), the build, and the segment and frame of the last
  jump before it.
- **The last jump takes the place of "where the core was synced".** dj knows it without
  asking the core, and the existing columns hold it. The breakpoint's name is one new
  column in `parity_failures`.
- **To re-run:** put the baseline at the jump's frame by any route, send the jump there,
  and play to the failing frame, with the same frames marked as pre-roll. The core gets
  the same events and so makes the same choices. This holds because the jump's frame
  does not depend on which snapshots a machine has (section 6).
- **`dj parity --explain` stops its search at the event.** It looks for the baseline's
  last write to each differing address. The baseline runs on to the frame end after a
  mismatch, so dj notes the trace position when the verdict arrives.
- **The tools show the breakpoint's name** next to the frame: `dj parity`, the MCP parity
  tools and the GUI's failure list.

### 10. The same emulator as the comparison core

Unchanged from today, and outside the event design. A `[[compare]]` entry with
`sync = "state"` loads the baseline's save state after a jump, runs each frame with
`retro_run`, and dj compares system RAM at the frame end. It gets no events, has no
breakpoints of its own, and needs no pre-roll.

- **A `[RESET]` line** is `retro_reset` on it, as now.
- **A write at an instruction address** is armed in its connector as well as the
  baseline's, so both emulators take it at the same instruction.

Two builds of one emulator on one ROM that agree at every frame end are very unlikely to
have differed inside a frame, so nothing is compared there.

### 11. Where the comparison core runs

Decided by the developer 2026-11-13: in dj's process, in the GUI and in `dj parity`. A
breakpoint in the middle of a frame then needs no copy of the baseline. This reverses
step 2 of `ISOLATION_PLAN.md`.

What that needs:

- **The callers move.** `Checker`, `check_span`, the GUI and `dj parity` are written
  against `RemoteComparison` today. Most of `Checker` goes, since dj no longer tracks
  whether the core is in step.
- **The crash note names the comparison core,** and holds both calls in progress, since
  the core runs inside the baseline's `retro_run`. (Developer: worth implementing.)
- **A crash or hang in the comparison core under `dj parity`** (Thread). Today it is a
  failure at that frame, and `docs/connectors.md` promises that for CI. In dj's process
  it kills the batch child, and the child's results for that segment die with it. So:
  - **The batch child writes the crash note and runs the watchdog that the GUI has**
    (`player/src/crash.rs`). The note already names the call in progress, and will name
    the comparison core. The supervisor reads it to learn which core died or hung.
  - **The supervisor starts the child again and tells it the segment and frame.** The
    new child checks that segment again up to the frame before, so the passes are kept,
    then writes the failure at that frame and goes on.
  - **Segments below it are not blocked.** The baseline is sound.
- **OpenGL contexts must nest.** A core that draws with OpenGL is entered while the
  baseline's context is current. On macOS leaving puts the earlier context back;
  elsewhere it makes none current (`player/src/gl.rs`), so the baseline would go on
  without one.
- **A worker was looked at again and turned down** (Developer, 2026-11-15). It adds too
  much overhead, and the design is still changing in large ways.

### 12. The API and the config

Changed in place; dj is not released. Thread: raise the version number so an older core
gets today's "rebuild the core" warning.

| Side | Goes | Comes |
| --- | --- | --- |
| Baseline connector | `dj_set_tick_exec`, `dj_get_tick_count`, the tick count in `dj_clear` | One function to set the breakpoint addresses and the function to call at each. One function to read registers |
| Comparison core | `dj_sync`, `DJ_SYNC_BOOT`, `dj_write`, `dj_run`, `dj_check_parity`, `get_proc` in the handle | `dj_breakpoints`, `dj_handle_event`, and the optional `dj_handle_write`. `dj_parity_diffs` stays, for what differed |
| `[[compare]]` in `config.toml` | The value `sync = "dj_sync"`, which names a function that goes | `preroll = N`, 0 if left out. Thread: the default value of `sync` becomes `"events"` |
| Segment format | | `[WRITE]` with `@REGION:0xOFFSET` |

- **All three connectors change:** Mesen, Mupen64Plus and mGBA. mGBA's exports the tick
  functions today; after the version bump an unchanged one is refused and the project
  loses its coverage.
- **How a connector matches addresses** (Thread). A list compared on every instruction
  is costly on the N64, where a core registers every breakpoint it may want. The
  connector keeps a map of its own, one bit per byte of code, set where something is
  armed, and tests it on each instruction. Not a spare bit in the coverage flags: the
  recorder stores whole flag bytes in the database, and "did anything run" tests for a
  byte that is not zero, so an armed address would be stored as coverage.
- **Each struct starts with its own size,** so a field such as "parity checking is on"
  can be added later.

**A helper header, shipped with dj and not part of the API** (Thread). Every core now
writes its own "copy the baseline after a jump, answer unjudged, then compare" logic. For
a core that keeps the original's RAM layout, as the Tetris decomp does, that is the same
few lines each time. The header does it, and copies on the jump event when the core
judges at frame ends.

## Notes for a core's author

These belong in `docs/connectors.md` when this is built.

- **The core runs one step behind.** It is told where the baseline has got to, then runs
  itself to that point.
- **A value read through the handle is the baseline's value at that event.** It is the
  value the game used only if a breakpoint sits where the game uses it. Each place where
  timing-dependent data enters the logic needs one.
- **Input when a tick spans two frames.** Put a breakpoint in the game's input routine,
  just after the raw bytes are final and before it works out pressed and released
  buttons. Copy the controller buffer through the handle there and answer "pending". The
  core's own version of the rest of the routine is then still tested.
- **A lag frame cannot always be judged.** On one, the interrupt handler runs in the
  middle of a tick, on whichever instruction was running. A port is the same logic
  compiled for another CPU and cannot place it. Where the handler changes something the
  interrupted logic reads afterwards, a port can differ without being wrong, and the
  core answers "unjudged". Thwaite had 7 such frames and Tetris 15.
- **State that follows emulator timing cannot be reproduced.** No Mercy's idle thread
  advances the random number generator. That needs a patch to the ROM, which is a
  separate feature: see the roadmap, "Pin timing-dependent randomness".

## What was measured

On dj's own connectors, before this design.

| Game | Result |
| --- | --- |
| Thwaite (NES, Mesen) | The game waits once per frame in 11,013 of 11,020 frames after boot; 7 lag frames |
| Tetris (NES, Mesen) | 30 segments, 24,392 frames, 15 lag frames |
| No Mercy singles match (N64) | 467 ticks in 934 frames. 1 tick started in one frame and finished in the next |
| No Mercy tag match, four CPUs (N64) | 303 ticks in 607 frames. 162 ticks (53%) started in one frame and finished in the next |

- **Mesen save states replay byte for byte.** Tick positions were the same from power-on
  and from snapshots.
- **N64 overlays reuse addresses.** `match_tick`'s address ran in 84 menu frames as other
  code.
- **Loads have no ticks.** No Mercy has stretches of up to 209 frames with no game-loop
  pass.
- **A spin loop is a bad place for a breakpoint.** In Thwaite's boot one loop ran its
  compare 2,693 times in a frame.

## Not in this plan

- **Patches to the ROM.** A separate feature, in the roadmap.
- **Named addresses in the project config,** for writes and for `[tick]` to refer to. Set
  aside by the developer, maybe for later.
- **Ticks on the timeline,** and a record of which frames had a tick. Display work.
- **A rebuilt ROM that does not match, run in an emulator.** No answer.

## What this costs

- **Every core writes its own copy-and-compare logic.** The helper header covers the
  common case.
- **dj cannot say whether the core is live.** It shows the last verdict.
- **Coverage is credited by whole frames** (section 5).
- **A core that cannot copy between frames needs pre-roll set by hand,** and pre-roll
  costs time on every jump (section 6).
- **The last frame or two of a segment are never credited** for such a core (section
  5).

## Earlier decisions this changed

Both were the developer's, made under an earlier design in which dj passed every
breakpoint to the core. The developer chose the new form on 2026-11-15.

- **"Yields live in the project config. They are facts about the game."** Now the core
  provides its breakpoints. `[tick]` alone stays in the config, because dj uses it with
  no core loaded.
- **"The core subscribing to yields" was dropped.** Now the core's list is the
  subscription.

## Open problems

Decided since the last revision, and no longer open: the names, the default pre-roll,
the tail of a segment, a reason on "unjudged", a dropped write, and the worker.

1. **The N64 connector has no breakpoints.** The game runs on a coroutine whose stack may
   be too small for the call. The connector can switch to the main coroutine to make it.
   Not checked: whether anything treats that switch as the end of the frame.
2. **The layout of the registers** for each instruction set. An N64 core needs them to
   find the running thread.
3. **What the handle's memory looks like on the N64.** Mupen64Plus keeps RDRAM as
   host-order 32-bit words. The handle must say, or hand out bytes in the CPU's order.
4. **OpenGL contexts nesting** (section 11). Not tried.
5. **A core that corrupts memory before it crashes,** dj's or the baseline's. No answer.
6. **dj's own tick count on the N64** counts other code at the `[tick]` address in
   another overlay. A project should pick an address in code that is always loaded.
7. **A fixed list of breakpoints and overlays** (section 1): whether "pending" is enough.
8. **Writing the baseline's memory inside a frame** (section 3). Not tried on any
   connector. To check on each: that the write takes effect at once, and that state
   hashes and save states stay repeatable.
9. **`ROADMAP.md`, `ISOLATION_PLAN.md` and the docs** describe earlier designs.

## Build order

Thread. Both spikes come before the move into dj's process, which gives up a shipped
protection.

1. **A spike in the Mesen connector.** One breakpoint at Thwaite's game wait and the
   blocking call, with no comparison core. Done when the hits and RAM hashes are the
   same across two runs and after loading a snapshot, state hashes are the same as
   without the breakpoint, and the cost per hit is measured.
2. **A spike in the Mupen64Plus connector.** One breakpoint at No Mercy's `match_tick`.
   Done when the hit count is the same from power-on and from a snapshot, a load fires
   none, and state hashes are unchanged. If this cannot be made to work, stop and
   rethink.
3. **The new connector API in all three connectors,** with dj counting ticks itself.
   Done when Thwaite's tick counts equal those of a
   plain `dj run` today, with the GUI and a parity check running, and mGBA still reports
   coverage.
4. **Move the comparison core into dj's process,** with the crash note, the supervisor's
   handling of a comparison core that dies, and nested OpenGL contexts. Done when
   `tests/compare.rs` passes without a worker, and a core that crashes on purpose is
   named in the GUI's note and recorded as a failure at its frame by `dj parity`, with
   its earlier passes kept and the segments below it still checked.
5. **Writes at an instruction address** (section 3). After step 4, because the
   same-emulator comparison needs the write armed in its connector too, and before step
   4 that core is in a worker. Done when a Thwaite segment with such a line plays to the
   same hash twice and from a snapshot, a line whose address is not reached is dropped
   with its warning, and the same-emulator comparison stays in parity across the write.
6. **`dj_handle_event` and `dj_handle_write`,** in the test core (`player/tests/testcore`) and two stubs: one
   that copies the baseline at every event and answers match, one that diverges on
   purpose. Done when each event and each verdict has been seen; a run of mismatches has
   recorded one failure; playback has paused at the first mismatch of a run and not at the later ones; a halt has
   been resumed; and neither an unjudged stretch nor the frames between a jump and the
   copy have credited any coverage. The cases in STATES.md, sections 3 and 4, are the
   test list.
7. **The helper header, and the Tetris decomp on it,** at the frame end. Done when its 30
   segments pass as they do today, the first frame after a jump included.
8. **The Tetris decomp at a breakpoint inside the frame,** at its game wait. It is the
   only real decomp at hand, and its 15 lag frames test where a core must answer
   "unjudged". Done when its 30 segments pass, with the lag frames unjudged or
   explained.
9. **Pre-roll** (section 6), with the Tetris decomp from step 8 and `preroll` set on its
   entry. Done when `dj parity` checks frame 0 of every segment that is not a root, and a
   failure found after a pre-roll re-runs to the same frame with the local database
   emptied.
10. **N64 breakpoints in full,** `match_tick` and the input routine, with a stub core
    that copies the pad buffer. Done when both count the same from power-on and from a
    snapshot.
11. **Failures and the tools** (section 9). Done when a failure at a breakpoint inside a
    frame can be re-run and explained from the command line.
12. **The docs:** `connectors.md`, `segments.md`, `config.md`, `gui.md`, `cli.md`, and
    the notes on `ROADMAP.md` and `ISOLATION_PLAN.md`.

A real decomp of a game whose ticks cross frames is the last test, and the only one that
proves the design. None is at hand as a libretro core.
