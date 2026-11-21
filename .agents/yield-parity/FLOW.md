# Flow: parity by breakpoints

Written 2026-11-15 15:20 EST; rewritten 16:28 EST, revised 16:57 and 17:30 EST with
[PLAN.md](PLAN.md). This is the design in that file as charts. The same rules as tables
of cases, with worked examples, are in [STATES.md](STATES.md). Earlier charts are in
[archive/](../_archive/yield-parity/); agents should not read those unless the developer says to. Nothing
here is built.

The comparison core lists breakpoints in the original game. dj runs the baseline and
calls the core with an event at each one, and at each frame end. The core runs itself
and answers pending, match, mismatch, unjudged or halt. dj does not sync the core or
track whether it is live.

A comparison core that is the same emulator as the baseline is not in these charts. It
is checked at the frame end as it is today.

## Where the baseline starts after a jump

Only when checking parity. Playing straight on from a parent into its child is not a
jump and skips this.

```mermaid
flowchart TD
    j(["A jump is asked for, to frame T of a segment:<br/>dj parity starting a segment, a seek, a selected segment"]) --> root{"Is T the start<br/>of a root segment?"}
    root -- "yes" --> none["No pre-roll. Power on.<br/>The jump is at T"]
    root -- "no" --> bp{"Does the core's [[compare]] entry<br/>set preroll = N, above zero?"}
    bp -- "no: the default.<br/>The core copies between frames" --> none2["No pre-roll. Load T.<br/>The jump is at T"]
    bp -- "yes" --> n["J is N frames before T"]
    n --> clamp["J stops at a root segment's start.<br/>It may be in the parent or further up"]
    clamp --> snap{"A stored snapshot at or<br/>before J? An ancestor's counts"}
    snap -- "yes" --> ls["Load the nearest one"]
    snap -- "no" --> ps["Start from the parent's start"]
    ls --> quiet["Play the baseline to J<br/>with no events sent"]
    ps --> quiet
    quiet --> owe["A jump is owed, at J.<br/>Frames J up to T are pre-roll"]
    none --> owe2["A jump is owed. No frame is pre-roll"]
    none2 --> owe2
    owe --> play(["Play. See 'One frame'"])
    owe2 --> play
```

The jump is sent at J whatever snapshot was loaded, so a failure re-runs the same way on
a machine with other snapshots.

"Copy again" jumps have no pre-roll: Sync now, play after a halt, and switching the
comparison on. Nothing is loaded, so there is nowhere earlier to start.

## One frame

```mermaid
flowchart TD
    load(["The comparison core loads"]) --> ask["dj calls dj_breakpoints once and gives<br/>the addresses to the baseline's connector,<br/>with its own [tick] address"]
    ask --> owe["A jump is owed"]
    owe --> fstart

    fstart["Frame k: apply its input lines to both cores.<br/>Switch capped breakpoints back on.<br/>The baseline is between frames"] --> owed{"Is a jump owed?"}
    owed -- "yes" --> jump[["Event: jump, with why.<br/>Empty the held coverage"]]
    owed -- "no" --> rs
    jump --> rs{"A RESET line<br/>in frame k?"}
    rs -- "yes" --> rev[["retro_reset on the baseline.<br/>Event: console reset"]]
    rs -- "no" --> wr
    rev --> wr{"A WRITE line<br/>in frame k?"}
    wr -- "yes, with no address" --> wev[["Apply it to the baseline.<br/>Call dj_handle_write, with the bytes"]]
    wr -- "yes, with an address" --> arm["Arm it: add its address to the<br/>connector's breakpoints for this frame"]
    wr -- "no" --> run
    arm --> run
    wev --> run["Baseline: retro_run"]

    run --> hit{"The baseline reaches<br/>an armed address?"}
    hit -- "yes" --> gate{"A load in progress,<br/>or over the cap this frame?"}
    gate -- "a load" --> run
    gate -- "over the cap" --> capoff["Warn once. Switch it off<br/>until the frame ends"]
    capoff --> run
    gate -- "no" --> each["For each thing armed at this address, in this order"]
    each --> tick["dj's [tick]: add one to the tick count"]
    tick --> aw[["An armed write: write the bytes to the baseline,<br/>stopped here. Call dj_handle_write, with the bytes<br/>and this address. Disarm it"]]
    aw --> bev[["A breakpoint of the core's:<br/>Event: break, with the id"]]
    bev --> run

    hit -- "no, the frame ended" --> left{"A write still armed?"}
    left -- "yes" --> wwarn["Drop it. Warn in the console, naming the line:<br/>its address was not reached in frame k.<br/>The core is not told"]
    wwarn --> pre
    left -- "no" --> pre{"Is frame k pre-roll?"}
    pre -- "no" --> fend["Take the frame's coverage flags, as today.<br/>Add them to the held coverage, unless an unjudged<br/>or a mismatch inside this frame claimed them"]
    pre -- "yes" --> fskip["Take the frame's coverage flags<br/>and hold none of them"]
    fend --> fev[["Event: frame end"]]
    fskip --> fev
    fev --> pic["Show the core's latest picture,<br/>or keep the old one if it drew none"]
    pic --> lost{"The first mismatch of a run,<br/>or a halt, this frame?"}
    lost -- "no" --> next(["Frame k + 1"])
    lost -- "yes" --> stop(["Pause at this frame boundary.<br/>Show the breakpoint and the reasons"])
    next --> fstart
```

- Every event carries the segment and frame, whether the frame is pre-roll, and a handle
  that reads the baseline's memory and registers.
- No event is sent while the core is halted, or while the comparison is switched off.
- Most addresses have only one thing armed. The order matters when a write and a
  breakpoint of the core's share one: the core sees the write before it compares.
- Each event carries a frame number: between frames, the frame about to run; inside a
  frame, the frame running; at a frame end, the frame just run.
- The first mismatch of a run, or a halt, answered between frames pauses playback there,
  before the frame runs. The chart shows the usual case, where it comes inside the frame
  or at its end.
- A core with no `dj_handle_write` is not called for a write. dj owes it a jump before
  the next frame instead, and for a write with an address treats the rest of that frame
  as unjudged.

## An event and its verdict

The call blocks for as long as the core takes.

```mermaid
flowchart TD
    ev(["dj calls dj_handle_event,<br/>or dj_handle_write for a write"]) --> runs["The core does what it decides: copies the baseline,<br/>runs itself to this point, compares, draws.<br/>It may call the video callback at any time"]
    runs --> pr{"Is this pre-roll?"}
    pr -- "yes" --> prv{"Verdict"}
    prv -- "halt" --> h
    prv -- "anything else" --> ret(["Return. The baseline carries on"])

    pr -- "no" --> v{"Verdict"}
    v -- "pending" --> p["Nothing. The held coverage stays"]
    v -- "match" --> m["Add the held coverage to the<br/>build's passed set. Empty it"]
    v -- "unjudged" --> u["Keep the reason, if one came.<br/>Empty the held coverage.<br/>If this is inside a frame, that frame's<br/>flags will be discarded too"]
    v -- "mismatch" --> prev{"Was the core's last verdict,<br/>pending aside, a mismatch?<br/>It is none after a jump<br/>and at the end of pre-roll"}
    prev -- "yes" --> shown["Show it. Keep no failure.<br/>Empty the held coverage. If inside a frame,<br/>that frame's flags will be discarded.<br/>Playback goes on"]
    prev -- "no" --> x["Read the reasons, with both sides' values.<br/>Note the trace position.<br/>The failure gets the held coverage and,<br/>if inside a frame, that frame's flags.<br/>Playback will pause at the frame boundary"]
    v -- "halt" --> h["Read the reason. The failure gets the held coverage<br/>and, if inside a frame, that frame's flags.<br/>Playback will pause at the frame boundary.<br/>The core is halted: a jump is owed"]

    p --> ret
    m --> ret
    u --> ret
    shown --> ret
    x --> ret
    h --> ret
```

With parity checking off, dj discards match, mismatch and unjudged; a halt still stops
playback.

A verdict given between frames (to a jump, a console reset, or a write with no address)
never touches the frame ahead, which has not run. In pre-roll a halt's failure gets only
the flags of the frame it happened in.

## What the core is sent over time

dj keeps two facts about the comparison core: whether a jump is owed, and whether it is
halted. Everything else is the core's own.

```mermaid
stateDiagram-v2
    [*] --> JumpOwed: the core loads
    JumpOwed --> Running: the next frame starts.<br/>dj sends one jump
    Running --> Running: console reset, write, break, frame end.<br/>pending, match, unjudged
    Running --> Running: mismatch. The first of a run pauses playback.<br/>Play or step carries on, with no jump
    Running --> JumpOwed: the baseline's state is replaced<br/>(a seek, a snapshot, a power-on),<br/>or the user picks Sync now
    Running --> Halted: halt
    Halted --> JumpOwed: play or step is pressed, or<br/>the baseline's state is replaced
    Running --> Off: the comparison is switched off
    Off --> JumpOwed: it is switched on
```

- **Several loads owe one jump.** Dragging the ruler loads many states; the core hears
  of it once, before the next frame that runs.
- **A parent segment playing into its child changes nothing.** No state was replaced. If
  the parent's last frame ended mid-tick, the core stays parked at its last breakpoint
  until the child plays, and its held coverage goes with it.
- **The jump says why:** a fresh power-on, a replaced state, or "nothing was replaced,
  copy again".

## What a core does after a jump

This is the core's own logic, not dj's. It is shown because it replaces dj's sync. The
helper header shipped with dj does it for a core that keeps the original's RAM layout.

```mermaid
flowchart TD
    r(["Event: jump. The baseline is between frames"]) --> fe{"Do I judge at frame ends?"}
    fe -- "yes" --> copy0["Copy what I need through the handle, now.<br/>Draw from it if I can.<br/>Answer unjudged"]
    copy0 --> n
    fe -- "no: I judge at a tick" --> stale["Mark myself stale. Answer pending"]
    stale --> e(["The next event"])
    e --> can{"Can I copy the baseline here?<br/>Only at my tick breakpoint"}
    can -- "no" --> pend["Answer pending"]
    pend --> e
    can -- "it will never work" --> halt(["Answer halt, with the reason"])
    can -- "yes" --> copy["Copy what I need through the handle.<br/>Answer unjudged: what ran since<br/>the jump was compared with nothing"]
    copy --> n(["Each later event"])
    n --> kind{"What is it?"}
    kind -- "not one I act on" --> pend2["Answer pending"]
    kind -- "console reset, and I am parked<br/>at a frame boundary" --> rst["Run my own reset path.<br/>Answer pending"]
    kind -- "console reset, and I am parked<br/>at a tick, or have no reset path" --> stale
    kind -- "write, and I am parked<br/>where it landed" --> wr["Apply the bytes to my own state.<br/>Answer pending"]
    kind -- "write, and I am parked<br/>somewhere else" --> stale
    kind -- "one I judge at" --> go["Run myself to this point.<br/>In pre-roll, I may skip the comparison"]
    go --> res{"Result"}
    res -- "same" --> ok["Answer match"]
    res -- "different" --> bad["Answer mismatch, with reasons"]
    res -- "cannot tell here: a lag frame,<br/>an overlay I do not know" --> unj["Answer unjudged"]
    pend2 --> n
    rst --> n
    wr --> n
    ok --> n
    bad --> n
    unj --> n
```

After a mismatch the core keeps its own state, and play or step carries on with no
jump. Whether it then drifts, matches again, or copies the baseline afresh is the core's
choice. "Sync now" sends a jump and makes it copy.

The "answer unjudged" on the jump event claims nothing: it comes between frames, so the
frame that follows is still credited by its own match.

A core is parked where a write landed in two cases: it judges at frame ends and the
write had no address, or it has a breakpoint at the write's instruction address.
