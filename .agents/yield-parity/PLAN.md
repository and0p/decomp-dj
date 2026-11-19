# Plan: parity by breakpoints

Written 2026-11-15 15:20 EST, from a conversation with the developer that day. The
developer asked for the design to be looked at afresh, with the comparison core doing
more and dj less. All twelve build steps are done: the two connector spikes ("Spike
results"), and the new connector API, the comparison core in dj's process, writes at an
instruction, events, the helper header, the Tetris decomp on events, pre-roll, N64
breakpoints with a stub core, failures and the tools, and the docs ("Built so far"). What
is left is under "Open problems", and the test that proves the design, a real decomp of a
game whose ticks cross frames, has not been made.

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
- 19:36 EST: the Mesen and Mupen64Plus spikes were run and both pass. Their results
  are under "Spike results"; the open problems and the build order are updated.
- 20:46 EST: the N64 save-storage problem the spike exposed is fixed in the connector
  patch.
- 22:40 EST: build step 3 is built: connector API 3 in all three connectors, with dj
  counting ticks itself.
- 22:54 EST: build step 4 is built: the comparison core runs in dj's process.
- 23:20 EST: build steps 5 and 6 are built: writes at an instruction, and events.
- 23:58 EST: build steps 7 and 8 are built: the helper header, the Tetris decomp on
  events (at its wait, and at frame ends), and the older comparison functions removed
  from dj. Tetris's 15 "lag frames" turned out to be long work with NMIs off, all judged.
- 2026-11-16 08:27 EST: build step 9 is built: pre-roll, in `dj parity`, in re-running a
  failure, and in the GUI's seek inside a segment.
- 2026-11-16 11:43 EST: the GUI's pre-roll reaches into the segments above, unseen
  ("Built so far", step 9).
- 2026-11-16 14:08 EST: build step 10 is built: a stub comparison core for No Mercy with
  two breakpoints. It found that No Mercy's `[cache] exclude` breaks stored snapshots.
- 2026-11-16 18:10 EST: why No Mercy's `[cache] exclude` broke stored snapshots: its range
  was 2 KB too wide and covered the end of the game's heap ("Built so far", step 10).
- 2026-11-16 15:44 EST: the Mupen64Plus connector has a trace hook, and `--explain` names
  writers on the N64 ("Built so far", after step 11).
- 2026-11-16 14:55 EST: build steps 11 and 12 are built: a failure says which event it
  answered, `--explain` stops its search there, and the docs are brought up to date.
- 2026-11-16 23:47 EST: two saves with no frame between them (open problem 8) no longer
  move a frame's state hash: dj gives back the state it last saved or loaded.

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
  already forces. The spikes checked both: the write took effect in its own frame,
  repeated exactly, and survived a snapshot taken after it. mGBA is not checked.
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
| `[[compare]]` in `config.toml` | The value `sync = "dj_sync"`, which names a function that goes | `preroll = N`, 0 if left out. The default value of `sync` is `"events"` (built in step 7) |
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

**A helper header, shipped with dj and not part of the API** (Thread; built in step 7 as
`connectors/include/dj_compare.h`). Every core now
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
  core answers "unjudged". Thwaite had 7 frames with no game wait. Tetris had 15, and step
  8 showed none of them is such a frame: all are long work with NMIs off, where no handler
  runs, and they are judged in full at the wait that follows ("Built so far").
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

## Built so far

**Step 3, 2026-11-15: connector API 3.** In `player/src/libretro.rs` and the three
connector patches; described in `docs/connectors.md`.

- **`dj_set_breakpoints`** takes a list of places and one function. The connector calls it,
  and waits, when an instruction at one is about to run. Returning non-zero switches that
  breakpoint off until the list is set again.
- **`dj_set_tick_exec` and `dj_get_tick_count` are gone.** `[tick]` is a breakpoint and dj
  counts the calls. `dj_clear` no longer touches a count.
- **The cap is 1,024 calls in one frame** (`BREAK_CAP`). Past it dj switches the breakpoint
  off, warns once, and sets the list again at the next frame.
- **Hits while a state is saved or loaded are ignored** by dj, and held back by the
  Mupen64Plus connector.
- **Each connector keeps a bit per byte for breakpoints,** apart from the coverage flags.
- **`dj_read_registers`** exists in all three, with a first layout per instruction set.
  Nothing in dj reads it yet.
- **The N64 has ticks now.** Its connector's tick functions were stubs.

Checked:

| Check | Result |
| --- | --- |
| Thwaite and Tetris, every segment's hashes and ticks against the old cores | identical |
| gba-test against the old core | identical |
| Ticks with the coverage flags cleared after every frame, as the GUI and a parity check do | the same as played straight: 845 (Thwaite), 35 (gba-test), 588 (No Mercy) |
| The cap, with `[tick]` in Thwaite's boot spin loop | switched off at 1,024 in the frame, warned, back on the next frame |
| The test suite, and the GUI's build | pass |

- **Thwaite, Tetris and gba-test are repinned** to the new builds. No Mercy is not: its ROM
  is outside what the thread could read.
- **`player/examples/tick_check.rs`** is the tool that made the tick comparison. (Removed
  2026-11-17 in the release clean-up; in git history up to commit `f5bfd2f`.)
- **Only dj's own `[tick]` breakpoint is armed so far.** A comparison core's breakpoints
  come with step 6.
- **The spike cores no longer load their connector:** they speak API 2. The commands under
  "Where the spike code is" need the spike patches carried over to API 3 to run again.

**Steps 5 and 6, 2026-11-15: writes at an instruction, and events.**

- **A `[WRITE]` line may end `@REGION:0xOFFSET`** (`player/src/format.rs`). The player arms
  it for its frame (`libretro::Core::arm_write`); the write is made from inside the
  connector's breakpoint call, the first time that instruction is about to run; if it
  doesn't run in the frame, the write is dropped with a console warning.
- **A comparison core that is the same emulator** arms the same write in its own connector.
- **`dj_breakpoints`, `dj_handle_event` and `dj_handle_write`** are read from a comparison
  core, with the version 2 handle (`memory`, `registers`), as sections 1 to 3 give them.
  A core with `dj_handle_event` is driven by events; any other is driven as before.
- **The older functions were still read** (`dj_sync`, `dj_write`, `dj_run`,
  `dj_check_parity`) until step 7, which removed them.
- **`parity::Checker`** has the state STATES.md lists: the jump owed, halted, the last
  verdict, the pile, the frame's claim. `Checker::begin` is new and is called before the
  baseline's frame; it sets a hook that `libretro.rs` calls at the frame's start, at each
  of the core's breakpoints and at each write made inside the frame. `Checker::step`, after
  the frame, sends the frame end. It now takes the frame's coverage flags itself and hands
  back what was credited and what a failure blames.
- **`dj parity` and the GUI** call `begin`, and use what `step` hands back. In the GUI a
  mismatch pauses once; playing on leaves a core that takes events as it is.
- **The test core has a small connector** (`-DCONNECTOR`) and an event mode (`-DEVENTS`),
  so breakpoints, coverage and events are tested without a ROM.

Checked:

| Check | Result |
| --- | --- |
| A core that judges at frame ends (`tests/events.rs`) | 40 of 40 frames pass, the first after the jump included |
| A core that judges at a tick inside the frame | no failure; the frame before its copy is credited to nothing; a lag frame is unjudged; one tail frame |
| A run of mismatches | one failure at frame 9; frames 10 to 12 shown and not kept; Sync now, or the core itself, restores parity |
| A halt | a failure; the next frame sends a jump, "copy again", and passes |
| An unjudged stretch | 4 frames skipped, and the code only they ran is not credited |
| Writes: between frames, at an instruction, at an instruction that doesn't run, with and without `dj_handle_write` | each as STATES.md says |
| A console reset, for a core at a frame boundary and one at a tick | the first stays in parity; the second starts again |
| Ticks counted by dj, and the cap | 35 of 40 frames; capped at 1,024 a frame |
| Thwaite on Mesen, two writes at the tick (`PRG:0x197`) | the same hashes from scratch, again, and from loaded states |
| Thwaite, a write at an instruction that never runs | dropped with the warning; the same end hash as no write |
| Thwaite against the same emulator across those writes | in parity, 300 of 300 frames |
| Tetris against its decomp, on the older functions | 30 segments, 24,392 frames, 0 failed |
| The whole test suite, and the GUI's build | pass |

What this turned up:

- **A lag frame costs a tick core four frames of credit, not one.** Coverage is credited
  by whole frames, one verdict late. In the test core, with a lag frame every seventh
  frame, 18 of 40 frames are credited and 21 are not. The frame before the lag frame and
  the lag frame are still held when the tick after it is unjudged; that frame is claimed;
  and the core copies the baseline in the next. Section 5 calls this conservative; this is
  its size.
- **`dj run` panics on a segment id shorter than eight characters** (`player/src/main.rs`,
  an older bug, not fixed here).

Not done, or not checked:

- **Pre-roll** (step 9): the flag was always off then. Built since: see step 9 below.
- **A failure does not record the breakpoint's name** in a column of its own (step 11). It
  is in the failure's message.
- **`--explain` still searches to the frame's end** (step 11).
- **The GUI was not run.** Its Sync now sends a jump with the reason "replaced state", not
  "copy again".
- **A write at an instruction on the N64 or the GBA** was not tried. On the N64 the bytes
  would go into memory kept as host-order words.
- **`sync = "events"`** in `[[compare]]` is not added: a core with `dj_handle_event` is
  recognised by that function.

**Step 4, 2026-11-15: the comparison core in dj's process.** The comparison still works
frame by frame, as before: `dj_sync`, `dj_run`, `dj_check_parity`. Only where it runs has
changed. Events and verdicts come with step 6.

- **`parity::Checker`, `check_span`, `dj parity` and the GUI use `compare::Comparison`,**
  loaded into the second core slot of their own process. The handle points at the
  baseline itself; there is no copy of the baseline any more.
- **The crash note** (`player/src/crash.rs`) names the comparison core when one is loaded,
  and the call it was in when it was the one running: `retro_run`, `dj_sync`, `dj_write`,
  `dj_check_parity`, `dj_parity_diffs`, `dj_run`. The baseline's call is kept beside it, so
  both show when the comparison core is entered inside the baseline's frame.
- **Under `dj parity`** the batch child writes that note too
  (`.local/crash-batch-<pid>.txt`), and runs the watchdog, so a hang leaves one as well.
  The supervisor (`isolate::supervise`) reads it. If the comparison core was running, it
  starts the child again and tells it the segment and frame (`DJ_BATCH_COMPARE_DIED`). The
  new child checks that segment up to the frame before, which keeps the passes, records a
  failure at the frame, and goes on. The segments below are still checked. A second death
  in the same segment ends the command with an error.
- **OpenGL contexts nest** (`player/src/gl.rs`). Leaving a context puts back the one that
  was current before it, when that was another core's. macOS already did this; the EGL
  code now does too.
- **`RemoteComparison` and the `compare` worker stay in the code,** used only by the tests,
  so going back is cheap. The GUI's "Restart comparison core" is gone. Since step 7 the
  worker serves `sync = "state"` only: a core that takes events is refused there.

Checked:

| Check | Result |
| --- | --- |
| `tests/compare.rs`, with the span tests in process | pass |
| A comparison core that crashes, crashes in `dj_check_parity`, or hangs, under `dj parity` (new test) | each a failure at frame 9, named as the comparison core with its call, the nine passes kept, the segment below checked and passing, no note left behind |
| GL contexts nesting (new test) | pass on macOS |
| Thwaite against the same emulator, in process | 12 segments, 11,140 frames, 0 failed |
| Tetris against its decomp, in process | 30 segments, 24,392 frames, 0 failed |
| The whole test suite, and the GUI's build | pass |

Not checked:

- **The EGL nesting code has only been type-checked,** by building it on macOS with the
  platform switch flipped. It has not run on Linux.
- **The GUI was not run.** Its note is the same code as the batch child's, which the new
  test exercises through a real crash.
- **`dj parity --rerun` and `--explain`** do not handle a comparison core that dies: the
  second death ends the command with an error.

**Steps 7 and 8, 2026-11-15: the helper header, and the Tetris decomp on events.**

- **`connectors/include/dj_compare.h`** (MIT, like the decoder header; packaged with the
  connectors). It declares the API, and has the helper: `struct dj_helper` with `copy`,
  `run`, `compare` and an optional `reset`, and `dj_helper_event`. `judge_at` is 0 for
  frame ends or a breakpoint's id. Judging at frame ends it copies on the jump; judging at
  a breakpoint it waits for it, but offers a power-on jump to `copy`, so a core can boot
  itself. Described in `docs/connectors.md`, "The helper".
- **The Tetris decomp takes events** (`examples/tetris/.decomp/libretro/tetris_libretro.c`),
  on the helper. Its core option `tetris_judge` picks where it judges.
  - **At its wait** (the default). Breakpoints at `PRG:0x2A36` and `PRG:0x2A4C`, the NOP in
    each wait routine after `verticalBlankingInterval` is cleared and before the loop that
    polls it. Both carry the same id. Each is reached once per wait, so the cap never
    matters. The decomp runs one tick there and every byte it keeps is compared.
  - **A third breakpoint at the NMI** (`PRG:0x0005`) takes the controllers. A tick that
    draws a screen with NMIs off reaches its wait a frame or two after its NMI, when dj
    has set later frames' input. It also counts NMIs per tick, which is how a real lag
    frame would be seen.
  - **At frame ends.** One tick per frame, as before.
- **The older comparison functions are gone from dj:** `dj_sync`, `DJ_SYNC_BOOT`,
  `dj_write`, `dj_run`, `dj_check_parity` and the handle with `get_proc`. `sync` in a
  `[[compare]]` entry is `"events"` (the default; the core must have `dj_handle_event`) or
  `"state"`. `"dj_sync"` is no longer a value: a config that has it does not open.
  `Checked::NeedsSync` went with them. The test core takes events in every build.
- **Docs changed with it:** `docs/connectors.md` ("Comparison cores" rewritten around
  events), `docs/config.md`, `docs/gui.md` (the lines that named the old functions), the
  connectors README and the Tetris decomp's README.

Checked:

| Check | Result |
| --- | --- |
| Tetris, judging at frame ends | 30 segments, 24,392 frames, all passed, 0 failed. The first frame after each jump is checked |
| Tetris, judging at its wait | 30 segments, 0 failed. 24,362 frames counted: each segment's last frame is its tail. 29 frames unjudged, the first of each segment that is not a root (the copy). In the report: 4,101 instructions with parity, 0 not checked |
| The 15 frames with no wait | 11 ticks that draw a screen with NMIs off and lose one frame, and 2 frames at each of the 2 power-ons. Each tick had at most one NMI. All are compared in full at the wait that follows; none is unjudged |
| The decomp broken on purpose (the pad at the NMI set to zero) | a failure at frame 31 of "Cycle through game types", named at the breakpoint, with the bytes that differ |
| Segments with `[WRITE]` and `[RESET]` lines | pass in both modes; a write between frames did not cost a copy |
| `tests/compare.rs` and `tests/events.rs` on the event-only API, the whole suite, the GUI's build | pass |

Not checked, and what this leaves:

- **No real lag frame exists in Tetris,** so "unjudged on a lag frame" has only been seen
  in the test core. The decomp's path for one (more than one NMI in a tick: copy at that
  wait, answer unjudged) has never run.
- **Frame 0 of a segment that is not a root is unjudged when judging at the wait.** That
  is what pre-roll (step 9) is for.
- **Judging at the wait is no faster or slower:** about 51 s for the 30 segments either way.
- **The GUI was not run** with the decomp in either mode.
- **The Thwaite decomp keeps its own copy of the API** (`examples/thwaite/.decomp/src/dj_api.h`),
  written to be replaced by dj's header. Not changed here.

**Step 9, 2026-11-16: pre-roll** (section 6).

- **`preroll = N` in a `[[compare]]` entry,** 0 if left out, and only for a core that takes
  events. Described in `docs/config.md`, `docs/connectors.md` ("Pre-roll"), `docs/cli.md`
  and `docs/gui.md`.
- **`parity::preroll_route`** gives the frames to pre-roll before a place: up to N, back
  through the segments above, stopping at a root's start.
- **`Checker::set_preroll(frames, for_segment)`** marks the next frames as pre-roll. It is
  called before the sync that owes the jump. In those frames every event carries
  `DJ_PREROLL`; only a halt counts; the frame's coverage is taken and dropped, never piled
  or counted; and when the last one ends the last verdict is "none" and the pile is empty.
  The checker keeps how many frames from the jump were pre-roll.
- **`dj parity`** seeks to the first pre-roll frame (the nearest stored snapshot at or
  before it in that segment, or the segment's start), plays the route with the checker in
  pre-roll, goes on into the segment as playback goes from a parent into its child, and
  checks it. A halt in pre-roll: the segment is "FAILED, not checked: in pre-roll", the
  failure is at the frame it happened in, and its run row is marked failed.
- **A failure records its pre-roll:** a new column, `parity_failures.preroll` (added to an
  older database when it is opened). `--rerun`, `--explain` and the GUI's re-run mark the
  same frames. Reports and JSON show it.
- **A comparison core that dies in pre-roll** is reported against the segment the pre-roll
  is for, at frame 0, so the supervisor doesn't blame the parent. The next child then
  checks nothing of that segment and runs no pre-roll for it.
- **In the GUI,** a seek with Check parity on, for a core with `preroll`, restarts from a
  snapshot at or before the frame the jump goes at, sends the jump there, and runs the
  frames up to the one shown in pre-roll.
- **The Tetris example sets `preroll = 3`.** Its longest stretch with no wait is two
  frames.

Checked:

| Check | Result |
| --- | --- |
| Tetris, judging at its wait, `preroll = 3` | 30 segments, 0 failed, no frame skipped: frame 0 of all 29 segments that are not roots is judged. 24,362 frames counted (each segment's last frame is its tail). 4,101 instructions with parity, 0 not checked |
| Tetris, judging at frame ends, `preroll = 3` | 30 segments, 24,392 frames, 0 failed: nothing changes for a core that needs none |
| A failure after a pre-roll, with the decomp broken on purpose | found at frame 28 of "Cycle through music options", synced at frame 57 of its parent with 3 frames of pre-roll. `--rerun` fails again at the same frame; `--explain` traces it |
| The same check in a copy of the project with an empty local database | the same failure: same frame, same place synced, same bytes |
| The test core (`tests/events.rs`, `pre_roll`) | the route across one and two segments; frame 0 judged with pre-roll and not without; a mismatch in pre-roll ignored, and the first one after it kept as a failure, not as "drifting"; a halt in pre-roll a failure at its own frame |
| `dj parity` end to end (`tests/compare.rs`) | a child passes after a pre-roll in its parent; a halt in pre-roll reports the child as not checked, with the failure at the parent's frame and its pre-roll in the JSON |
| The whole test suite, and the GUI's build and tests | pass |

Not checked, and what this leaves:

- **The GUI's pre-roll was run without a window only** (see below). Nothing drove the
  window itself.
- **"Pre-roll" is not shown as a state** beside the comparison core's name. The status bar
  shows "Pre-roll for frame N" while the frames above run.
- **An ancestor's snapshot is not used** when the first pre-roll frame's segment has none
  at or before it: dj plays from that segment's start. The jump is at the same frame
  either way.
- **Cost under `dj parity`:** about 56 s for Tetris's 30 segments against 51 s, most of it
  getting the baseline to the frame before each segment.
- **A lag frame inside the pre-roll can use it up.** In the test core, three frames of
  pre-roll with a lag frame in the middle left the core stale at the place asked for. The
  number has to cover the longest stretch with no tick, plus one.

**The GUI's pre-roll in the segments above** (added 2026-11-16 11:43 EST, at the
developer's request). The GUI's playback holds one segment, so a seek near a segment's
start had too few frames before it. Now:

- **When.** A seek that is a jump for the comparison core, to a frame nearer the start
  than N, with Check parity on and no failure being re-run. Selecting a segment is one;
  so are looping, stepping back to the start, and loading a comparison core while paused
  there.
- **What runs.** A job (`Job::Preroll`) takes `preroll_route`'s spans above the segment
  shown. The baseline goes to the first span's segment, loads its nearest stored snapshot
  at or before the jump frame, plays to the jump frame with nothing sent, then plays the
  pre-roll frames through the checker as `dj parity` does, into each segment down
  (`Checker::enter`), and comes out at frame 0 of the segment shown. The rest of the seek
  is the ordinary one, with the checker in step.
- **Nothing shown changes.** The playback, its snapshots, the roll and the picture are
  left as they are. The frames above are not drawn and their coverage is dropped.
- **The check at the boundary.** The state at the end of the parent must hash to the
  state the playback started from. If not (an edit above), the pre-roll is given up.
- **The snapshot that makes it cheap.** When reaching the jump frame took 30 frames or
  more, the state there is stored under the parent's content key, like a `diff_every`
  snapshot. The next pre-roll there loads it.
- **Giving up.** The baseline loads the playback's snapshot again and the core copies at
  the frame shown, as with no pre-roll. Cases: the parent's own start isn't stored, the
  boundary check fails, an error. A seek while the job runs cancels it the same way.
- **A halt in those frames** is recorded as a failure at the parent's frame and shown as
  the problem; the frame asked for is shown with the core left out of step until the next
  jump.

| Check (Tetris, `preroll = 3`, the GUI's code driven without a window) | Result |
|---|---|
| Selecting a segment whose parent has 444 frames | jump at frame 441 of the parent; the core in step at frame 1 of the segment; no problem |
| The same again | 44 ms against 652 ms (debug build): the parent's stored frames now have 441 |
| 40 frames played on from there | no failure; the pass still records |
| A seek back to frame 1 | jump at frame 442 of the parent |
| A seek to frame 30 | jump at frame 27 of the segment, as before |
| A pre-roll given up | synced at frame 0 of the segment |
| A pre-roll cut off halfway, then a seek | the baseline is back at the state it had |

The check was a test written for the purpose and removed: it needs a project and its ROM.
Not checked: a halt in the frames above, the boundary check failing, a parent whose start
isn't stored, and the window itself.

**Step 10, 2026-11-16: N64 breakpoints in full.**

Nothing in dj or the connector had to change: the breakpoints of step 3 and the events of
step 6 were enough. What is new is the core that uses them.

- **The stub core** is `connectors/mupen64plus/stub/nomercy_stub.c`, with a README. It is
  a comparison core with the game left out, on the helper header, judging at a
  breakpoint.
- **The input routine.** `pads_read` is at `0x80005818`, in the main segment, which is
  always loaded. It has one return, at `RDRAM:0x5BD0`, and the breakpoint is there: the
  pad buffer (`g_pads`, 48 bytes at `0x8009EDA0`) is complete, and the stub copies it.
  The addresses came from the No Mercy decomp's notes and a memory dump.
- **The tick.** A breakpoint at `match_tick` (`RDRAM:0x114514`). The stub counts it and
  answers "match" when the pads it copied are the ones in the game's memory.
- **Other code at the tick's address** is told apart by the caller, not the stack
  pointer: the return address must be the match's one call (`0x800D9E04`), and the
  instruction at the call site must still be `jal match_tick`. That answers open problem
  6 for this game: "pending" is enough.
- **Byte order** (open problem 2). The stub reads bytes through whole host-order words.
  It is a few lines in the core; the handle was left as it is.

| Check (No Mercy, a project copy without `[cache] exclude`; see below) | Exhibition (2,136 frames) | Four-CPU tag match (1,800 frames) |
|---|---|---|
| Pad reads | 883 | 702 |
| Real ticks | 502 | 340 |
| Other code at the tick's address | 86 | 57 |
| Pad reads between two ticks | always 1 | always 1 |
| Ticks whose pad read was in an earlier frame | 0 | 1 |
| The stub's log from power-on against from the stored snapshot | identical, 1,472 lines | identical, 1,100 lines |
| `dj parity` | 1,013 passed, 1,121 skipped, 0 failed | 688 passed, 1,110 skipped, 0 failed |

- **The log** has a line for every pad read (the 48 bytes and the return address), every
  tick (its number and the pads) and every hit of other code (return address and stack
  pointer). "From power-on" is an empty local database, where dj plays the root segment
  first; "from the snapshot" is the run after it, which loads the root's stored end.
- **The skipped frames** are the menus and the load before the match, where there is no
  tick, and the frames up to the stub's first copy.
- **A fault made on purpose** (`NM_STUB_FAULT=100`) is found: a failure at frame 1332, at
  `match_tick`, with the 48 bytes of each side.

**Found on the way: No Mercy's `[cache] exclude` breaks its stored snapshots.**
`examples/nomercy/config.toml` has `[cache] exclude = ["RAM:0x38F000-0x3FFFFF"]`, the
three framebuffers, zeroed in stored snapshots. With the core as built now:

| Run of "start CPU vs CPU exhibition" | With the exclude | Without |
|---|---|---|
| From power-on | ok | ok |
| From the root's stored end | **changed**: another end hash | ok |

- **The zeroes are in the right place** in the state (checked by comparing states). The
  game's memory outside the framebuffers differs by frame 2 all the same, and the match
  never starts: no tick at all in 2,136 frames.
- **So the game, or the emulator on its behalf, reads those bytes.** Which was not
  found. The `[hash] exclude` of the same range is harmless: it only leaves bytes out of
  a hash.
- **Until it is understood, the project should not have `[cache] exclude`.** Done the
  same day, at the developer's word: the section is removed from the project's config and
  its local database, whose snapshots were stored with it, was deleted.
- **The project is repinned** to the core as built now (`dj.60c9c35b`, from
  `dj.4343e0a2`): all three segments reproduce their hashes from power-on.
- **The cause, found 2026-11-16 18:10 EST** by a session started for it, with the new
  trace hook: **the range starts 0x800 bytes too low.** Nothing reads the framebuffers
  back.
  - The three buffers are 3 × 320 × 240 × 2 = 0x70800 bytes and end at 0x400000, so they
    start at 0x38F800. The 2 KB below them, 0x38F000 to 0x38F7FF, are the end of the
    game's heap (2,234,368 bytes at 0x8016E000).
  - At the end of the root segment that tail holds the record of the graphics task being
    sent (an `OSTask` at 0x38F6E0), the last heap block's header (0x38F750) and a
    viewport and display list (0x38F760 to 0x38F7F7) that the frame's main display list
    points at.
  - A trace of the four frames after a load shows CPU reads at 0x38F6D8 to 0x38F72B (the
    task is copied by `mem_copy`, 0x80036710) and none at or above 0x38F800.
  - Zeroing 0x38F800 to 0x3FFFFF alone leaves memory outside the buffers identical for
    300 frames. Zeroing 0x38F6D0 to 0x38F75F alone makes it differ at the second frame.
    Zeroing in the state and zeroing in memory after the load give the same result, so
    loading is not involved.
  - With `[cache] exclude = ["RAM:0x38F800-0x3FFFFF"]`, both child segments replay from
    the stored root state with their recorded hashes. Checked again in this thread:
    "start CPU vs CPU exhibition" twice and "4 CPU match", all ok.
  - `docs/config.md` gave the too wide range as its example. It is corrected there, with
    how to work a range out.
  - The project's `[hash] exclude` has the same 2 KB too many, so its hashes ignore that
    tail. Narrowing it changes every recorded hash; not done.
  - `[cache] exclude` is back in the project's config with the corrected range, at the
    developer's word.
  - `[hash] exclude` is narrowed to the same range (2026-11-16, at the developer's word),
    and the three segments' hashes recorded again from power-on. They reproduce on a
    second run from power-on, and both children reproduce from the stored root state, so
    the 2 KB of heap are stable and now hashed. The root's start hash did not change.

Not checked, and what this leaves:

- **A failure's re-run and `--explain`** on the N64 are step 11.
- **The GUI** with this core was not run.
- **One game and two segments.** No segment here has a frame with two ticks.
- **A lag frame is still untested on a real game.** One tick in the tag match took its
  pads in an earlier frame, and it was judged like any other; the stub has no logic that
  an interrupt could split.
- **The guard against hits during a load** (open problem 9) still never fired.

**Step 11, 2026-11-16: failures and the tools** (section 9).

Re-running already worked at a breakpoint, on the NES and the N64, because a re-run sends
the same events. Three things were missing or wrong.

- **A failure says which event it answered.** `parity_failures` has a new column `at`:
  the breakpoint's name, "the frame end", "a write" or "the jump". It is empty for what
  was not a verdict (a failed sync, a core that died) and for rows kept before. `dj
  parity --report --json`, `--explain --json` and the MCP `parity_failures` tool give it
  as `at`. The reason's text already named it, so the GUI's list shows it unchanged.
- **`--explain` stops its search at the event.** When a verdict fails, dj notes how many
  trace events the baseline had made. The search for the last write to each differing
  address looks no further, and the output says where it stopped.
- **A diff with an address and a symbol is traced by its address.** It was reported as
  "given by a data symbol" and not traced.
- **The Tetris decomp has `dj_parity_diffs`:** one difference per differing byte of RAM
  (the first 32), so `--explain` has addresses to trace. Before, it gave only text.

| Check | Result |
|---|---|
| Tetris, a copy of the decomp broken on purpose (`player1_fallTimer` counts by 2), judged at its wait | 21 of 30 segments fail. Each failure is "at updateAudioWaitForNmiAndResetOamStaging, before its wait" |
| `--rerun` of one (frame 1220 of "Watch demo", synced 3 frames up in its parent, as pre-roll) | fails again at the same frame |
| `--explain` of it | 164,354 events traced over frames 1210 to 1220; the search stops 154,999 events in; `$0045` and `$0065` each get their writer in frame 1220 (`STA $40,X` at `$878A`, `STA $60,X` at `$87B2`) |
| The N64 stub with its fault (`NM_STUB_FAULT=100`) | failure at frame 1332, `at` = "match_tick". `--rerun` with the fault fails again at the same frame; without it, it passes |
| `--explain` on the N64 | plays it again and lists what differed. No writers: the Mupen64Plus connector has no trace hook |
| The test core (`tests/events.rs`, `tests/compare.rs`) | a mismatch at a frame end records "the frame end"; a halt at its breakpoint records "tick_end"; the JSON of `--report` and `--explain` carries `at` |
| Tetris with the real decomp, after it gained `dj_parity_diffs` | 30 segments, 0 failed, judged at its wait and at frame ends |
| The player's test suite, and the GUI's build and tests | pass |

Not checked, and what this leaves:

- **Whether the stop changes an answer** was not shown with a case where the original
  writes the same address again after the breakpoint. The position is right (it is
  inside the failing frame's events); a test needs a core with a trace hook, and the
  test core has none.
- **No trace hook on the GBA,** so `--explain` names no writers there. The N64 got one
  the same day (below).
- **The GUI** shows the event in the failure's message only; it was not run.

**After step 11, 2026-11-16 15:44 EST: a trace hook in the Mupen64Plus connector** (at the
developer's request), so that `--explain` names writers on the N64.

- **The connector** records an event per instruction (in `dj_exec`) and per byte the CPU
  reads or writes (in the four aligned read and write functions of `r4300_core.c`). The
  patch file is regenerated; its id is now `847c2e28`.
- **The event keeps its 40 bytes.** It was made for the 6502. A 32-bit address has its
  high half in two of the four reserved bytes (`addr_high`); an execute carries the whole
  instruction word in `value` and `operands`; the registers are 0. Mesen writes 0 in the
  reserved bytes, so its traces read the same as before.
- **dj** reads the whole address, has a small MIPS disassembler (loads, stores, jumps,
  branches, integer arithmetic; the rest as `.word`), and picks the instruction set from
  the connector's region. The MCP `trace` tool uses the same.
- **`--explain` traces frame by frame** and keeps the newest frames when they come to
  more than 12 million events. Before, one buffer of 4 million was filled from the start
  and the newest events were the ones dropped: on the N64, with ten frames back, it named
  a write six frames too early.
- **The stub's diff** is in region `RDRAM` now, the connector's name, so it is traced.

| Check | Result |
|---|---|
| No Mercy, all three segments from power-on with the new core (`dj repin`) | hashes reproduce; 41 frames a second, as before |
| `--explain` of the stub's fault (frame 1332, at `match_tick`) | 9,147,933 events over frames 1322 to 1332, none let go; the search stops 8,666,257 events in; the pads' last writer is `sb v0, 0xB(a1)` at `$80005B98`, inside `pads_read`, in frame 1332 |
| The same with `--frames-back 2` | 2,542,244 events; the same writer |
| Tetris `--explain` (Mesen) after the change | the same events, stop and writers as before |
| The player's tests (a new one for the disassembler), the GUI's build | pass |

Not checked: the MCP `trace` tool on the N64 (it shares the code, and was not called); a
trace's reads and writes against memory, byte by byte; a trace long enough to let frames
go. `calls` in the MCP server is still 6502 only. The No Mercy project is repinned to the
new build.

**Stepping back in the GUI, 2026-11-16** (after the developer ran the window with Tetris;
the hidden pre-roll looked right there).

- **What was seen:** stepping back, the comparison picture did not change until playback
  started.
- **The likely cause** (from the code, not confirmed in the window): the GUI used pre-roll
  only with Check parity on. Without it the jump goes at the frame shown, and a core that
  copies at a breakpoint draws nothing in the frame it copies in.
- **Pre-roll now runs whenever the comparison core runs,** checking or not, in the segment
  and above it.
- **A step back holds the sync** for 0.6 seconds (`STEP_BACK_HOLD`), as a held mouse button
  already did. Only the baseline runs meanwhile; the picture is faded with "waiting to
  sync" and "Syncs when you stop stepping back". Then the idle check re-runs the frame
  shown with a sync and its pre-roll. The developer suggested a second. 0.4 was tried
  first and was too short: it ran out before the key's repeat began, so the core synced
  once before the held steps started. It is 0.6 now.
- **Checked without a window** (Tetris, `preroll = 3`, Check parity off and on; a test
  written for it and removed): three steps back leave the core out of step with the
  baseline on the right frame; after the hold, one sync 3 frames before the frame shown;
  a step forward is not held; back at frame 0 the pre-roll goes into the parent; a
  picture is there with Check parity off.
- **Not checked:** the window, and the timing itself (the test's clock does not run).

**Two things the Thwaite decomp's thread reported, fixed 2026-11-16 17:00 EST.**

- **A diff with a symbol lost its address** when dj read it, so a core that gave both
  could not be explained. dj keeps the address whenever a region is given and shows the
  symbol beside it ("g_pads, RDRAM 0x9EDA0"). The No Mercy stub gives both again, and its
  failure is still explained to the same writer. (Step 11 had only fixed the place the
  explanation looked.)
- **The connector API had no header.** `connectors/include/dj_connector.h` declares it:
  the version, coverage regions, breakpoints, registers, the build id and the trace hook,
  with the trace event as it is now. It is a file of its own, not part of
  `dj_compare.h`: the Thwaite decomp has its own copy of these declarations
  (`src/dj_api.h`), and adding them to the header it includes would have broken its
  build. That thread can include the new header and drop its copy when it chooses. Its
  copy of the trace event still calls the last four bytes `reserved`, which is the same
  layout.

**Step 12, 2026-11-16: the docs.**

| File | What changed |
|---|---|
| `docs/cli.md` | `--explain` stops at the core's verdict; which connectors can trace; a failure's `at` |
| `docs/connectors.md` | "On the N64": byte order, other code at a breakpoint's address, the registers, ticks and frames, framebuffers. A failure names the event it answered. Which cores have `dj_parity_diffs` |
| `docs/gui.md` | How a verdict of a core that takes events counts frames; the hidden pre-roll (earlier the same day) |
| `docs/config.md` | `[cache] exclude`: how to check it, and that it no longer holds for No Mercy |
| `docs/database.md`, `docs/mcp.md` | `at` and pre-roll in a failure |
| `docs/segments.md` | Read through; it was already right (writes at an instruction came with step 5) |
| `.agents/ROADMAP.md` | A note at "Comparison and parity" that the API was replaced and which names are gone; the two rows of the release table; three places that named `DJ_SYNC_BOOT`; what is still to do after this work |
| `.agents/ISOLATION_PLAN.md` | The worker serves only `sync = "state"` cores now |

The roadmap's older entries still name the earlier functions. They are the record of what
shipped then, and the note above them says so.

## Spike results

Run 2026-11-15, as build steps 1 and 2. Each spike is a patched copy of a connector that
adds a few breakpoints and one function to call at each, and a harness that plays a
segment many times and compares the runs. Neither changes dj or the shipped connectors.

### Mesen (NES)

Thwaite, three segments: "Defend" (2,400 frames), "Idle" (3,600) and "Boot" (120).
Breakpoints at the tick, `doStateAction` (`PRG:0x197`), and the top of the game loop
(`PRG:0xA9`).

| Question | Result |
| --- | --- |
| The same hits across two runs? | Yes: frame, RAM hash and registers at every hit, up to 5,812 compared |
| The same state hashes with and without breakpoints? | Yes, after every frame. Each segment's recorded end-hash is reached |
| The same after loading a snapshot taken mid-segment? | Yes |
| Calls while saving or loading? | None |
| A save state taken with breakpoints armed | 0 of 35,840 bytes differ |
| A write to RAM inside the call | Reads back, takes effect in its own frame, repeats exactly, survives a snapshot |
| Registers | Read. PC is the breakpoint's address at every hit |
| Cost | About 720 frames a second not armed, 717 armed where nothing runs, 711 with 8.4 million calls (3,500 a frame). A call that returns at once: 3 to 4 ns. All within about 1%, near run-to-run noise |

- **The call arrives in the opcode fetch,** before the instruction runs.
- **A spin loop is as bad as feared.** A breakpoint in the boot's `vwait1` loop was hit
  3,891 times in one frame. The cap is needed.
- **No lag frame occurred** in these segments, so lag frames are still untested.
- **Only internal RAM on one game without bank switching** was tried.

### Mupen64Plus-Next (N64)

WWF No Mercy, two segments: "CPU vs CPU exhibition" (2,136 frames) and the four-CPU tag
match (1,800). Breakpoints at `match_tick` (`RDRAM:0x114514`) and at its one call site
(`RDRAM:0xD9DFC`). Run with GPU copies to RDRAM off; see "Existing problems" for why.

| Question | Result |
| --- | --- |
| Can the call be made on the game's coroutine? | Yes. Its stack is 8 MB (`co_create` in `libretro.c`), so its size is not a concern |
| Can it be made from `retro_run`, after a switch to the frontend's coroutine? | Yes. Nothing treats the switch as the end of the frame |
| The same hits from power-on and from a loaded state? | Yes: frame, RDRAM hash and registers at every hit |
| The same RDRAM with and without breakpoints? | Yes, after every frame |
| The same after loading a snapshot taken mid-segment? | Yes, with either way of calling |
| Calls while saving or loading? | None delivered |
| A write inside the call | Zeroing `jal rng_main` at RDRAM 0x570 took effect in that frame, repeated exactly, and survived a snapshot |
| Byte order of memory | 32-bit words in the host's order. On this machine the byte at address A is at A ^ 3 |
| Registers | Read. PC is the breakpoint's address at every hit |
| Cost | About 7 ns a call direct and 16 ns through `retro_run`, over 20 million calls. Playback speed is within noise either way |

- **Either way of calling works, so the direct call is enough.** The switch to the
  frontend's coroutine is not needed.
- **Other code at a breakpoint's address can be told apart by the registers.** In the
  exhibition segment `match_tick`'s address ran 588 times; 502 were the real function.
  The other 86, before the match was loaded, had another stack pointer (0x8008CFC8
  against 0x8008D118) and never reached the call site's breakpoint.
- **Two hits in one frame happen:** twice in each segment.

Not covered:

- The connector held back no hit during a load in these runs, so that guard never fired
  and is untested.
- One game, and only the pure interpreter, which dj forces.
- Finding the running thread. Only the stack pointer was read.

### Existing problems the N64 spike exposed

None is about breakpoints. Each affects dj as it is today.

1. **Save states on this core did not restore SRAM, and dj hashes SRAM.** *Fixed 2026-11-15
   in the connector patch.* "SRAM" is dj's name for libretro's save RAM, which on this
   core is every kind of save storage: EEPROM, Controller Paks, SRAM and Flash RAM. The
   game writes it at about frame 665 of the exhibition segment. A state loaded after that
   kept the later save until the game wrote it again: 665 frames of state hashes differed
   while RDRAM was identical.
   - **The fix:** a save state now carries the save storage after the core's own state,
     and a load puts it back. The reasoning is in `connectors/mupen64plus/README.md`.
   - **Checked** with a core built from the new patch on a clean copy of the pinned
     commit: no frame's state hash differs. A state cut to the old length still loads and
     behaves as before.
   - **Still to do by hand:** build the core (`connectors/mupen64plus/build.sh`) and
     `dj repin` the No Mercy project. The patch's id is now `4343e0a2`.
2. **`mupen64plus-EnableCopyColorToRDRAM = "Async"` makes RDRAM depend on what ran
   before.** With the project's own settings RDRAM differed for 1,138 frames between a
   run from power-on and one from a loaded state, and two runs from the same loaded state
   disagreed. The GPU's buffers are not in a save state and the copy is asynchronous. A
   comparison core that reads the picture's bytes from RAM would see the same noise. The
   roadmap already questions this default.
3. **Two saves with no frame between them.** The second runs the game on to its next
   interrupt before it saves, so it holds a state from inside the frame, and that state
   did not load back identically. dj saves at a segment's end and may save again at the
   next segment's frame 0.
   - **What it did to dj:** the second save, or a save right after a load, changed RAM, so
     the same frame hashed two ways. On No Mercy the root's start hash went from
     `19f776ca` to `6dafc72f` once the GUI had gone back to frame 0, which gave the root a
     second entry in the database, and a stored frame 0 carried another hash than its
     pass's start.
   - **Fixed 2026-11-16** in dj, not the connector: `Core::serialize`
     (`player/src/libretro.rs`) returns the state last saved or loaded for as long as no
     frame has run, nothing has been written through `memory` or `bus_byte`, no port's
     device has changed and no event function has been called. Reads go through
     `memory_view` and `bus_read`, which leave it alone. A pass's frame 0 is stored under
     its start hash.
   - **Checked** on No Mercy: the start hash, the hash after a snapshot, and the hash after
     loading that snapshot and saving again are the same, at power-on, at the exhibition's
     frame 0 and at its frame 25. All three segments still reach their recorded hashes,
     and parity against the stub gives the same counts as before.
   - **Not done:** whether a state saved from inside a frame loads back identically was
     not looked at again. dj no longer makes one unless memory is written between a load
     and a save with no frame run.

### Where the spike code is

- `player/examples/mesen_spike.rs` and `player/examples/n64_spike.rs`: the harnesses.
  *Removed from the repository on 2026-11-17 with the other one-off tools (release
  clean-up); they are in git history up to commit `f5bfd2f`.*
- `build/mesen-spike-src` and `build/mupen64plus-spike-src`: the patched copies. `build/`
  is not in git, so the connector changes exist only there. They add
  `dj_spike_set_breaks` and `dj_spike_registers` (and `dj_spike_suppressed` on the N64).
- `build/cores-spike/`: the two cores.

To run them again, from `player/`:

```bash
cargo run --release --example mesen_spike -- ../examples/thwaite ../examples/thwaite/build/thwaite.nes ../build/cores-spike/mesen_libretro d27cab3c "PRG:0x197,PRG:0xA9" 0x3DA
```

and from `examples/nomercy/`:

```bash
SPIKE_NO_FB=1 ../../player/target/release/examples/n64_spike . "WWF No Mercy (USA) (Rev 1).z64" ../../build/cores-spike/mupen64plus_next_libretro 38ac3eba "0x114514,0xD9DFC"
```

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

Decided since the 16:57 revision, and no longer open: the names, the default pre-roll,
the tail of a segment, a reason on "unjudged", a dropped write, and the worker.

Answered by the spikes, and no longer open: whether the N64 can make a blocking call
inside a frame, the byte order of N64 memory, and whether memory can be written inside a
frame.

Fixed since the spikes, and no longer open: save states on Mupen64Plus not restoring the
game's save storage ("Spike results", existing problem 1).

1. **The layout of the registers** has a first version for each instruction set
   (`docs/connectors.md`). The No Mercy stub reads the N64's (step 10): the return
   address, to tell a function's real call from other code. Finding the running N64
   thread was not tried.
2. **How the handle hands out N64 memory.** It is host-order 32-bit words. Either the
   handle says so, or it hands out bytes in the console's order. For now the docs say so
   (`docs/connectors.md`, "On the N64") and the core swaps.
3. **OpenGL contexts nesting on Linux** (section 11). Built, and tested on macOS; the EGL
   code has only been type-checked.
4. **A core that corrupts memory before it crashes,** dj's or the baseline's. No answer.
5. **dj's own tick count on the N64** counts other code at the `[tick]` address in
   another overlay: 86 of 588 hits in the spike. A project should pick an address in
   code that is always loaded.
6. **A fixed list of breakpoints and overlays** (section 1): whether "pending" is enough.
   The spike shows a core can tell the other code apart by the stack pointer. Step 10's
   stub does it by the caller, and "pending" was enough for No Mercy.
7. **GPU copies into RDRAM** ("Spike results", existing problem 2). An N64 project used
   for parity needs them off, or the picture's bytes left out of what is compared.
8. *No longer open (2026-11-16 23:47 EST):* two saves with no frame between them
   ("Spike results", existing problem 3). dj gives back the state it last saved or loaded
   while nothing has run or been written, so the core is not asked twice.
9. **The guard against hits during a load** never fired in the spike, so it is untested.
10. *No longer open (step 12):* `ROADMAP.md` and `ISOLATION_PLAN.md` carry notes that say
    which of their passages describe the earlier API, and the docs are up to date.
11. **No real decomp has a lag frame to test with.** Tetris has none (step 8). The rule
    for one rests on the test core.
12. *No longer open (2026-11-16 18:10 EST):* No Mercy's `[cache] exclude` broke stored
    snapshots because its range was 2 KB too wide ("Built so far", step 10). Both of the
    project's ranges are corrected.

## Build order

Thread. Both spikes come before the move into dj's process, which gives up a shipped
protection.

1. **A spike in the Mesen connector.** *Done 2026-11-15, and it passes: see "Spike
   results".* One breakpoint at Thwaite's game wait and the
   blocking call, with no comparison core. Done when the hits and RAM hashes are the
   same across two runs and after loading a snapshot, state hashes are the same as
   without the breakpoint, and the cost per hit is measured.
2. **A spike in the Mupen64Plus connector.** *Done 2026-11-15, and it passes, with GPU
   copies to RDRAM off: see "Spike results".* One breakpoint at No Mercy's `match_tick`.
   Done when the hit count is the same from power-on and from a snapshot, a load fires
   none, and state hashes are unchanged. If this cannot be made to work, stop and
   rethink.
3. **The new connector API in all three connectors,** with dj counting ticks itself.
   *Done 2026-11-15: see "Built so far".*
   Done when Thwaite's tick counts equal those of a
   plain `dj run` today, with the GUI and a parity check running, and mGBA still reports
   coverage.
4. **Move the comparison core into dj's process,** *(done 2026-11-15: see "Built so
   far")* with the crash note, the supervisor's
   handling of a comparison core that dies, and nested OpenGL contexts. Done when
   `tests/compare.rs` passes without a worker, and a core that crashes on purpose is
   named in the GUI's note and recorded as a failure at its frame by `dj parity`, with
   its earlier passes kept and the segments below it still checked.
5. **Writes at an instruction address** (section 3). *Done 2026-11-15: see "Built so
   far".* After step 4, because the
   same-emulator comparison needs the write armed in its connector too, and before step
   4 that core is in a worker. Done when a Thwaite segment with such a line plays to the
   same hash twice and from a snapshot, a line whose address is not reached is dropped
   with its warning, and the same-emulator comparison stays in parity across the write.
6. **`dj_handle_event` and `dj_handle_write`,** *(done 2026-11-15: see "Built so far")* in
   the test core (`player/tests/testcore`) and two stubs: one
   that copies the baseline at every event and answers match, one that diverges on
   purpose. Done when each event and each verdict has been seen; a run of mismatches has
   recorded one failure; playback has paused at the first mismatch of a run and not at the later ones; a halt has
   been resumed; and neither an unjudged stretch nor the frames between a jump and the
   copy have credited any coverage. The cases in STATES.md, sections 3 and 4, are the
   test list.
7. **The helper header, and the Tetris decomp on it,** *(done 2026-11-15: see "Built so
   far")* at the frame end. Done when its 30
   segments pass as they do today, the first frame after a jump included.
8. **The Tetris decomp at a breakpoint inside the frame,** *(done 2026-11-15: see "Built
   so far"; the lag frames are explained, and none needs "unjudged")* at its game wait. It is the
   only real decomp at hand, and its 15 lag frames test where a core must answer
   "unjudged". Done when its 30 segments pass, with the lag frames unjudged or
   explained.
9. **Pre-roll** (section 6), *(done 2026-11-16: see "Built so far")* with the Tetris decomp from step 8 and `preroll` set on its
   entry. Done when `dj parity` checks frame 0 of every segment that is not a root, and a
   failure found after a pre-roll re-runs to the same frame with the local database
   emptied.
10. **N64 breakpoints in full,** *(done 2026-11-16: see "Built so far")* `match_tick` and the input routine, with a stub core
    that copies the pad buffer. Done when both count the same from power-on and from a
    snapshot.
11. **Failures and the tools** *(done 2026-11-16: see "Built so far")* (section 9). Done when a failure at a breakpoint inside a
    frame can be re-run and explained from the command line.
12. **The docs** *(done 2026-11-16: see "Built so far")*: `connectors.md`, `segments.md`, `config.md`, `gui.md`, `cli.md`, and
    the notes on `ROADMAP.md` and `ISOLATION_PLAN.md`.

A real decomp of a game whose ticks cross frames is the last test, and the only one that
proves the design. None is at hand as a libretro core.
