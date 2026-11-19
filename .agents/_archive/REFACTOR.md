# Refactor: fewer, sharper ideas

The tools grew feature by feature, and several now overlap: segments can be dirty,
unverified, "partially unverified" (the ruler band), unaccepted, unhydrated or drafts, and
coverage can be counted, live or "new until hydrated". Each made sense when it was added.
Together they are hard to hold in your head. This note sets out a smaller model and maps
every current feature onto it: keep, merge or drop.

It is a plan, not a spec. Where it changes format version 1 it says so; most of it doesn't.

**Status: carried out** (every step in [Order of work](#order-of-work)). The docs describe
the result; this note stays as the record of why, with what turned out differently noted
in place.

## The model: four rules

1. **Segment files are the only truth.** What is in `segments/` (plus `config.toml`,
   `blobs/`, symbol maps and `unreachable.txt`) is the project. Anything in the editor that
   isn't saved yet, your edits or an agent's work, is an unsaved change, and Save is the one
   way it becomes part of the project.
2. **The database is a cache.** Every row is a fact worked out from (setup, start state,
   inputs), so the same key always gives the same answer. Any clean pass fills it: watching
   in the GUI, Clean all segments, the CLI, an agent. It is written the moment a fact is known, never
   "saved", never rolled back, and can always be deleted and rebuilt.
3. **Hashes record themselves.** Playing a segment to its end records its hashes when that
   overwrites nothing. The only hash decision a person makes is whether a changed ending is
   intended.
4. **Coverage is a function of the current tree.** Covered = run by a clean, saved segment,
   looked up in the cache. What unsaved segments add shows at once as new (yellow-green) and
   counts when saved. There is no separate "new until hydrated" state.

Everything below follows from these.

## Concepts, before and after

| Concept | Today | After |
| --- | --- | --- |
| Clean / dirty / unverified | Worked out from hashes. Leaving dirty took Accept or Hydrate and clean | Two states, clean and dirty: unverified folds into dirty. Reaching the end records hashes ([playback.md](../docs/playback.md#recording-hashes)), so dirty mostly clears itself |
| Accept end hash | Manual step after every edit | Gone: endings are recorded automatically, and an unsaved changed ending shows **!** (Use new end hash is an interim step, done) |
| Ruler band ("partially unverified") | Frames not re-run since an edit; vanished on leaving the segment | The dirty part of the segment, in the tree dot's colour (done) |
| Hydrate | The only way coverage and snapshots reach the database | "Fill what's missing": a convenience, since playback fills the cache too |
| Hydrate and clean all | Writes start hashes to files; needs nothing unsaved | **Dropped**, replaced by Clean all segments (done) |
| Clean all segments | New, at the end of the Segment menu with Hydrate all | Plays what isn't clean, records hashes as unsaved changes (done) |
| Live coverage | Connector flags, in memory, lost on leaving the segment | The progress of the current pass. It becomes cached coverage when the pass completes |
| "New" coverage | Played but not hydrated | Run by this pass, not by any segment in the tree |
| Drafts | A parallel segment kind: own folder, own colour, own Save as segment, own `--allow save` | Unsaved segments that came from an agent. Save, Delete and Undo work on them as on anything else |
| `--allow save` | Lets an agent write `segments/` | Probably dropped: the person's Save is the gate |

## Feature map

### Hashes and status

- **Keep** the names clean and dirty, worked out from hashes, never stored. Unverified (no
  end hash) folds into dirty: see [Status](#status-two-axes).
- **Keep** checkpoints as they are.
- **Done:** auto-record at the end of a play, Use new end hash, the ruler band in the
  status colour, the transport label showing the segment's status, and Clean all segments /
  Clean this and below.
- **Decided: the tool doesn't judge upstream changes.** Making one has clear consequences,
  and whether a changed ending matters is the author's call. So a changed ending is
  recorded like any other (playing to the end or cleaning records it, as an unsaved change)
  and **flagged**: an unsaved segment whose end hash differs from its saved file shows a
  **!** next to its dot (the mark may get milder later). Saving clears it; undo or reload
  takes the change back. This replaces "Use new end hash" and the *ending changed* stop:
  cleaning carries on down the tree through changed endings, flagging each one.
- **Later, if wanted:** a nudge to branch instead of editing a segment that has children;
  and optional per-segment goals (`expect: RAM:level == 2`, or an event that must hold,
  using the MCP's conditions) that make a missed goal a real failure.

### Database and hydration

- **Change: GUI playback records.** A pass that runs from frame 0 to the end without a
  backwards seek, an edit or a jump in from a stored snapshot runs through `Recorder`
  instead of bare `Session::step`. It commits snapshots, previews, coverage and first hits
  when it finishes, just as hydrating does. A broken pass stays live-only.
- **Change: snapshots from any pass.** The GUI's in-memory snapshots (every 10 frames, for
  scrubbing) and the recorder's (every `diff_every` frames, at markers, the end state) share
  one capture where they land on the same frame. A pass broken by a backwards seek or a jump
  still commits the snapshots it took: each is a valid state of this content from this start,
  whether or not the pass finished. Only coverage and first hits need the whole pass, so
  only they are dropped. Frames after an edit belong to the new content and are stored
  under its key.
- **Change: one reader of the connector.** Today the live view reads and clears the
  connector's flags every refresh, and `Recorder` expects to own them. The recorder becomes
  the only reader, and the live view reads its running total.
  *As built:* the recorder gathers the flags into its own running total after every frame,
  so the live view may go on reading and clearing them each refresh (it needs "what ran
  since the last refresh" for the activity glow). Neither disturbs the other.
- **Keep** Hydrate all and Hydrate this / this and below, described as filling the cache.
- **Drop** `dj hydrate --clean` in favour of a CLI `dj clean` that uses the same rule as
  the GUI (`try_record` in `player/src/hydrate.rs`) and writes files, since the CLI has no
  unsaved state.
- **Keep** the database as one file on disk in WAL mode. An in-memory database persisted on
  Save was considered and rejected: it adds a second save point and a second copy of the
  truth, and the cache never needs undoing.

### Coverage

- **Change:** the report reads the segments in the editor, saved or not, so an unsaved
  segment's coverage shows at once (yellow-green where it is new), and deleting it takes its
  coverage with it. It counts towards the totals when saved.
- **Decided: dirty segments don't count.** Covered means run by a clean segment, as today.
- **Change: one colour per meaning on the map and function list.** In order of precedence
  where a unit is run by several segments:
  - **Green:** run by a clean, saved segment. Counted.
  - **Yellow-green** (today's live colour, `cov::LIVE`): new, not counted until saved: run by an unsaved segment (or the live
    pass being played) and by no saved clean segment. Unsaved coverage that a saved
    segment already runs is just green, so no further distinction is needed.
  - **Dark, faded green:** run only by dirty segments. Not counted; cleaning them brings it
    back. Found from the cache (a dirty segment's last pass), without playing anything.
  - **Grey:** never run. Blue: unreachable, as today.

  The totals show the yellow-green and faded parts apart ("+N unsaved", "+N if cleaned").
  Nothing on the map uses a pure yellow, which would read as a warning.
- **Roadmap:** tracking coverage lost ([ROADMAP.md](ROADMAP.md#coverage)).

### Agents and drafts

- **Keep** `.local/drafts/` as the hand-off between `dj mcp` and the GUI. It works across
  processes, and a fuzz job can make 20 at once.
- **Change:** the GUI shows drafts as unsaved segments, styled as unsaved with an agent
  mark rather than as a separate kind. File > Save moves them into `segments/` along with
  your own edits, and recording happens as for any segment. Deleting one is a normal delete.
- **Drop** Save as segment, Discard draft as a special action, and probably `--allow save`
  and `save_draft` (keep it only if headless agent runs need to commit on their own).
  *As built:* Save as segment and Discard draft are gone. `--allow save` and `save_draft`
  stay for now: they are opt-in, and a headless run (fuzzing with no GUI open) has no other
  way to commit. Dropping them is a one-line decision later.
- **Keep** following a session and sharing your view.

### Watching files

- **Done:** the GUI picks up segment files changed by other tools, and warns when one with
  unsaved edits here changed on disk.

## Status: two axes

Two independent axes, each shown on its own:

- **Clean / dirty** (today's dirty and unverified become one: dirty). Clean: the
  segment has an end hash and starts where its parent ends. Playing or cleaning settles
  it. Separately, an unsaved segment whose ending differs from its saved file shows **!**:
  not a status, a flag for the author ([Hashes and status](#hashes-and-status)).
- **Cached / uncached** (today's hydrated). Cached: a clean pass of this content from this
  start is in the database, with its coverage and snapshots.

All four combinations occur: clean and uncached is a fresh clone; dirty and cached
is a child played from its parent's new end that ends somewhere new.

**Saving dirty segments is discouraged, not blocked.** Save offers to clean what the
save would leave dirty (the edited segments and everything below them), and says what
stays dirty if you save anyway. *As built:* the Save dialog offers **Clean, then save**
(Clean all segments, then Save) and **Save anyway**.

**What files alone can't show.** Status is worked out by comparing hashes between files,
so it catches a save made before cleaning, but not hand-edited hashes that agree with each
other, an input edit that kept the old hashes, or a child with no `start-hash`. On opening,
the GUI should also check the cache: a stored pass of this exact content and start whose
end differs from the file flagged without playing anything, and clean but
uncached content is worth a quiet "not checked on this machine" mark. Full proof is still a
play (`dj run`, or Clean all segments with "check everything").

## Order of work

1. Done: fold unverified into dirty in the tree, counts, `dj status` and docs. Record
   changed endings automatically and flag them with **!**; drop Use new end hash.
   (`dj status` marks dirty segments `?`, no hashes recorded, or `~`, starts somewhere its
   parent no longer ends, so that **!** keeps one meaning.)
2. Done: GUI playback records through `Recorder` (see the connector note above).
3. Done: coverage over the editor's tree (rule 4), with the colours above
   (`coverage::build_report_with`, `Totals::unsaved_new`, `Totals::if_cleaned`).
4. Done: drafts as unsaved segments; Save as segment and Discard draft dropped
   (`--allow save` kept, see above). Clean all segments cleans drafts too, except those
   open in an agent session.
5. Done: `dj clean` on the CLI, replacing `dj hydrate --clean`.
6. Done: docs, concepts.md first.

Not done, from [What files alone can't show](#status-two-axes): checking the cache on
opening for stored passes whose end differs from the file, and a "not checked on this
machine" mark.

## Follow-up notes

- **Play straight through changed endings.** Next/List must carry on whenever the child is
  reachable, saved or not (today they stop at anything unsaved, and recording hashes counts
  as unsaved). An opt-in "Stop at a changed ending" toggle is on the
  [roadmap](ROADMAP.md#gui).
