# States and cases: parity by breakpoints

Written 2026-11-15 16:57 EST, at the developer's request, to go with [PLAN.md](PLAN.md)
and [FLOW.md](FLOW.md). Revised 17:30 EST for the developer's decisions that day.
Nothing here is built.

PLAN.md gives the rules. This file lists every state the baseline, dj, the comparison
core and the GUI can be in, and walks each case through the rules. It exists because
three reviews in a row found rules that failed in a case nobody had written down. If a
case here disagrees with PLAN.md, one of them is wrong: fix both.

Section 6 lists what writing the cases out turned up.

## 1. Who holds what

| Who | What it holds | Who else can see it |
| --- | --- | --- |
| Baseline | The game's state. Its place: a segment and a frame. Whether it is between frames or inside `retro_run` | dj; the core through the handle, during a call |
| dj | **Comparison:** off, on, or on with parity checking | The GUI shows it |
| dj | **Jump owed:** no, or yes with a reason (power-on, replaced state, copy again) | Nobody |
| dj | **Halted:** the core said halt and no jump has been sent since | The GUI shows it |
| dj | **Pre-roll until T:** the frames before T are pre-roll | The core, as a flag on each call |
| dj | **Last verdict:** none, match, unjudged or mismatch. Pending does not change it. It is "none" after a jump and at T | Nobody |
| dj | **The pile:** coverage flags of whole frames, held until a verdict | Nobody |
| dj | **This frame's claim:** none, discard or blame. Set by a verdict inside the frame, used when the frame's flags arrive | Nobody |
| dj | **Armed writes** for this frame, and **breakpoints switched off** by the cap | Nobody |
| dj | **The last jump's place:** segment and frame. A failure records it | Kept with the failure |
| Comparison core | **Stale or live:** whether it has copied the baseline since the last jump | Nobody. dj never knows |
| Comparison core | **Where it is parked:** at a frame boundary, or at one of its breakpoints | Nobody |
| GUI | Playing, paused, stepping, or dragging the ruler. The end-of-segment mode: stop, loop or next. The selected segment and the sticky chain | The user |

The comparison core's two facts are the ones the earlier designs had dj track. Here dj
does not, so every rule below must work without them.

## 2. The three places a call can arrive

"Event" below means a call to `dj_handle_event`. A write is a call to `dj_handle_write`.
Both return a verdict, and the rules are the same for both.

| Place | Calls | The baseline is |
| --- | --- | --- |
| **Between** frames | jump, console reset, a write with no address | Stopped before frame k's `retro_run`. The call carries k, the frame about to run |
| **Inside** frame k | break, a write with an address | Stopped inside `retro_run`. The call carries k |
| **End** of frame k | frame end | Back from `retro_run`. Frame k's flags have just been dealt with. The call carries k, the frame just run |

Between frames, the calls come in this order: jump, console reset, write.

## 3. Every verdict in every place

Parity checking on, not pre-roll. "This frame" is frame k.

| Verdict | Between | Inside | End |
| --- | --- | --- | --- |
| pending | Nothing | Nothing | Nothing |
| match | Credit the pile, empty it. Usually empty already | Credit the pile, empty it. Frame k is not on it yet | Credit the pile, empty it. Frame k is on it, unless claimed |
| unjudged | Empty the pile. **Frame k is not touched** | Empty the pile. Claim frame k: discard | Empty the pile, frame k with it |
| mismatch, first of a run | The failure gets the pile. Pause now, before frame k runs. It is recorded against frame k with the call's name | The failure gets the pile. Claim frame k: blame. Pause when frame k ends | The failure gets the pile, frame k with it. Pause |
| mismatch, later in a run | Empty the pile. Playback goes on | Empty the pile. Claim frame k: discard. Playback goes on | Empty the pile. Playback goes on |
| halt | As a first mismatch. Halted; a jump is owed | The same | The same |

- **A mismatch is the first of a run when the last verdict is not "mismatch".** Only the
  first is recorded and pauses playback. Later ones are shown and nothing more.
- **Unjudged, mismatch and halt may carry a reason,** as text.
- **A claim made inside a frame stays.** Blame beats discard. A later match in the same
  frame does not release it.
- **When frame k's flags arrive:** claim none, they go onto the pile; discard, they are
  dropped; blame, they go to the failure.
- **A verdict between frames never touches the frame ahead.** Frame k has not run.

Other modes:

| Mode | match, unjudged, mismatch | halt | The frame's flags |
| --- | --- | --- | --- |
| Pre-roll | Ignored. The last verdict stays "none" | Recorded as a failure at that frame, with that frame's flags. Pause. The place asked for is not checked | Dropped |
| Comparison on, checking off | Ignored | Shown. Pause | Not taken for parity |
| Comparison off | No calls are made | | |

## 4. Every moment between frames

What dj does before the next frame runs, by what just happened. "Pre-roll: if set" means
the `[[compare]]` entry has `preroll` above zero; the default is none.

| # | What happened | Jump owed | Pre-roll | The pile | Last verdict |
| --- | --- | --- | --- | --- | --- |
| 1 | The next frame of the same segment, playing or stepping forward | No | | Kept | Kept |
| 2 | A parent's last frame, carrying on into a child: "Next" into the sticky or only child, or the end button | No | | Kept. It goes with the core into the child | Kept |
| 3 | A segment's last frame, mode "Stop". Paused at the end | No | | Kept while paused | Kept |
| 4 | From 3, a child is picked with the end button | No. As 2 | | Kept | Kept |
| 5 | From 3, a segment is selected in the tree, a child included | Yes: replaced state | If set | Emptied | None |
| 6 | A segment's last frame, mode "Loop" | Yes: replaced state | If set | Emptied | None |
| 7 | The ruler is being dragged | Yes, once, however many loads | Not yet | Emptied | None |
| 8 | The ruler is let go, or a seek | Yes: replaced state | If set | Emptied | None |
| 9 | Step back one frame | Yes: replaced state | If set | Emptied | None |
| 10 | A root segment's start is selected | Yes: power-on | No | Emptied | None |
| 11 | A segment is edited at or before the playhead, so the GUI reloads | Yes: replaced state | If set | Emptied | None |
| 12 | A `[RESET]` line in this frame | No | | Kept | Kept |
| 13 | A `[WRITE]` line with no address, and the core has `dj_handle_write` | No | | Kept | Kept |
| 14 | A `[WRITE]` line with an address, and the core has `dj_handle_write` | No. It is armed | | Kept | Kept |
| 15 | A `[WRITE]` line of either kind, and the core has no `dj_handle_write` | Yes: copy again, before the next frame | No | Emptied. With an address, the rest of that frame is unjudged too | None |
| 16 | An input or `[DEVICE]` line | No. They go through libretro | | Kept | Kept |
| 17 | The comparison is switched on | Yes: copy again | No | Empty | None |
| 18 | Parity checking is switched on, the comparison already on | No | No | Starts empty. dj clears the connector's flags | None |
| 19 | Parity checking is switched off | No | | Dropped | None |
| 20 | "Sync now" | Yes: copy again | No | Emptied | None |
| 21 | After the first mismatch of a run paused playback, play or step | No. The core stays as it is | | Empty already | Mismatch |
| 22 | After a halt, play or step | Yes: copy again | No | Empty already | None |
| 23 | The comparison core's library is rebuilt and loaded again | Yes: copy again | No | Emptied | None |
| 24 | `dj parity` starts a segment | Yes | If set, unless it is a root | Emptied. See below for the last segment's tail | None |
| 25 | A failure is re-run | Yes, at the frame the failure recorded | The same frames as before | Emptied | None |

- **"Copy again" needs no pre-roll.** Nothing was loaded, so there is no place to start
  earlier from. A core that copies at a tick is stale until its next one.
- **Case 5 is a load even when the child's start is the state the baseline is already
  in.** The GUI loads the stored state. Thread: leave it; picking the child with the end
  button (case 4) is the way to carry on.
- **Case 18 turns checking on with the core as it is.** If the core is stale it answers
  pending until it copies. Nothing is wrong, and nothing is credited until its first
  match.
- **Case 24 and the tail.** When the segment before ended with the last verdict a match,
  the frames still on the pile are its tail: the baseline was part-way through a tick
  the segment does not finish. `dj parity` does not count them as unchecked. When the
  last verdict was anything else, they count as unchecked.

## 5. What a core does, by kind

dj does not enforce any of this. It is what a correct core does, and what the helper
header does.

**A core that judges at frame ends** (no breakpoints; the Tetris decomp):

| Call | Stale | Live |
| --- | --- | --- |
| jump | Copy the baseline now. Answer unjudged. Live | The same |
| console reset | Pending | Run its own reset path. Pending |
| write, no address | Pending | Apply the bytes. Pending |
| write, with an address | Pending | It is not parked there. Go stale. Pending |
| frame end | Copy the baseline. Answer unjudged. Live | Run one frame. Compare: match, mismatch, or unjudged on a lag frame |

**A core that judges at a tick** (a breakpoint where a tick ends):

| Call | Stale | Live |
| --- | --- | --- |
| jump | Pending. It cannot copy between frames | Go stale. Pending |
| console reset | Pending | It is parked at its last tick, and the baseline was part-way through the next. Go stale. Pending |
| write, no address | Pending | The same reason. Go stale. Pending |
| write, at its own breakpoint's address | Pending | Apply the bytes. Pending. The break follows |
| write, at another address | Pending | Go stale. Pending |
| break, at its tick | Copy the baseline. Answer unjudged. Live | Run to the tick. Compare: match, mismatch or unjudged |
| break, at the input routine | Pending | Copy the pad buffer. Pending |
| break, in an overlay it does not know | Unjudged, with the reason | Go stale. Unjudged, with the reason |
| frame end | Pending | Pending |

- **"Go stale" is always followed by a copy at the next good call,** which answers
  unjudged. So what ran while the core was stale is never credited.
- **A stale core answers pending, not unjudged, at calls it does not act on.** The
  unjudged at its copy empties the pile for the whole stale stretch.
- **After a mismatch a core may do either:** carry on from its own state, or go stale
  and copy again. The helper header's choice is a setting of its own.

## 6. What writing this out turned up

Each is now in PLAN.md.

1. **"Unjudged" must not claim the frame ahead** (section 3, "Between"). The earlier rule
   had it discard "the frame it arrives in". A jump arrives before frame k, so a
   frame-end core's first frame after every jump earned nothing.
2. **A console reset is like a write with no address** (section 5). A tick core is parked
   at its last tick while the baseline is part-way through the next, so it cannot run
   its reset path from the right state. Only a core parked at a frame boundary can.
3. **Pausing at every mismatch made play useless** after the first. The developer's rule
   fixes it: only the first of a run pauses and is recorded (section 3, case 21).
4. **The last verdict is "none" at T** (section 1). Otherwise a mismatch in pre-roll hid
   the first real one.
5. **The tail of a segment** (case 24). A tick core's last frames in a segment can have
   no verdict. The developer's rule: none is expected, so they are not reported.
6. **Turning checking on must clear the connector's flags** (case 18), or the first frame
   carries what ran before.
7. **A mismatch can arrive between frames** (section 3). It is rare, but the rules need
   an answer: pause before the frame runs.
8. **A core that cannot follow writes needs a rule** (case 15). Without `dj_handle_write`
   the core does not know its state went wrong, so dj sends it a jump.

## 7. Worked cases

Each row is one call. "Pile" is what is held after it.

### A. A frame-end core, after a seek to frame T

No pre-roll: the default.

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| Before T | jump | Copies. Unjudged | empty | The jump emptied it already. Frame T is untouched |
| End of T | frame end | Runs a frame. Match | empty | Frame T's flags went on, then were credited |

The first frame after the jump is checked, as today.

### B. A tick core under `dj parity`, starting segment S, with `preroll = 2`

Ticks end inside frames 97 and 99 of the parent, and inside frames 1 and 3 of S. The
parent has 100 frames, and S has 5. The nearest stored snapshot is at the parent's
frame 60.

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| | | | | dj loads frame 60 and plays to frame 98 with no calls |
| Before parent 98 | jump, pre-roll | Stale. Pending | empty | The jump is 2 frames before S, whatever was loaded |
| End of parent 98 | frame end, pre-roll | Pending | empty | Frame 98's flags are dropped |
| Inside parent 99 | break, pre-roll | Copies. Unjudged | empty | Ignored: pre-roll |
| End of parent 99 | frame end, pre-roll | Pending | empty | Frame 99's flags are dropped |
| End of S 0 | frame end | Pending | S0 | |
| Inside S 1 | break | Runs to the tick. Match | empty | Credits S0. Checks the code from the copy to here |
| End of S 1 | frame end | Pending | S1 | |
| End of S 2 | frame end | Pending | S1, S2 | |
| Inside S 3 | break | Match | empty | Credits S1 and S2 |
| End of S 3 | frame end | Pending | S3 | |
| End of S 4 | frame end | Pending | S3, S4 | The segment ends part-way through a tick |
| Before the next segment | jump | | empty | S3 and S4 are the tail: the last verdict was a match, so they are not reported |

A failure in S records the jump at parent frame 98, whatever snapshots the machine has.
With `preroll` left at zero, the jump would be at S 0, the core would copy inside S 1,
and S 0 and S 1 would count as skipped.

### C. A tick that crosses from a parent into its child, playing straight on

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| End of the parent's last frame | frame end | Pending | parent's last | The baseline is part-way through a tick |
| | | | parent's last | No jump. If the mode is "Stop", everything waits here |
| Inside child 0 | break | Runs to the tick. Match | empty | Credits the parent's last frame |

### D. A mismatch, then play

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| Inside k | break | Mismatch | empty | The first of a run: recorded, with the pile. Frame k is claimed: blame |
| End of k | frame end | Pending | empty | Frame k's flags go to the failure. Playback pauses |
| | | | | The user presses **play**. No jump |
| Inside k+2 | break | Mismatch | empty | Later in the run: shown, not kept. Frame k+2 is discarded. Playback goes on |
| Inside k+4 | break | Match | empty | The core has put itself right. Last verdict: match |
| Inside k+6 | break | Match | empty | Credits k+5. Parity again |
| Inside k+20 | break | Mismatch | empty | A new run: recorded, and playback pauses when k+20 ends |

If the core never puts itself right, the user picks "Sync now": a jump, the core copies
at its next tick and answers unjudged, and parity can start again.

### E. A write at an instruction address, at the core's own breakpoint

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| Before k | | | | The write is armed. No call |
| Inside k | write | Applies the bytes. Pending | unchanged | dj wrote the baseline first |
| Inside k | break | Runs to the tick. Match | empty | The same address, so this follows at once |

If the address is not reached in frame k: a warning in the console, no write, no call.

### F. A lag frame, for a frame-end core

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| End of k | frame end | Unjudged, "lag frame" | empty | Frame k counts as skipped. The core may go stale and copy at the next frame end |

### G. An overlay the tick core does not know

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| Inside k | break | Goes stale. Unjudged, "overlay not decompiled" | empty | Frame k is discarded |
| End of k ... | frame end | Pending | grows | |
| Inside m | break, still unknown | Unjudged | empty | Everything since is dropped |
| Inside n | break, known again | Copies. Unjudged | empty | |
| Inside n+2 | break | Match | empty | Parity again |

Frames k to n count as skipped, and the tools can say why.

### H. A halt during pre-roll

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| Inside parent 99 | break, pre-roll | Halt | empty | A failure at parent frame 99. Playback pauses |

`dj parity` reports segment S as not checked, with the halt's reason, and counts it as
failed.

### I. A write, for a core with no `dj_handle_write`

| Place | Call | The core | Pile | Note |
| --- | --- | --- | --- | --- |
| Before k | | | emptied | dj writes the baseline. The core is not called. A jump is owed |
| Before k | jump, copy again | Frame-end core: copies. Unjudged | empty | Sent at once, since this is between frames |
| End of k | frame end | Match | empty | Frame k is checked |

For a write with an address, the core's state is wrong from the middle of frame k. dj
treats the rest of frame k as unjudged and sends the jump before frame k+1.
