> **Archived. Agents: do not read this file unless the developer tells you to.** It is an earlier revision or a long design discussion, kept for the record, and it contradicts the current design in places. The current design is [PLAN.md](../PLAN.md), with charts in [FLOW.md](../FLOW.md).

# Flow: sync, ticks and parity, now and as proposed

Written 2026-11-11 by the meta-analysis thread, to go with
[CHECKPOINT_PARITY_PLAN.md](CHECKPOINT_PARITY_PLAN.md).

- **"Now"** is what the code does today (`player/src/parity.rs`, `player/src/compare.rs`,
  `player/src/player.rs`, `gui/src/main.rs`).
- **"Abandoned"** is the first design for checking inside a frame, where the baseline
  stops at a yield and returns to dj (`dj_run_to_yield`). It was marked abandoned on
  2026-11-11 and is kept for the record only.
- **"Current proposal"**, at the end of this file, is the design that replaced it: the
  connector calls dj at each yield and the baseline never returns mid-frame. Nothing in it
  is built.

## Now: one check per frame

The comparison core runs in a worker process. The handle it gets points to a copy of the
baseline that has only had the baseline's save state loaded into it.

```mermaid
flowchart TD
    start(["Next frame k of the segment"]) --> instep{"Comparison core in step<br/>at frame k?"}

    instep -- "no: after a jump, a failure,<br/>or a WRITE it couldn't follow" --> ser1["Save the baseline's state"]
    ser1 --> mode{"sync mode"}
    mode -- "dj_sync" --> shadow1["Worker loads the state<br/>into its copy of the baseline"]
    shadow1 --> djsync["dj_sync with the handle, the frame,<br/>and DJ_SYNC_BOOT at a fresh power-on"]
    mode -- "state" --> unser["Comparison core loads<br/>the baseline's state"]
    djsync --> syncok{"Synced?"}
    unser --> syncok
    syncok -- "no" --> syncfail["Show the reason.<br/>Try again at the next jump"]
    syncok -- "yes" --> held["Set the comparison core's<br/>devices and held inputs"]
    held --> base

    instep -- "yes" --> base["Baseline: apply frame k's lines,<br/>then retro_run"]
    syncfail --> baseonly["Baseline: apply frame k's lines,<br/>then retro_run. Frame not checked"]

    base --> lines["Comparison core: apply frame k's lines"]
    lines --> write{"A WRITE line?"}
    write -- "has dj_write" --> djwrite["dj_write, with the copy of<br/>the baseline as the handle"]
    write -- "no dj_write" --> needsync["Mark: sync again<br/>before the next frame"]
    write -- "none" --> run
    djwrite --> run
    needsync --> run["dj_run if the core has it,<br/>else retro_run"]

    run --> runok{"dj_run failed?"}
    runok -- "yes" --> fail
    runok -- "no" --> checking{"Check parity on,<br/>and no sync needed?"}
    checking -- "no" --> flags
    checking -- "yes" --> ser2["Save the baseline's state.<br/>Worker loads it into the copy"]
    ser2 --> check["dj_check_parity with the handle,<br/>or compare system RAM byte for byte"]
    check --> verdict{"Verdict"}
    verdict -- "match" --> pass["Passed"]
    verdict -- "skip" --> skip["Not judged"]
    verdict -- "mismatch or error" --> fail["Failed: read dj_parity_diffs.<br/>Record the build, the sync point,<br/>the frame and what differed"]

    pass --> flags["Take the baseline's coverage flags<br/>for this frame, and clear them"]
    skip --> flags
    fail --> flags
    flags --> credit{"Outcome"}
    credit -- "passed" --> addpass["Add the frame's flags<br/>to the build's passed set"]
    credit -- "failed" --> stop(["Stop both cores.<br/>The comparison core is out of step"])
    credit -- "other" --> next(["Frame k + 1"])
    addpass --> next
    baseonly --> next
```

### Ticks, now

```mermaid
flowchart TD
    cfg["[tick] exec in config.toml"] --> set["Session opens:<br/>dj_set_tick_exec on the baseline"]
    set --> count["The connector counts each time<br/>that instruction runs"]
    count --> clear{"dj_clear called?"}
    clear -- "segment start" --> zero["Count goes to 0"]
    clear -- "every checked frame,<br/>and every GUI refresh" --> zero
    zero --> count
    count --> fin["Segment end: read the count once<br/>and store it as the segment's ticks"]
    fin --> bug1["Undercounted whenever<br/>the GUI or parity ran"]

    copy["The worker's copy of the baseline"] --> never["Never runs a frame,<br/>never has its tick location set"]
    never --> bug2["dj_get_tick_count through<br/>the handle always reads 0"]
```

## Abandoned: stop-and-return checks at yields (was "Proposed")

> **Abandoned on 2026-11-11.** Do not build from this section. It covers everything from
> here down to "Sub-frame input, after the release". The design that replaced it is under
> "Current proposal: a blocking call at each yield" at the end of this file, and the
> reasons are in the plan's section "Experiments and where the design stands".
>
> What was dropped: `dj_run_to_yield` and the baseline returning to dj mid-frame, the
> `check` key per yield, and the rule that picks one frame's inputs for a step.

A yield is a place in the original's code where one step of its logic is done and it
starts to wait. The baseline stops there, mid-frame, and the comparison core runs one step
to catch up. A frame can hold no yield (a lag frame), one, or several.

```mermaid
flowchart TD
    jump(["After a jump: the baseline is<br/>at a frame start"]) --> points["dj_set_points on the baseline:<br/>yields and counters.<br/>Arms the yields, zeroes the counts"]
    points --> fstart

    fstart["Frame k: apply its lines.<br/>WRITE lines go to both cores"] --> runy["Baseline: dj_run_to_yield.<br/>Runs at least one instruction, then on<br/>to the next yield or the frame end"]
    runy --> sample["Sample dj_input_reads and the counts.<br/>Take coverage flags and carry them"]
    sample --> what{"What stopped it?"}

    what -- "the frame ended" --> fend["Count one more frame end<br/>for the step in progress"]
    fend --> present["dj_present on the comparison core:<br/>show its picture, advance nothing"]
    present --> fnext(["Frame k + 1"])
    fnext --> fstart

    what -- "a yield" --> cap{"This yield hit more than<br/>max_per_frame times?"}
    cap -- "yes" --> caperr(["Error naming the yield:<br/>it is misconfigured"])
    cap -- "no" --> snap["Snapshot the baseline's memory<br/>and CPU state for the handle.<br/>No save state is taken"]
    snap --> instep{"Comparison core in step?"}

    instep -- "no" --> djsync["dj_sync with the handle:<br/>the yield's name, memory, CPU state"]
    djsync --> syncok{"Synced?"}
    syncok -- "no" --> syncfail["Show the reason.<br/>Try again at the next yield"]
    syncok -- "yes" --> resume
    syncfail --> resume

    instep -- "yes" --> inputs["Set the comparison core's inputs:<br/>those of the frame where the read count<br/>first rose since the previous yield"]
    inputs --> adv["dj_advance_to_yield with the handle:<br/>yield, frame ends since the last yield, counters.<br/>The decomp does its vblank work, then one step"]
    adv --> reached{"Which yield did<br/>the decomp reach?"}
    reached -- "none" --> unchecked["Step not checked.<br/>Sync again at the next yield"]
    reached -- "a different one" --> fail
    reached -- "the same one" --> wantcheck{"check = true<br/>for this yield?"}
    wantcheck -- "no" --> lined["Lined up, not checked"]
    wantcheck -- "yes" --> check["dj_check_parity with the handle,<br/>or compare the handed-over RAM"]
    check --> verdict{"Verdict"}
    verdict -- "match" --> pass["Add the flags carried since<br/>the previous yield to the passed set"]
    verdict -- "skip" --> lined
    verdict -- "mismatch or error" --> fail["Failed: record the frame, which yield<br/>of the frame, and what differed"]

    fail --> finish["Finish the baseline's frame:<br/>dj_run_to_yield until it ends,<br/>ignoring yields"]
    finish --> stop(["Stop both cores.<br/>The baseline is on a frame boundary"])

    pass --> resume
    lined --> resume
    unchecked --> resume["Frame ends since the last yield: 0"]
    resume --> runy
```

### What changes for ticks (abandoned)

```mermaid
flowchart TD
    step0["Step 0, for every connector:<br/>dj reads the count before each dj_clear<br/>and adds it to a running total"] --> fixed["The segment's stored ticks are right"]
    api3["API 3 connector: the tick is a counter<br/>set with dj_set_points"] --> keep["Its count survives dj_clear"]
    keep --> handle
    step0 --> handle["The handle carries the counters:<br/>total since the sync, and this frame's"]
    handle --> decomp["The comparison core reads them<br/>from the handle, never through get_proc"]
```

### The same emulator as the comparison core (`sync = "state"`) (abandoned)

```mermaid
flowchart TD
    s1(["After a jump"]) --> s2["Sync at the frame start:<br/>the comparison core loads<br/>the baseline's save state"]
    s2 --> s3["dj_set_points on both cores,<br/>the same list"]
    s3 --> s4["Baseline: dj_run_to_yield"]
    s4 --> s5["Comparison core: dj_run_to_yield"]
    s5 --> s6{"Both returned the same yield,<br/>or both ended the frame?"}
    s6 -- "no" --> s7(["Failure: out of line"])
    s6 -- "yes, a checked yield" --> s8["Compare RAM"]
    s6 -- "yes, a frame end" --> s4
    s8 --> s9{"Match?"}
    s9 -- "yes" --> s4
    s9 -- "no" --> s7
```

### Still open in the abandoned design

| Box | What's open |
| --- | --- |
| "dj_run_to_yield … to the next yield" | How a yield in a spin loop avoids firing every iteration. The plan's thread wants a yield to be entry into a range of code; this thread found that fires on every interrupt during the wait, and suggests one-instruction yields with a list of locations |
| "Both returned the same yield, or both ended the frame?" | The plan's thread wants only the sequence of yields to match, so emulators with different timing can be compared. That leaves the comparison core's inputs undefined |
| "Set the comparison core's inputs" | A step that reads the controller in two frames gets the first frame's inputs. On Mesen, a read in the last part of scanline 240 sees the previous frame's |
| "The decomp does its vblank work" | In a lag step the handle can't say where in the step's logic the extra frame ends fell |
| Everything in "Proposed" | Not built or run. The plan's latest review asks for the Thwaite probe and a Mesen spike first |

## Sub-frame input, after the release

Not part of any flow in this file: inputs stay keyed by frame. Later, a segment line such as
`812.2 P1 JOYPAD A 1` (segment format 2) would change an input from the game's third
controller read in frame 812. The controller read count that the connector reports is what that
index would use.

## Current proposal: a blocking call at each yield

Added 2026-11-11 21:23 EST by the meta-analysis thread. Last rewritten 2026-11-12 21:35 EST, when
the design was settled enough to move to its own folder. **The same charts, without the
history on this page, are in [yield-parity/FLOW.md](FLOW.md), and the
design is written out in [yield-parity/PLAN.md](PLAN.md). Build from
those.** Nothing here is built.

What changed in this last rewrite, with the plan section that decided it:

- The comparison core no longer subscribes to yields; every yield is passed to it, and
  it answers "skip" for one it disregards. Yields have no N64 guard. ("Yield config, the
  call, and patch files")
- The statuses are match, mismatch, skip and error, with a "completed a tick" flag. The
  core gives every verdict; dj compares no memory itself. (the same section, and "Final
  round")
- A mismatch or an error stops playback at the frame boundary. There is no halt setting
  and no time limit box. (the same sections)
- After a load, dj offers a sync at every yield, the frame end included, and the core
  accepts where it can. Checkpoints and the "is this a tick start?" test are gone.
  ("Final round")

### One frame

```mermaid
flowchart TD
    setup(["Session opens"]) --> arm["dj gives the baseline's connector the yields:<br/>the [tick] and [[yield]] entries.<br/>The frame end is always one"]
    arm --> fstart

    fstart["Frame k: apply its input lines to both cores.<br/>A WRITE line goes to the baseline<br/>and puts a decomp out of step"] --> run["Baseline: retro_run, as today"]
    run --> hit{"The baseline reaches<br/>a yield's address?"}
    hit -- "yes" --> call[["The connector calls dj and waits.<br/>See 'At a yield'"]]
    call --> run
    hit -- "no, the frame ended" --> fend["retro_run returns"]

    fend --> note["dj adds frame k's number to the list of frames<br/>since the last call to the comparison core"]
    note --> fcall[["The frame-end yield: the same handling<br/>as 'At a yield', with the baseline between frames"]]
    fcall --> pic["Show the comparison core's picture"]
    pic --> lost{"Was a mismatch or an error<br/>recorded at any yield this frame?"}
    lost -- "no" --> next(["Frame k + 1"])
    lost -- "yes" --> halt(["Stop both cores at this frame boundary.<br/>Show the yield and the reasons.<br/>A user or an agent may step on by hand"])
    next --> fstart
```

### At a yield

The baseline is stopped, mid-frame unless this is the frame end, for as long as this
takes. dj may read its memory and registers. dj must not run it, save it, load it or
reset it. Whatever the answer, the call returns and the frame finishes.

```mermaid
flowchart TD
    enter(["The connector calls dj: which yield, which address"]) --> off{"Is a save state<br/>being loaded?"}
    off -- "yes" --> ret(["Return. The baseline carries on"])
    off -- "no" --> count["Add one to this yield's count.<br/>For [tick], that is the baseline's tick count"]
    count --> cap{"More than the cap<br/>for one frame?"}
    cap -- "yes" --> caperr["Record an error naming the yield:<br/>it is in a spin loop"]
    caperr --> ret
    cap -- "no" --> loaded{"Is a comparison core loaded,<br/>and no mismatch or error<br/>recorded yet this frame?"}
    loaded -- "no" --> ret

    loaded -- "yes" --> instep{"Comparison core in step?"}
    instep -- "no" --> offer[["Offer a sync.<br/>See 'After a load or a jump'"]]
    offer --> ret

    instep -- "yes" --> tell["One call to the comparison core: the yield's name,<br/>which address fired, whether parity checking is on,<br/>the yield's counts, the thread on the N64,<br/>the numbers of the frames since the last call,<br/>and a handle to read the baseline"]
    tell --> runs["The core runs itself for as long<br/>as it decides, then returns a status,<br/>a 'completed a tick' flag, and reasons"]
    runs --> status{"Status"}

    status -- "skip" --> skipc["Count it as not judged"]
    skipc --> empty
    status -- "match" --> pass["Take and clear the coverage flags.<br/>Add them to the build's passed set.<br/>Count it as judged"]
    status -- "mismatch" --> lostrec["Take and clear the coverage flags.<br/>Record the yield, the frame and the reasons,<br/>with both sides' values"]
    status -- "error" --> cannot["Record the error.<br/>The core is out of step"]

    pass --> empty["Empty the list of frames<br/>since the last call"]
    lostrec --> empty
    cannot --> empty
    empty --> tick{"Did the core say it<br/>completed a tick?"}
    tick -- "yes" --> addtick["Add one to the comparison's tick count"]
    tick -- "no" --> ret
    addtick --> ret
```

The core gives every verdict. dj does not compare memory itself and keeps no list of
bytes to ignore. "skip" covers a yield the core disregards, an overlay it is not built
for, and checking being off.

### After a load or a jump

A save state is only ever taken or loaded at a frame start. dj does not manage a
comparison core's memory: the core brings itself fully into step from the baseline.

```mermaid
flowchart TD
    load(["A save state is loaded:<br/>a seek, a segment's snapshot, a parent's end state"]) --> kind{"What is the<br/>comparison core?"}

    kind -- "the same emulator" --> st["It loads the same save state,<br/>at this frame start. In step at once"]

    kind -- "a decomp" --> out["The core is out of step.<br/>The baseline plays on as normal"]
    out --> ynext["The baseline reaches a yield.<br/>The frame end is one"]
    ynext --> offer["dj offers a sync: dj_sync with the yield's name,<br/>which address fired, and a handle to read the baseline"]
    offer --> answer{"The core's answer"}
    answer -- "declines: it cannot sync here" --> ynext
    answer -- "failed" --> failed(["Show the reason.<br/>No more offers until the next load or jump"])
    answer -- "synced" --> instep(["In step from this yield.<br/>Nothing before it was checked.<br/>Checking starts at the next yield"])

    cont(["A parent segment plays straight into its child"]) --> nojump["Not a jump: the comparison core stays in step.<br/>The list of frames and the coverage flags carry across,<br/>and are credited to the segment where the yield lands"]
```

A project with no `[tick]` and no `[[yield]]` has only the frame end, so its decomp
syncs at a frame boundary. That is what works today for Tetris. A decomp of a game whose
frame ends fall inside a tick (No Mercy's tag match) declines the frame end and syncs at
its tick.

### The same emulator as the comparison core

Two emulators can't each wait inside the other's call, so neither is called at a yield.
With only the frame end this is today's check: one comparison per frame.

```mermaid
flowchart TD
    a(["After a jump: both load the same<br/>save state at a frame start"]) --> b["Both get the same list of yields"]
    b --> c["Baseline runs frame k. At each yield, the frame end<br/>included, its connector records the yield and a hash of RAM"]
    c --> d["Comparison core runs frame k<br/>and records the same way"]
    d --> e{"Same yields in the same order,<br/>with the same hashes?"}
    e -- "yes" --> f(["Frame k + 1"])
    e -- "no" --> g(["Mismatch: name the first yield that differs.<br/>Stop both cores"])
    f --> c
```

### Still open

See "Open problems" in [yield-parity/PLAN.md](PLAN.md).
