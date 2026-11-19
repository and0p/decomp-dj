> **Archived. Agents: do not read this file unless the developer tells you to.** It is an earlier revision or a long design discussion, kept for the record, and it contradicts the current design in places. The current design is [PLAN.md](../PLAN.md), with charts in [FLOW.md](../FLOW.md).

# Flow: yields, sync and parity

Written 2026-11-12 21:35 EST by the meta-analysis thread. This is the design in
[PLAN.md](PLAN.md) as charts. Nothing here is built.

A yield is a named place where the baseline calls dj and waits. `[tick]` and each
`[[yield]]` in the project config is one; the frame end is always one. dj passes every
yield to the comparison core, which answers match, mismatch, skip or error.

How dj works today, and the designs that were dropped on the way here, are in
[../CHECKPOINT_PARITY_FLOW.md](CHECKPOINT_PARITY_FLOW.md).

## One frame

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

## At a yield

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

## After a load or a jump

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

## The same emulator as the comparison core

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
