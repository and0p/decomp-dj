# Plan: the local database

Status: mostly implemented (see "What's built" at the end). This file covers the local database: a local store for everything derived from playing a
project. It uses the words defined in [concepts.md](../docs/concepts.md#terms):
segment, snapshot, diff, child segment, save and macro. How to use the database is in
[database.md](../docs/database.md), and its settings in [config.md](../docs/config.md#cache-optional).

## What it holds

Everything here can be rebuilt by playing the project, so none of it goes into
version control. Each author or collaborator fills it on their own machine
("hydrates" it). Here is what it holds:

- **Snapshots**, so the GUI and CLI can start a segment, or seek inside one, without
  playing every parent first.
- **Diffs**, so RAM at more frames can be rebuilt cheaply.
- **Previews**: PNGs of each segment's start and of each marker's frame.
- **Coverage**: blocks and instructions for each segment.
- **Cross references** from code to segments. An agent decompiling a routine can ask
  "which segment first runs this block, at which frame, and where is the nearest
  snapshot before it?", then load that snapshot and trace from there.

Saves (SRAM, memory cards, cartridge saves) that a root segment starts from stay
in `blobs/` ([segments.md](../docs/segments.md#blobs)), which is in version control. A save at the end of a
segment is derived, so it goes in the database. A tool can export it to `blobs/` when
someone wants to start a new root segment from it.

**The database is optional.** Without one, the tools work as they do today: they
play from power-on, or from the nearest snapshot in memory. Everything below speeds
things up, and nothing depends on it.

## Storage: SQLite (decided)

There is no server. Everything lives in one `.local/cache.db` per project.
It replaced the earlier `.local/coverage.sqlite` and `.local/states/` folder.

- **No setup for collaborators.** It's a file inside the project's `.local/`
  folder, so it can't point at the wrong project, and deleting the folder resets
  it.
- **Agents can read it directly.** Python's standard library, the `sqlite3` command
  line tool, and every language's drivers can open it. `dj sql "<query>"`
  runs a query, and an MCP tool can come later. In WAL mode, the GUI, a hydrate run and an
  agent can all read while one of them writes.
- **No pile of small files.** Pages, previews and diffs are rows. SQLite reads and
  writes blobs under about 100 KB faster than separate files do. Pages are small (64 KB at most), so
  everything fits in the database, with no separate blob folder.
- **Sharing a filled cache** is a `cache pack` / `unpack` bundle, so a collaborator
  doesn't have to replay everything.

DuckDB can attach the file later for heavy coverage analytics, with no second copy of
the data.

## How rows are keyed: content keys, not segment ids

What a segment produces depends on three things: the state it starts from, its own
lines, and the setup (ROM, core and core options, which `Session` already hashes
into a setup id). So derived rows are keyed by a **content key**:

```
content key = blake3(setup id, start-hash, normalized body of the segment)
```

The normalized body is the frame count and the lines, without the title, tags or
comments. This key has some useful properties:

- **Renaming, retagging or moving a segment invalidates nothing.**
- **An edit invalidates exactly what changed.** The edited segment gets a new key,
  and its child segments get new keys too, because their start-hash changes. No
  dirty tracking is needed.
- **Identical segments are stored once,** for example when two collaborators record
  the same menu path.
- **Undo is free.** Undoing brings back the old key, and the old rows are still
  there.

A small `segments_now` table maps the project's current segment ids to content
keys. It's rebuilt whenever the project is opened, and queries join through it.

## Snapshots, pages and diffs

**Pages are stored once, by hash, in the database.** A snapshot (the core's
serialized state) is cut into pages of `page_kb` (set per project). If `page_kb`
is missing, or is at least the snapshot's size, page and snapshot are 1:1: the whole
snapshot is one page, which suits the NES and its few-KB states. Each page is hashed with blake3 and
stored once in a `pages` table, compressed with zstd. A snapshot itself is just a
**manifest**: the list of its page hashes plus a few header fields. An 8 MB N64
snapshot is 128 pages, so its manifest is about 4 KB. Code, static data and
textures are the same from one snapshot to the next, so most pages are shared
across thousands of snapshots, and each new snapshot only adds the pages that
actually changed.

**Diffs come almost for free on top of pages.** A diff is a manifest that names the
snapshot it follows, and stores only the pages that changed since then, with no
copy of the unchanged ones.
- Each changed page can also be stored as a zstd patch against the previous version
  of the same page (zstd's `--patch-from` mode), instead of as the whole page. A page
  where a few counters moved becomes a few hundred bytes.
- Rebuilding a frame means taking the segment's start snapshot, applying the diffs
  in order, and loading the result into the core.
- A full manifest is written every few diffs, so a rebuild never has to walk a long
  chain.

**What gets captured is set per project:**

- **The start of every segment** gets a snapshot. The end of a segment is only its
  `end-hash`, since its child segments start from exactly that state, and snapshots
  are keyed by state hash, so it is stored once however many children it has. A
  segment with no children doesn't get an end snapshot. Reaching its end means
  playing it from its start.
- **A diff every N frames** inside long segments (`diff_every`). This caps how far
  a seek has to play.
- **A diff at each marker** (`diff_at_markers`), so the moments authors care about
  load instantly.
- **Excluded ranges** (`exclude`): ranges such as framebuffers, audio buffers and
  display lists that change every frame and say nothing about game logic. They're
  zeroed before hashing pages, so they don't create new pages. Their contents are
  regenerated on the next frame after a load.
- **Traced windows**: per-frame history for a short stretch, such as the 10
  seconds before a block first runs. These are recorded on request, never during
  hydration.

## Tables (first draft)

| Table | Holds |
|---|---|
| `setups` | setup id, ROM sha256, core name and version, core options |
| `contents` | content key, setup, start hash, end hash, frames, when it was played |
| `segments_now` | segment id → content key, for the project as it is now |
| `pages` | page hash → zstd bytes, or a zstd patch plus the hash of the page it patches |
| `snapshots` | state hash → setup, manifest (page hashes), size |
| `diffs` | content key, frame, base (a snapshot or the previous diff), manifest of changed pages |
| `images` | content key, frame, role (`start`, `marker`, `screenshot`), PNG |
| `coverage` | content key, region, covered ranges (today's table, re-keyed) |
| `coverage_sum` | function or block → how many current clean segments cover it, and the first one (a cache, see "Coverage") |
| `unreachable` | ROM ranges loaded from the project's `unreachable.txt`, with their comments |
| `parity` | comparison core, content key, block → result (`match`, `differs`), first differing frame, evidence |
| `first_hits` | content key, ROM offset, first frame it ran, CPU address, bank or overlay |
| `code` | ROM offset → instruction, block id, function (from the symbol map and the sweep) |
| `runs` | when hydrate or play runs happened (today's table) |

**Cross references.** The coverage connector already reports which ROM bytes ran.
Hydration also records the frame each offset *first* ran within a segment. That
gives `first_hits`, and with `diffs` an agent can ask:

```sql
-- Where does block 0x1C3A0 first run, and what's the nearest stored frame before it?
SELECT s.segment_id, h.frame,
       (SELECT max(frame) FROM diffs d WHERE d.content = h.content AND d.frame <= h.frame) AS from_frame
FROM first_hits h JOIN segments_now s ON s.content = h.content
WHERE h.rom_offset = 0x1C3A0
ORDER BY h.frame LIMIT 5;
```

A `from_frame` of NULL means the segment's start snapshot. CLI wrappers cover
the common questions:

- `dj where <symbol|offset>` lists the segments and frames that first run it.
- `dj snapshot <segment> <frame> --out f.state` writes a snapshot to start a
  trace from.
- `dj sql` runs anything else.

## The cache config

Each project sets thresholds in a `[cache]` section of `config.toml`, so
everyone hydrating it gets the same layout. `.local/local.toml` can override any
value on one machine. Later the tool can ship presets per system, in sparse and
aggressive variants. A project names a preset and overrides single values:

```toml
[cache]
preset = "n64-sparse"          # shipped defaults; the values below override it
diff_every = 18000             # frames between diffs inside a segment; 0 = none
diff_at_markers = true
exclude = ["RDRAM:0x00300000-0x003FFFFF"]   # e.g. framebuffers
page_kb = 64                   # missing, or >= snapshot size = 1:1 (one page per snapshot)
previews = "thumb"             # thumb (half width and height) | full | off
preview_at = ["start", "marker"]
budget_gb = 20                 # cache gc trims unused content past this
```

| Preset | Pages | Diffs inside segments | Previews |
|---|---|---|---|
| `nes-aggressive` | 1:1 | every 60 frames and at markers | full |
| `nes-sparse` | 1:1 | at markers | thumb |
| `n64-aggressive` | 64 KB | every 1,800 frames and at markers | thumb |
| `n64-sparse` | 64 KB | every 18,000 frames and at markers | thumb |

**Choosing values.** There's no profiling command for now. Choosing values is up to
the author, who weighs:

- **Snapshot size.** Small states (NES) go 1:1. Large states (N64) get pages, so
  unchanged memory is shared.
- **Seek distance.** The time to play `diff_every` frames headless is the longest a
  seek can take.
- **Memory that churns every frame.** Framebuffers, audio and display lists go in
  `exclude`.
- **Disk.** The estimates below give a sense of scale.

## Size, for a feel

**N64, sparse.** This is the ceiling case: 8 MB RDRAM with the Expansion Pak, and
paths up to 90 minutes long (324,000 frames at 60 Hz). Take a tree of 500 segments
averaging 2 minutes, with 30 hours of play in total. These are estimates, and
real numbers depend on the game and core:

- **Snapshots and diffs:** compressed with zstd, the first snapshot is about 1–3 MB.
  Later snapshots and diffs only add the pages that changed. If a 5-minute gap
  changes a third of RDRAM after exclusions, that's about 0.5–1 MB each. Add 500
  segment starts and about 360 interval diffs, and the total is likely 0.4–1 GB.
- **Previews:** thumbnails (160×120) are about 15 KB each. 500 starts plus 2,000
  markers come to about 40 MB.
- **Coverage and first hits:** stored as covered ranges, not one byte per ROM byte.
  This is tens of KB per segment, or tens of MB in all.
- **Total:** about 0.5–1.5 GB.

**N64, aggressive.** A diff every 30 seconds comes to roughly 5–15 GB. Per-frame
history for a whole 90-minute path isn't practical even with excluded ranges, so it
is recorded only for traced windows.

**NES:** a tree of 1,000 segments of 1,000 frames each is in the low hundreds of MB
even with the aggressive preset.

**Seek time is the trade-off of sparse settings.** A seek plays forward from the
nearest stored frame. With a diff every 5 minutes, that's at most 18,000 frames,
about 30–60 seconds on a fast N64 core running headless. The GUI's progress bar
already covers it. An author who finds it slow can lower `diff_every`.

**N64 coverage needs one more piece.** N64 code runs from RDRAM after the cart DMAs
it there, and overlays reuse the same addresses. So the connector has to track PI
DMA to turn an executed RDRAM address back into a ROM offset (or an overlay and an
address). That's a design item for the N64 connector.

## The GUI: memory first, then the database

The GUI keeps snapshots in memory, one every 10 frames for the current segment
(gui/README, "How seeking works now"), and starts a segment from its parent's stored
end state when that parent is clean. These are three levels, checked in order:

1. **Snapshots in memory**, capped by `max_memory_mb` in `.local/local.toml` (512 MB
   by default). Past the cap, the snapshots farthest from the playhead go first.
2. **The database.** On a memory miss, the GUI rebuilds the nearest stored frame
   from pages and diffs, and keeps the decoded result in memory.
3. **Playing.** The GUI plays forward from whichever of the two is nearest, and
   only from power-on if neither has anything.

When a project is hydrated, selecting a segment loads its start snapshot. It
never plays the parent segments.

## Hydrating

Hydration fills the database. It can be started from the CLI
(`dj hydrate`) or from the GUI, and in both:

- **It can cover part of the tree:** the whole tree, **this segment**, or **this
  segment and all its child segments**. Partial hydration is normal. What's missing
  is filled in later, or played when needed.
- **It can be paused, resumed or stopped at any time.** Each finished segment is
  committed on its own, so stopping loses at most the segment in progress.
- **It can be watched live.** The GUI shows the segment being hydrated in the
  emulator view with its piano roll, and the tree marks each segment as it finishes.
  The CLI prints progress.
- **"Hydrate and clean"** also re-plays dirty segments. For each one, if it matches
  its recorded hashes it becomes clean. If it doesn't, hydration records which
  checkpoint or end hash failed and moves on to the rest of the tree, rather than
  accepting the new hashes. The author reviews the mismatches afterwards.
- **Only missing content is played.** Content keys that are already stored are
  skipped. Separate subtrees can run in parallel processes, since each core runs in
  its own process.

`dj cache gc` drops content that no current segment uses, oldest first, until
the store is under `budget_gb`. Then it drops pages that no snapshot or diff refers
to.

## Phases

1. **Snapshots, pages and previews.** Add `cache.db` with `setups`, `contents`,
   `segments_now`, `pages`, `snapshots` and start/marker `images`. Move
   `.local/states/` into it. Add the GUI's memory LRU with `max_memory`, and
   hydration of a segment or subtree with pause and stop.
2. **Coverage.** Move the coverage tables into `cache.db`, keyed by content, so a
   split or join simply re-records them. Add `first_hits`, `code`, `coverage_sum`
   (clean segments only) and `unreachable.txt`, plus `dj where`. The `parity`
   table comes with comparison cores, once it's decided how parity is checked.
3. **Diffs.** Add `diffs` with `diff_every`, `diff_at_markers` and `exclude`, then
   the presets, a RAM view in the GUI, and the
   watched-address lanes from the Tetris notes.
4. **Agents.** Add `dj sql`, an MCP server with the common queries, and
   `cache pack` / `unpack`.

## Coverage

Coverage uses the terms in [concepts.md](../docs/concepts.md): covered, uncovered, unreachable, parity and no
parity.

**Stored per content key, never per project.** When a segment is played, the
connector reports the ROM bytes it executed during that segment's own frames (not
its parents'). These are saved as covered ranges in `coverage`, with the first frame
of each in `first_hits`. A segment's coverage is a fact about its content, so it
never goes stale. A dirty or edited segment simply has a different content key.

**The project's coverage is worked out from the segments as they are now.** A
block is covered if any segment in `segments_now` that is currently **clean** has it
in its ranges. Dirty segments don't count, because their hashes say they no longer
play as recorded. Unplayed segments don't count either. Status:

- `unreachable` (blue) if the block is in a range from `unreachable.txt`.
- Otherwise, `no parity` (red) if any comparison core has a `differs` row for it from
  a clean segment.
- Otherwise, `parity` (purple) if a comparison core matched every clean segment that
  covers it.
- Otherwise, `covered` (green) if any clean segment covers it.
- Otherwise, `uncovered`.

Parity is kept per comparison core, so the GUI and reports pick which comparison
core to color by.

**Keeping it fast.** Unioning instruction ranges across hundreds of segments on every
edit would be too slow for the GUI. So `coverage_sum` keeps, for each function and
block, how many clean segments cover it. When a segment turns dirty or clean, only
its own blocks are counted down or up. Instruction-level detail is read from
`coverage` on demand, for example when looking at one function. The GUI shows the
per-function and per-block view live, and instruction detail only when asked.

**Unreachable ranges** live in `unreachable.txt`, in version control, as one ROM
range per line with `#` comments. The table is reloaded whenever the file changes.

**Reports** (the gap list, per-function percentages, and an export for
collaborators) are queries over these tables, so the CLI, the GUI and an MCP server
all give the same numbers.

## Decided

- **One baseline core per project, and a version change is a warning.** A
  project is anchored to the core named in `config.toml`. Emulator cores are
  stable, and a new version is very unlikely to change how a game plays. When the
  running core's version differs from the recorded one, the tools warn and carry on.
  The state hashes still catch any real difference. Comparison cores (decomps wrapped
  as libretro cores, or other emulators) never load our snapshots. They play the
  same inputs from power-on and work out for themselves what a snapshot of memory
  means to them.
- **Snapshot layout is checked per core, not designed for up front.** Pages only
  dedup if memory sits at the same offset in every snapshot. Most cores write a
  fixed-size, fixed-layout state, and Mupen64Plus-Next (N64) does. The cores that
  would break it are ones that compress their states or write variable-length chunks.
  NES states are 1:1, so this doesn't matter there. When a large-state core is added,
  the check is simple: the snapshot size stays constant and nearby snapshots share
  most pages. If a core ever fails it, the fallback is content-defined chunking.
- **The N64 core doesn't matter for now.** The tool is developed against the NES.
  The N64 numbers here are for scale only, and the RDRAM size (4 or 8 MB) follows the
  game.

## Open questions

- **Should the core version be part of the cache key?** If it is, a new version starts
  an empty cache, which is safe but means everything is hydrated again. If it isn't,
  snapshots and coverage carry over and the warning is the only signal: a snapshot
  that fails to load falls back to playing, and the state hashes catch a game that
  plays differently. I lean towards leaving it out, but this is undecided.

- **N64 page size.** 64 KB is a guess for the N64 presets. Smaller pages dedup better
  but make longer manifests. Try 16, 64 and 256 KB on a real game once pages exist.
- **How often to write a full manifest** in a chain of diffs. I'd suggest every 8
  diffs.

## What's built

Implemented in `player/src/cache.rs` and `player/src/hydrate.rs`, used by the CLI
and the GUI:

- `cache.db` with `setups`, `contents`, `segment_runs`, `segments_now`, `pages`,
  `snapshots`, `frames`, `images`, `coverage`, `first_hits`, `unreachable`, `runs`
  and `run_results`. `.local/states/` and `coverage.sqlite` are no longer used.
- Content keys, snapshots split into hashed zstd pages (1:1 unless `page_kb` is
  set), snapshots at frame 0, every `diff_every` frames, at markers and at the end
  of segments with children; start and marker previews; the four presets.
- Hydration of everything, a segment or a subtree, with `--clean` and `--force`
  on the CLI and pause, stop and live view in the GUI.
- Coverage keyed by content (clean, hydrated segments only), `first_hits`,
  `unreachable.txt`, and `dj where`, `snapshot`, `sql`, `cache stats` and
  `cache gc`. `--no-db` and `database = false`.
- The GUI's memory limit (`max_memory_mb`) and seeking from stored snapshots.
- The GUI's Coverage tab, built on the same report as `dj report` and on `first_hits`.

Not built yet:

- `exclude`: built 2026-11-15 for storing (`Session::state_to_store`, docs/config.md,
  "Cache exclude"). It takes memory regions (`RAM:0x38F000-0x3FFFFF`), not the
  connector's code regions as in the example above, and each range must also be in
  `[hash] exclude`.
- Pages are stored whole; zstd `--patch-from` against the previous page isn't done.
- `code` and `coverage_sum` (the report recomputes from `coverage` each time, which
  is fast enough at NES sizes), and `parity` (waits on comparison cores).
- The RAM view, watched-address lanes, the MCP server and `cache pack` / `unpack`.
- Snapshot layout per core (no core needs it yet).

