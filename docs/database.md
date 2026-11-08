# The local database

`.local/cache.db` is a SQLite file that holds what playing the project produced. It is a
cache: every row is a fact worked out from (setup, start state, inputs), so the same key
always gives the same answer. It is never committed and never "saved": any full pass
writes it the moment it finishes, whether you watch it in the GUI, clean or let an agent
play, and everything in it can be rebuilt by playing the project. Cleaning fills in what
is missing as it goes ([below](#in-the-gui)). The design and its reasons are in
[CACHE_PLAN.md](../.agents/CACHE_PLAN.md).

The database is optional. Without it (`--no-db`, or `database = false` in
`.local/local.toml`), the tools play from power-on, or from the nearest snapshot in
memory. It only makes things faster, except for coverage reports, which read it.

## What it holds

Rows are keyed by each segment's **content key**: a hash of its setup, start state and
lines (for a root, also its `sram` and `snapshot` blob ids). An edit gives a segment a new key, so it invalidates only that segment's rows.

- **Snapshots** of the core state at frame 0, every `diff_every` frames, at markers, and at the end of segments with children. Each is split into pages that are stored once by hash and compressed with zstd.
- **Previews** (PNG) at the start of each segment and at markers.
- **Coverage** flags for each segment, and the frame at which each ROM byte first ran in it, which `dj xref` reads ([coverage.md](coverage.md)).

How many snapshots and previews it keeps is set by the `[cache]` table and its presets,
in `config.toml` or per machine in `.local/local.toml` ([config.md](config.md#cache-optional)).

## From the command line

```sh
dj clean <dir> [SEGMENT] [--accept] [--all]                 # clean, filling in what the database is missing
dj hydrate <dir> [SEGMENT [--children]] [--force]            # fill it in without recording hashes
dj cache stats <dir>                                         # size, and how well pages share
dj cache gc <dir>                                            # drop what no segment uses
dj sql <dir> "SELECT ..."                                    # query it directly
```

- `clean` plays the segments that aren't clean or whose content isn't stored yet, and writes the hashes they reach ([playback.md](playback.md#checking-hashes)). Running it again is nearly instant. `--all` plays every segment, stored or not.
- `hydrate` fills the database the same way but never writes a file, for scripts that only want the database. `SEGMENT --children` limits it to a subtree; `--force` plays even what is stored.
- `cache stats` warns when the file is larger than `budget_gb`.
- `cache gc` deletes contents that no current segment uses (and snapshots of passes that never finished, for content no segment has now), then unused snapshots and pages, and compacts the file. It refuses while a dj-gui has the project open, since an unsaved edit's snapshots belong to content no saved segment has yet (the GUI keeps a `gui-<pid>.open` file in `.local/` while open); `--force` runs it anyway.
- Every command that plays takes `--no-db` to run without the database.

### Views

The tables can change between versions. Queries from scripts and agents should use the
views, whose columns stay:

| View | One row per | Columns |
| --- | --- | --- |
| `v_first_hits` | ROM offset first run in a current segment | `segment_id`, `status` (`clean` or `dirty`), `region`, `rom_offset`, `frame` (the first frame of that segment where it ran), `content` (the content key) |

```sh
dj sql <dir> "SELECT segment_id, status, frame FROM v_first_hits WHERE region = 'PRG' AND rom_offset = 0x1C3A ORDER BY status, frame"
```

"Current" means the saved segment files. `dj sql` brings the list up to date before each
query; `sqlite3` sees it as of the last time `dj` or the GUI opened the project. Edits not
saved yet aren't in it.

## In the GUI

The end of the Segment menu has **Clean all segments** and **Replay everything**. Cleaning
plays, top down, the segments that aren't clean or whose content isn't stored yet, and
[records their hashes](playback.md#recording-hashes) as unsaved changes (so it works with
edits open, and Save keeps the result). A clean segment whose content is stored is passed
over without playing. It carries on down the tree through changed endings, marking each
with **!**; segments below one that couldn't be cleaned are skipped. Drafts are cleaned
too, except those still open in an agent session. **Replay everything** plays every
segment again, stored or not: a check after a core update, say.
Right-click in the tree for **Clean this and below**. A clean shows every 4th frame in the
picture, with a progress bar and **Pause** and **Stop** in the status bar. Selecting,
seeking and playing wait until it ends. Segments finished before a stop stay stored.

The database also answers for hashes: whenever the GUI loads or reloads segments, a
segment that isn't clean, whose start is trusted, and whose current content has a stored
pass gets that pass's hashes [recorded](playback.md#recording-hashes) without playing
(as an unsaved change). A stored pass that ends differently from the file is left for a
real play, since a changed ending needs checkpoint hashes the database doesn't keep.

Playing fills the database too: a pass that runs a segment from frame 0 to its end without
a jump or an edit records its snapshots, previews, coverage and first hits when it
finishes, as hydrating would. A pass broken by a jump back or forward, or an edit, keeps the
snapshots it took, since each is a valid state of that content. So cleaning is only a
convenience, for what you haven't played.

A segment cleaned by a broken pass (one you jumped around in, then let reach its end) is
clean, since its hashes are recorded, but it isn't stored: coverage doesn't count it until
a full pass or a clean plays it from frame 0 to its end.

Segments whose current content is stored show a blue **db** after their length in the
tree. **Use the local database** turns it off for this machine.

### Seeking and memory

- While a segment plays, the GUI keeps a snapshot in memory at the start of every 10th frame (sharing the capture with the database's where they fall on the same frame).
- A seek starts from the nearest snapshot at or before the target, from memory or from the database, whichever is nearer, then runs the core forward and draws only the last frame. With `diff_every = 60`, a seek anywhere is near instant.
- Selecting a segment starts from its parent's end snapshot in the database when the parent is clean, so deep segments open without playing the chain. The GUI also stores each segment's start snapshot, and the end snapshot of segments with children.
- Long seeks and loads run in slices, so the window stays usable, with a progress bar in the status bar. Selecting another segment cancels one.
- An edit drops the snapshots after the edited frame and redraws the current frame.
- Emulation > **Snapshot memory** (`max_memory_mb`, 512 MB by default) caps in-memory snapshots. Past it, the snapshots farthest from the playhead go first; frame 0 always stays, and anchors every 300 frames stay while they fit. An NES state is a few tens of KB, so NES never reaches it; it matters for cores with large states.
