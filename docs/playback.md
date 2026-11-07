# Playback and state hashes

How a player turns segments into emulator runs, and how hashes prove the runs still match
what was recorded. This is normative for format version 1.

## Playing a segment

To play segment `R`:

1. Walk `parent` links from `R` up to its root segment.
2. Load the ROM, apply core options and port devices from `config.toml`.
3. If the root segment has `snapshot`, `retro_unserialize` it. If it has `sram`, copy it
   into save RAM.
4. For each segment from the root segment down to `R`: check `start-hash` if present,
   apply each frame's lines and call `retro_run()`, check `checkpoint` and
   `end-hash` if present.

Players may skip step 4 for ancestors by loading a cached local state taken
at an ancestor's end. They may do so only if that state was produced by this
same setup, its hash matches the ancestor's `end-hash`, and the ancestor is
clean (see [below](#clean-and-dirty)), so nothing above it has changed since. Otherwise they play
the chain from the root segment.

A player that has just finished a segment may go straight on into one of its
children without reloading anything. The core is already in the child's start
state; the player only releases all inputs and sets port devices for the child
([frames](segments.md#frames)).

## One pinned setup

State hashes are only meaningful for one core build, one ROM, and one set of core
options, all pinned in [config.toml](config.md). A different build is a different
project, or a re-validation of this one (see [below](#clean-and-dirty)).
`dj init` records every core option's current value, not only the ones you changed, so
the setup stays pinned when a core's defaults change.

## State hashes

A state hash identifies "where the game is" without shipping memory.
Every hash in the format (state hashes, checkpoints and blob names) is the first 32
hex digits (128 bits) of a BLAKE3 digest, with no prefix: the format version pins the
algorithm, so changing it would be a new version. A state hash is the digest of, for
each region in `[hash].regions` in order:

```
region name (ASCII) | 0x00 | size as u64 little-endian | region bytes
```

Only memory the core exposes through `retro_get_memory_data` (or the
memory maps, for `BUS:<start>-<end>` ranges) can be hashed. System RAM is
the usual choice. Snapshot blobs themselves are not hashed for this
purpose, because they hold emulator internals (timers, audio buffers) that
change with core versions even when the game is in the same place.

A segment's `start-hash` must equal its parent's `end-hash`.

## Logic ticks (draft)

Frame numbers are the clock, but they are not always the game's clock. Games
that run their logic once per vertical blank (most 8- and 16-bit games) have
one logic tick per frame. Others (most N64 games, and any game that lags) run
logic at their own rate, so "frame 812" does not mean "tick 812".

A project can declare what counts as one logic tick:

```toml
[tick]
exec = "PRG:0x1F3A"       # a tick is each execution of this ROM location
# or (reserved): write = "RAM:0x0021"  # each write to this address
```

The coverage connector counts ticks while the core runs. Tools report ticks
next to frames (`frame 812, tick 790`), so events and parity checks can be
lined up by tick. In the Thwaite example, the tick is `doStateAction`: menus
run 0 ticks, and a 300-frame segment that pauses the game runs 130.

Only `exec` is implemented in v1; `write` is **(reserved)**.

Input lines stay keyed by frame in v1, because that is what a core consumes. Keying input by tick is **(reserved)** for a later version
once it has been tried on a multithreaded game.

## Clean and dirty

A segment is **clean** if it has an `end-hash` and its `start-hash` (if any) equals its
parent's `end-hash`, and its parent is clean. Otherwise it is **dirty**:

- it has no `end-hash` (it is new, or was edited and not played to its end since), or
- its `start-hash` differs from its parent's current `end-hash` (the parent was edited
  and re-recorded), or
- a segment above it is dirty: its start state rests on something that has changed.

Clean and dirty are never stored. Tools work them out from the hashes. Cleaning a dirty
segment means playing it from its parent's end state and recording the hashes it reaches
([below](#recording-hashes)). Its children may then be dirty in turn, and so on down the
tree.

When a segment is edited, tools remove its `end-hash` and every `checkpoint` after the
first edited frame, because those states are no longer known. The segment and everything
below it are dirty until it is played to its end and the new hashes are recorded. Editing
only `[EVENT]` lines changes no state, so it keeps every hash.

Saving a dirty segment is allowed, but the GUI offers to clean first: a saved dirty
segment says only that it hasn't been checked. Hand-edited hashes that agree with each
other, or an input edit that kept the old hashes, look clean from the files alone; only a
play (`dj run`, `dj clean`) proves them.

## Recording hashes

Playing a segment to its end records its hashes, as long as its start is trusted: it
played from where its parent ends, and the parent is clean (a root segment starts where it
starts). Then:

- With no `end-hash` yet, its `start-hash`, `end-hash` and `start-frame` are recorded.
- If it still ends with its recorded `end-hash` and checkpoints, only its new `start-hash`
  is recorded: it is **cleaned**.
- If the end hash or a checkpoint differs, the new hashes are recorded anyway: its
  **ending changed**. The tools don't judge whether that matters (an upstream change has
  clear consequences, and the author knows whether it was intended). The GUI marks a
  segment whose unsaved ending differs from its saved file with a **!** next to its dot
  until it is saved; undo or reload takes the change back. `dj clean` lists them.

A segment below one that isn't clean waits for it.

In the GUI these are unsaved changes, like edits: File > Save (Ctrl/Cmd+S) keeps them. On
the command line `dj clean` writes them to the files (changed endings only with `--accept`).

## Checking hashes

From the command line:

- `dj play <dir> <id>` plays one segment and its chain. `--accept` writes the new hashes.
- `dj run <dir>` plays every segment and exits with an error when a segment's hashes differ from the recorded ones. `--accept` writes the new hashes instead.
- `dj run --from-scratch` plays every chain from power-on instead of from cached parent states, which checks that snapshots round-trip exactly.
- `dj status <dir>` prints the tree with clean and dirty marks: `?` for a dirty segment with no recorded hashes, `~` for one that starts somewhere its parent no longer ends (or below such a segment).
- `dj clean <dir> [segment]` plays every segment that isn't clean or whose content isn't in the local database (or that segment and those below it; `--all` plays every one), top down, and [records](#recording-hashes) the hashes each reaches in its file. The command line has no **!** to review, so a changed ending is listed and not written, the segments below it wait, and the command exits with an error. `--accept` writes changed endings too and carries on below them, as `--accept` does for `dj play` and `dj run`.

In the GUI, the segment tree shows clean (green) and dirty (orange) dots. Reaching a
segment's last frame [records its hashes](#recording-hashes) as an unsaved change.
**Segment > Clean all segments** (or **Clean this and below** on a segment's menu) does
the same for every segment that isn't clean, top down.
