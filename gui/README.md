# dj-gui

A desktop viewer and editor for decomp-dj projects, built with egui. How to use it is in
[docs/gui.md](../docs/gui.md), with editing in [docs/editing.md](../docs/editing.md) and
the coverage views in [docs/coverage.md](../docs/coverage.md#in-the-gui). This file is
about the code.

| File | What it does |
| --- | --- |
| `src/main.rs` | The app: menus, segment tree, preview, playback and seeking, applying edits |
| `src/roll.rs` | The piano roll: building rows from a segment, drawing, scrolling, zoom, drag to edit |
| `src/cov.rs` | The Coverage tab, the ROM map and live coverage |

The emulation itself goes through the `decomp_dj` library in [player/](../player/)
(`Session::enter_replay`, `restore_inputs`, `step`, `snapshot`), so the GUI and the
CLI play segments the same way. Every recorded Thwaite end hash matches when it is
played in the GUI.

## Seeking

- Video frames are not cached. While a segment plays, the GUI keeps a snapshot in memory at the start of every 10th frame (`SNAP_EVERY`).
- A seek uses the nearest snapshot at or before the target, from memory or from the local database. It sets the held inputs again, because input state is not part of a snapshot, then runs the core forward and draws only the last frame.
- Long work runs in slices of about 25 ms per screen refresh (`JOB_SLICE`), so the window keeps drawing. This covers a seek more than 30 frames past the last run frame, and selecting a segment whose ancestors have no stored state (the GUI plays the chain). Clicking another segment cancels the job; a seek during a segment load is applied once the load finishes.
- In-memory snapshots stay under `max_memory_mb`; past it, the ones farthest from the playhead go first. Frame 0 always stays, and anchors every 300 frames stay while they fit.

## Hydrating

Emulation > Hydrate all runs the shared `decomp_dj::hydrate::Hydrator` in the same 25 ms
slices, showing every 4th frame. Afterwards the selected segment is loaded again.

## Undo and redo

Every edit records the affected segments' full text before the change as one undo step
(`remember` in `src/main.rs`). Undo puts those segments back (removing segments the step
created) and records the reverse step for redo. After an undo on the segment being shown,
playback drops its snapshots from the first changed frame on and redraws where you are. If
the step changed the segment's length or parent, another segment, or one of its ancestors,
the segment is loaded again instead.

Deleting removes the whole branch (`delete_nodes`) as one undo step.

## Coverage

The Coverage tab shows `decomp_dj::coverage::build_report`, the same report as
`dj summary`, rebuilt after any edit, undo, save or hydration. Live coverage reads the
connector's flags on every screen refresh and then clears them, so they say what ran
since the last refresh. The flags are OR'd into a running total that restarts when a
segment starts playing, when playback continues into a child and after an edit.
