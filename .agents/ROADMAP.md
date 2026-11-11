# Roadmap

Ideas and open questions that are not part of format version 1, and the current limits.
What is prioritised for the first release is in [Before the first release](#before-the-first-release).
When one ships, its user-facing description moves into `docs/` and the entry here goes.

## Before the first release

Priorities: **high** blocks the release, **medium** should be in it, **low** can follow it.

| Priority | Item | Plan | Status |
| --- | --- | --- | --- |
| high | Land the N64 connector and MIPS decoder; docs agree on what's supported | [CONNECTOR_PLAN.md](CONNECTOR_PLAN.md) | done 2026-11-09 |
| high | CI on Linux and macOS (Windows as a template), Thwaite hashes on every change | [RELEASE_PLAN.md](RELEASE_PLAN.md) | written 2026-11-10, not run yet (push to run) |
| high | Projects record the format and tool version; older tools refuse newer projects; a core rebuild re-pins | [below](#project-and-cli), "Versioning projects" | done 2026-11-10 |
| high | Clean-up and licensing: scratch files, terminology in the CLI help, the Tetris data tables, connector licences | [RELEASE_PLAN.md](RELEASE_PLAN.md#licensing-of-the-connectors-to-settle) | not started |
| high | Release workflow and install docs (`v0.1.0`) | [RELEASE_PLAN.md](RELEASE_PLAN.md) | portable ZIPs and draft release written 2026-11-10, not run yet |
| medium | Decoder interface (step 1: one trait and one sweep; step 2: loadable decoders) | [DECODER_PLAN.md](DECODER_PLAN.md) | done (plugins 2026-11-10) |
| high | Cores and plugins can't crash or hang dj: workers for decoder plugins, comparison cores, batch commands, then the GUI's core | [ISOLATION_PLAN.md](ISOLATION_PLAN.md) | done for plugins, comparison cores and batch commands; the GUI's core protected in process (edits kept, crash note); moving it to a worker designed, not built (decided 2026-11-10: not now, maybe before the release) |
| medium | Root segments that start from a snapshot, finished ([below](#root-segments-from-a-snapshot)) | | partly built |

### Root segments from a snapshot

A root segment can start from a save state blob instead of power-on, for games whose state
is fine to share (open source, homebrew, abandonware with permission). A project with a
commercial game should not ship one: a state holds the game's RAM, and for some systems
(N64 RDRAM) the game's own code.

**Already built** (format version 1): the `snapshot` header field names a blob in `blobs/`
(`<hash>.state`, `retro_serialize` output of the pinned core), root only
([segments.md](../docs/segments.md#header-fields)); the player loads it before frame 0
(`player.rs`) and takes `start-frame` from the segment; the GUI sets it in Properties
("Snapshot", copied into `blobs/`); and a snapshot root acts like any stored state for a
comparison core: `dj_sync` gets no `DJ_SYNC_BOOT` (`gui/src/main.rs`, `boot` is only set
for a root without a snapshot), and `sync = "state"` cores load the baseline's state.

**Left to do:**

- **Making one from the CLI and MCP.** `dj new` has no `--snapshot FILE` (or `--sram`);
  `dj snapshot` writes a state to a file but can't turn it into a root. Add
  `dj new <dir> --snapshot FILE [--start-frame N]` and the same for the MCP tools.
- **Making one from where you are, in the GUI.** "New root segment from here": take the
  state at the playhead, store it as a blob, and create a root with that `snapshot` and
  `start-frame`. Today it takes `dj snapshot` and Properties.
- **A clear warning before sharing.** When a snapshot blob is added, say that it holds the
  game's state and may not be distributable; `dj init` could also ask in config whether the
  project may carry snapshots (`snapshots = "allowed"`), so tools warn when a blob is added
  to a project that says no, and a project template for commercial games adds
  `blobs/*.state` to `.gitignore`. Extend the note in
  [segments.md](../docs/segments.md) with when sharing is fine.
- **Checks.** A snapshot must load in the pinned core: verify it on add (load it and read
  back the state hash, stored as `start-hash`), and say clearly which blob failed when a
  rebuilt core can't load it (ties in with "Versioning projects"). Test on more than one
  OS for Mesen.
- **Tests.** A root from a snapshot plays and reproduces its hashes from an empty database;
  a comparison core is synced at frame 0 without `DJ_SYNC_BOOT`; deleting or re-parenting
  keeps the blob only on a root (the GUI drops it already, with a message).
- **Docs.** gui.md and editing.md on making a snapshot root; connectors.md already says
  `DJ_SYNC_BOOT` is only for a power-on; cli.md for the new flags.

## Known limits

- Three cores have connectors: Mesen (NES), mGBA (GBA; [connectors/mgba/](../connectors/mgba/README.md)) and
  Mupen64Plus-Next (N64, first version; [connectors/mupen64plus/](../connectors/mupen64plus/README.md)).
- Block and function rollup decodes the 6502, MIPS (`mips3`) and ARMv4T (`armv4t`). Other CPUs get raw opcode-byte counts until their decoder is added.
- GBA: only the `ROM` region rolls up, so functions a game runs from IWRAM or EWRAM show as never entered; `dj init` picks `RAM` to hash, which on mGBA is only 32 KB of EWRAM (use the `BUS:` ranges in [connectors/mgba](../connectors/mgba/README.md#for-a-project)); the mGBA connector costs about 37% speed (a call per instruction).
- N64: code in an overlay is credited to whatever the symbol map names at its addresses, whichever overlay was loaded (see "N64 follow-ups").
- Determinism has been checked on Linux x86_64 and macOS arm64; Windows still needs a run.
- The GUI is a first prototype: button inputs only (no analog, lightgun or pointer rows yet), keyboard-only recording for one controller at a time, one core at a time. Tested on Linux and macOS; Windows not tried yet.
- `exclude` in `[cache]` is recorded but not applied to snapshots yet.

## Known bugs

- **The Functions tab overlaps the coverage map when the map is made taller.** Dragging the
  map's panel (the resizable `cov_map` bottom panel in the sidebar, `gui/src/main.rs`) up
  while the sidebar shows the Functions tab makes the sort and filter row, and whatever is
  below it, draw on top of the map. Likely cause: `coverage_tab` (`gui/src/cov.rs`) lays out
  the totals, notes, the selected function's details and the parity section at their full
  height with no scroll area, and only the function list (`ScrollArea` `cov_list`) shrinks,
  so with less room left the rows above it run past the sidebar's remaining space into the
  map panel instead of being clipped or scrolled. Not confirmed.

## GUI
- **Shipped 2026-11-10: a debug console in the app** ([gui.md](../docs/gui.md#debug-console)): View >
  Debug console, a log below the coverage map of library warnings and notes, the cores' log
  (`GET_LOG_INTERFACE`), workers' stderr and the status bar's messages, with a filter, level
  toggles, Copy and Clear (`decomp_dj::console`). Still to do: jobs (cleans, hydrates, seeks)
  and comparison syncs as their own lines; writing it to `.local/gui.log` for bug reports;
  the MCP console (under "Project and CLI") as one tab of it; the cores' log with its format
  strings filled in (needs a C shim, since Rust can't take C variadics).
- **A-B loop.** Loop a chosen stretch of frames, like VLC's A-B repeat: set A and B at the
  playhead (buttons in the transport and keys, or drag a range on the frame ruler), and
  playback jumps back to A on reaching B until the loop is cleared. Shown as a band on the
  ruler with handles to adjust it. Open: whether a loop may cross segment boundaries (from a
  parent into a child, say) or stays in one segment; how it relates to the Loop end mode
  (which loops the whole segment); whether it's per segment and remembered (in the local
  database or `.local/`) or just for the session; and how it works while recording (likely
  re-recording over the loop each pass, with overwrite on). Jumping back to A uses the
  nearest snapshot, so the comparison core re-syncs at A each pass.
- **Rewinding up the chain.** Stepping or seeking back past frame 0 goes into the
  parent's last frames (loading a snapshot of the parent, from memory or the local
  database), and playing forward from there follows the sticky segments back down. A
  possible next step is one timeline across the whole chain, with the segment
  boundaries drawn on the ruler.
- **A file watcher instead of polling.** The GUI learns about drafts, sessions and the
  shared view by fingerprinting `.local/` about once a second, so following an agent shows
  one step in several (a 60-frame `step` takes about 100 ms at 500 to 700 fps) and new
  drafts arrive late. Watch `segments/`, `blobs/`, `.local/drafts/` and `.local/mcp/` with the
  `notify` crate (inotify, FSEvents, ReadDirectoryChangesW) and react within milliseconds,
  keeping the fingerprint poll as a slow fallback for file systems that don't notify
  (network mounts). Since every writer writes to a temporary file and renames, a change event always finds a whole file.
  This keeps the no-daemon design and costs about a day; it fixes the lag and the
  half-written reads, which are the file-system part of the draft problems. Ownership
  (agent-owned segments) isn't, and stays as it is. Sockets
  ([below](#project-and-cli)) are then only needed for attach.
- **Pick a start blob from the project when making a root segment.** New root segment (and
  the properties dialog's Snapshot and Save file rows) should offer the blobs already in
  `blobs/` as a dropdown filtered by typing, besides choosing a file from disk: `.srm` files
  for the save file, `.state` for the snapshot. Each entry shows its name (see "Named blobs"
  [below](#project-and-cli); until then the short hash), size, and which segments use it.
  Today the GUI never lists `blobs/`: it only reads a blob a segment names, and nothing is
  kept in memory. So keep a list of the blobs (name, kind, size, hash, the segments using
  it), built when the project opens without reading every file (the hash is the name today;
  with named blobs it would come from the index), and kept current by the file watcher above
  (`blobs/` added to what it watches) or the fingerprint poll until then, so a file copied in
  by hand or by an agent shows up without a reload.
- **Recording follow-ups** (recording itself is in docs/gui.md#recording).
  - **Gamepads.** Device discovery, hot-plugging, analog dead zones and per-OS differences
    make them a larger job than the keyboard.
  - **Per-console default bindings.** The first version has one default layout (WASD for the
    d-pad, J = B, K = A, U = Select, I = Start) for one controller. Each console (NES, SNES,
    N64, ...) should get its own default that covers its buttons, picked from the core or
    the project's system.
  - **Two players in real time.** Bindings for a second controller at once.
- **Two lines for the playback bar.** The bar under the picture has grown crowded: play,
  step, the marker buttons, the ⏭ continue split button, the end mode, Record and speed.
  Split it in two: the transport (play, step, markers, continue, Record, speed) on one line,
  and the modes and options (end mode, and the comparison controls below) on the other,
  with the frame and status line below as now. Open: how it wraps at narrow widths (fold
  to one line when there's room, or wrap).
- **Comparison controls on the playback bar, redesigned.** The Compare dropdown, Run and
  Check parity were taken out of the bar; for now they're only in the Compare menu. Once
  the two-line layout exists, bring back something compact: an indicator or toggle for the
  comparison core (which one, in step or not) and for parity checking.
- **A region dropdown for the coverage map.** Once connectors report several code regions
  ([CONNECTOR_PLAN.md](CONNECTOR_PLAN.md)), the map and totals show them appended as one
  block. A dropdown over the map would pick one region (or all), for systems with several
  CPUs or RAM-resident code (SNES audio RAM, Mega Drive Z80 RAM, N64 RDRAM and RSP).
- **Audio playback (GUI).** Play the core's audio while a segment plays, with a volume
  slider and a mute button in the playback toolbar. When more than one core is running
  (side-by-side comparison), an A|B toggle picks which core you hear.
- **More ways to join.** Merge into children (copy the parent's frames onto the front of every child, then remove
  the parent: no hash changes, but the frames are duplicated). (Join a chain is done:
  Edit > Join chain….)
- **Instead of deleting,** merge a segment into its children (Delete, keep children, which
  moves them up to the grandparent, is done).
- **Focus on tagged segments.** The tree filters by tag (`#tag`, the **#** menu) and can show
  what's below the matches. Still open: a Focus-like view of every segment with a tag at
  once (Focus here takes one segment), clearing the tag filter as it focuses.
- **Menu follow-ups.** The menus are regrouped (File, Edit, View, Project, Emulation, Compare, MCP).
  Placeholders still to fill: File > Options… (the GUI preferences in `.local/local.toml`,
  with Keyboard mapping maybe folded in) and Project > Project settings… (the
  project-related settings of `config.toml` and `local.toml`). Later, View toggles to hide or
  show the major parts of the window.
- **Back and forward, like a browser.** A history of where you've been, as segment and
  frame: selecting another segment (in the tree, the Functions tab's "First runs in", a
  marker, a search), or a seek far enough to count as a jump, pushes the place you left.
  Back returns to that segment *and* frame (500 frames into the first segment brings you
  back to frame 500, not frame 0); Forward redoes it. Buttons in the transport (or toolbar),
  the mouse's back and forward buttons, and keys: Backspace for back and Shift+Backspace
  for forward. Backspace already means two other things: delete the selected presses in
  the piano roll, and rewind while recording. Keep those where they apply (presses
  selected, or record mode on), or move all deleting to the Delete key alone so Backspace
  is only back. Mac laptops have no separate Delete key (their "delete" is Backspace; forward
  delete is Fn+delete), so the second choice makes deleting awkward there. Open: which seeks count as a jump (clicking the ruler, Home and End, a
  first-hit link) and which don't (stepping, playback); whether history survives an edit
  that removes a segment (skip entries that no longer exist); and whether it's kept per
  session only.
- **Markers tab:** list a marker's "first frame where a watched value differs" once that
  exists. (Following Focus here is done.)
- **Cancelling a long load** without selecting another segment, and MAX speed off the UI
  thread.

## Coverage
- **Shipped: annotations.toml.** `[[unreachable]]`, `[[data]]` and `[[function]]` entries
  with a `why`, replacing `unreachable.txt`. Data is skipped by the sweep and left out of
  totals in both modes; code that runs inside an annotated range warns; `dj annotations`
  lists and checks the entries. Tetris carries its unreachable two-player code and the
  seven `switch_s_plus_2a` jump tables.
- **Editing annotations in the GUI (later phase).** Right-click a map square or a range
  in the Functions tab to mark it unreachable or data, or to name or resize a function, and
  write the entry (with a `why` prompt) to annotations.toml.
- **Symbol sources to accept.** Users bring their own symbols, so more formats are the way
  to cover more projects: importing Mesen `.cdl` files (its code/data logger marks data
  positively; or a connector flag for ROM bytes read as data, 0x04), and a headless Ghidra
  export (CSV), for larger systems.

- **A coverage report that can be committed.** `dj report --write` (or a step of `dj run`)
  writes a `report/` folder in the project that goes into version control, so a
  collaborator can see what is covered, and the overall %, from a fresh clone without
  cleaning anything, and review coverage changes in a diff. The folder splits the headline
  from the detail:
  - `report/README.md` (or `summary.md`): the headline numbers (functions, blocks and
    instructions covered, overall %, how many segments counted, dirty or not played), what
    changed since the last report, and the largest gaps. Short enough to read on GitHub.
  - `report/functions.csv`: one row per function with its covered and total blocks and
    instructions.
  - `report/blocks/` (or one `blocks.txt`): the verbose part, which blocks are covered in
    each function, for anyone digging into one area.
  - `report/meta.toml`: what the report was built from: the content keys of the segments it
    counts, the core build and the symbol maps. Tools warn when the report is out of date (a
    segment's content key or a symbol map changed since), and the GUI could show it before
    the local database is filled.

  Open: one line per block keeps diffs readable but grows with the ROM; ranges of covered
  offsets per function would be smaller for N64-sized games. Parity columns join once
  comparison cores exist.
- **Coverage in the GUI:** the Functions tab ([coverage.md](../docs/coverage.md#in-the-gui)) shows the report, a ROM map
  and live new coverage while playing. It rebuilds the whole report after every edit,
  which takes milliseconds at NES sizes. For large games, keep coverage per segment and
  per function or larger block, sum it in a cache (`coverage_sum`, [CACHE_PLAN.md](CACHE_PLAN.md)), and
  update only the sums for segments whose clean or dirty status changed. Parity colors
  (purple and red) join the map once comparison cores exist.
- **Turning coverage off.** A setting (`coverage = false`) for projects that only set up
  segments for later analysis or compare cores by eye. Off: no `dj_clear` or flag reads, no
  coverage or first hits stored, the Functions tab and map hidden in the GUI, and the
  coverage CLI and MCP tools saying it's off. Plain libretro cores with no connector already
  work as the baseline (`dj init` and `dj run` only print a note); this makes that a choice.
  Segments don't hold coverage, so turning it on later and cleaning fills it in. Open:
  per project (config.toml) or per machine (local.toml), and a GUI toggle.
- **Tracking coverage lost.** When an edit, a delete or a re-recorded segment means code
  that used to be covered no longer is, nothing says so: the totals just drop. Keep the
  previous coverage per segment (or per report), and show what a change cost: in the GUI
  after a save or Verify ("−14 instructions in `doStateNewLevel`"), and in `dj report`
  against the last committed report. Pairs with the committed report above, whose diff
  would show it in review.
- **"First runs in" may need its own view.** The function details now show the best first
  hit and open the rest in a filterable list ([coverage.md](../docs/coverage.md#in-the-gui)).
  If that proves too small for code that runs in every segment (the NMI handler, the main
  loop), move it into a separate view: a searchable table of segment, frame and depth,
  grouped by root segment. Also open: "earliest" compares frames inside each segment, not
  frames since power-on (a frame plus the lengths of the segments above it), which would
  be the fairer order across branches; `xref` and the MCP tools share the sort.
- **Symbol maps during active decompilation.** Symbol maps in the formats a decomp already
  keeps are supported ([coverage.md](../docs/coverage.md#symbol-maps)), so the decomp stays the source of truth. Still to do:
  let `.local/local.toml` point a `[[symbols]]` entry at a file in the decomp's own
  checkout, since that path differs per machine; reload when the file changes, so a
  rename shows on the coverage map and list at once; and maybe an opt-in "Name this
  function" action in the Functions tab that appends a line to `symbol_addrs.txt` (or
  the `.mlb`). Rolling up regions other than `PRG`, and a MIPS sweep for N64, come
  with the N64 connector.

## Project and CLI

- **Snapshot and memory profiles for common consoles.** `[cache]` has presets for NES and
  N64 only ([config.md](../docs/config.md#cache-optional)). Add tuned presets (page size, snapshot
  interval, previews, and `exclude` ranges once they're applied) for the other common
  consoles: SNES, Game Boy / Color, Game Boy Advance, Genesis / Mega Drive, Master System,
  PC Engine, PlayStation, Nintendo DS. Measure each core's save-state size and how much of
  it changes per frame to pick the values, and choose the preset from the core or the
  project's system by default.
- **Connectors for the best core of the 10 most popular consoles.** Only Mesen (NES) has a
  connector today ([connectors.md](../docs/connectors.md)). For each of the ten most
  popular consoles, pick the best libretro core the way Mesen was picked for the NES (the
  best balance of accuracy and speed, with deterministic save states and a debugger to
  hook execution into), then write its connector. First step: the list of consoles and the
  chosen core for each, with the reasons.
  Planning notes for a second core (bsnes-mercury) and where decoding should live:
  [CONNECTOR_PLAN.md](CONNECTOR_PLAN.md).
- **Named blobs.** Look into letting a blob have a name people choose (`sram = "world-3-save"`,
  `blobs/world-3-save.srm`) instead of only its content hash. Today the hash is enforced, not a
  convention: `Collection::read_blob` refuses a blob whose content doesn't hash to its name,
  `write_blob` names it by hash, and segments, the database key and [segments.md](../docs/segments.md#blobs)
  assume a hash. What a name would need: something else to catch a changed or swapped file
  (a `blob-hash` header field beside the name, or a `blobs/index.toml` of name to hash), so
  replays still fail loudly; the cache key using the content hash, not the name; rules for
  names (no path separators, no clash with the hash form); a rename that updates every
  segment naming the blob; and the GUI's properties dialog asking for a name when a file is
  picked. Many segments sharing one blob (a blank save) should keep working.
- **Shipped 2026-11-10: versioning projects** ([config.md](../docs/config.md#top-level),
  [segments.md](../docs/segments.md#older-projects)). `format` gates (a newer one is refused,
  naming what to do), `dj` records the newest dj that wrote the project (an older dj warns, as
  for settings it doesn't know), segment files start with `dj-segment 1` (`dj-replay 1` still
  read), `dj upgrade` rewrites them all at once, connectors report a build id
  (`dj_build_id`: upstream commit + patch hash, the same on every platform) that dj checks
  against `[core] build`, and `dj repin` re-verifies the tree after a core change and records
  the new build. Hashes never depended on the core binary, only the local database did. Still
  open: per-platform `sha256` pins for `[[compare]]` (see "Portable `[[compare]]` entries").
- **Backups without version control.** `dj backup <dir>` (and File > Back up… in
  the GUI) writes the project's shared files to a ZIP: `config.toml`, `segments/`,
  `blobs/`, `symbols/` and `annotations.toml`, never `.local/` (the database can be
  rebuilt by cleaning) and never the ROM. `dj restore <zip> <dir>` puts them back,
  replacing `segments/` whole so deleted segments come back and new ones go. Useful
  before a big agent-driven edit or an experiment, for authors who don't use git.
  Possible extras: an automatic backup before risky commands (a bulk clean, a large
  agent edit), named backups under `.local/backups/` with a limit on how many are
  kept, and a `diff` between a backup and the current files listed by segment title.
- **Project generator (CLI).** A command that sets up a new project in one go:
  find or ask for the ROM, pick a matching core, write `config.toml`, `.gitignore`,
  a README and an empty root segment, and optionally a symbol map. `dj init` covers the
  bare minimum today; this could be an interactive mode of it or its own command.
- **Package managers.** First releases are a folder per platform on GitHub Releases (the
  tools, the connectors and the example, no cores; [RELEASE_PLAN.md](RELEASE_PLAN.md)). Later: a Homebrew formula or cask (a tap first,
  then homebrew-core once it's stable), and Scoop or winget on Windows, and maybe an AUR
  package, a Flatpak or `cargo install` for the tools alone. Open: whether patched cores
  (GPL) are ever packaged, on their own, and where an installed copy looks
  for its cores (next to the executable, a share directory, a user directory).
- **Shorter segment ids.** Replace UUIDs with something short enough to quote in docs
  and commit messages, for example 8 characters of base32 (40 bits). Tools must check
  the new id against every id in the project when generating one, and try again on
  a collision. A tool that finds two files with the same id refuses to load the
  project. Existing UUIDs stay valid, and the CLI already accepts unique prefixes.
- **Pinned memory (cheats) in segments (maybe).** A command that holds RAM at a value for
  the rest of the segment, like a Game Genie or GameShark code: for example
  `0 [PIN] RAM 0x0451 63` keeps a player's health full, and `[UNPIN] RAM 0x0451` (or the
  end of the segment) stops it. The player writes pinned bytes before every frame, or
  before every logic tick when the project defines `[tick]`, so the game never sees
  another value for long. Pins don't carry into child segments; a child that wants one
  adds its own `[PIN]` at frame 0, so a segment's file still says everything that affects
  it.
  - **Why:** fuzzing and long runs that would otherwise end early. Two CPU players fighting
    with full health forever, or a run that can't die, can keep reaching new code while the
    fuzzer explores.
  - **Costs:** a segment with a pin is poked (its coverage is counted apart from play
    alone, as for `[WRITE]`). Adding a command to the format needs care: players must
    reject commands they don't know, so older tools would refuse these segments.
  - **Open:** whether ROM patches (true Game Genie codes, which change what the CPU reads
    from ROM rather than RAM) are worth supporting too; they need the core's help. And
    whether a fuzzing job could apply pins on its own without writing them into a segment.
- **Macros and Lua scripts** for active RNG manipulation and pokes, instead of long
  runs of `[WRITE]` lines. They would be stored locally, and segments would refer to
  them by name, as `[RNG]` ([reserved commands](../docs/segments.md#reserved-commands)) already reserves.
- **MCP follow-ups.** `dj mcp` is built ([mcp.md](../docs/mcp.md)). Left to do:
  - **Sockets alongside the files.** The GUI follows sessions and shares its view through
    files in `.local/mcp/`, which needs no connection handling while agent servers come and
    go, but can't share a core (the lag is fixed by the file watcher in the
    [GUI section](#gui)). Keep the files as discovery and fallback, and add a local socket: `dj mcp` listens and writes its address and a token
    into the `sessions-<pid>.json` it already publishes. The GUI connects for live frames and
    inputs, falling back to the files when it can't. Then **attach**: the GUI drives the
    agent's own core instead of opening a copy at the same point. Use loopback TCP with the
    token (or a named pipe) on Windows, or TCP everywhere so every platform shares one code
    path; Rust's standard Unix sockets don't exist on Windows. Build live follow first,
    then attach.
  - A Windows job object, so workers end if `dj mcp` is killed.
  - Trace hooks for connectors other than Mesen, and disassembly for CPUs other than the 6502.
  - Parity per function and per segment, once comparison cores exist (`dj parity <segment> --against <core>`, and an MCP tool).
  - **An MCP console in the GUI.** A panel (MCP > Show console, or clicking the MCP state in
    the status bar) with a running log: errors and warnings, servers starting and stopping,
    sessions opened and closed, drafts written, saved or discarded, jobs starting, finding
    new code and finishing, tool calls that failed, and when an agent read the shared view.
    `dj mcp` would append events to a log in `.local/mcp/` (or send them over the socket,
    once there is one) and the GUI would tail it, with filters by level and session and a
    way to copy lines into a bug report.
  - More stable SQL views: `v_function_coverage` (covered and total instructions and blocks per function). Unlike `v_first_hits` ([database.md](../docs/database.md#views)) it needs the symbol maps, which aren't in the database, so the tools would have to write the functions into a table first.

## Comparison and parity

- **Shipped: comparison cores, phase 1.** `[[compare]]` entries, the GUI's Compare
  dropdown and Run toggle (now the Compare menu), `dj_sync` (with `DJ_SYNC_BOOT`) or save-state sync after
  jumps, `dj_write` for `[WRITE]` lines, and the pictures side by side
  ([gui.md](../docs/gui.md#comparison-cores), [connectors.md](../docs/connectors.md#comparison-cores)).
  GUI only; the comparison core's coverage hooks are ignored.
- **Shipped: parity (phase 2)**, in the GUI ([gui.md](../docs/gui.md#parity),
  [connectors.md](../docs/connectors.md#parity)): `dj_check_parity` with the RAM fallback,
  `dj_run`, Check parity, stopping on any failure with the faded picture and its message,
  the build hash at load, each checked frame's own coverage, failures and passes in the
  local database per core, the colours on the map and the function list, Re-run, Clear and
  Re-run old failures. Where it differs from the design below: passes are one set of PRG
  flags per core and build (not per frame or keyed by segment content), failures name the
  segment by id, a Re-run that passes clears an old failure without asking, failed syncs
  aren't kept, and parity is counted per instruction (not per block). Still to do: counts
  per block ("passed on X frames") and the CLI and MCP (`dj parity`).
  - **A frame is the test.** After each frame both cores ran from a synced state, the
    comparison core's connector gives a verdict through an optional
    `int dj_check_parity(baseline, frame, char *reason, size_t)`: MATCH, MISMATCH or SKIP,
    with a reason. A core without it falls back to a RAM compare (below). Frames are tested
    one at a time, from wherever playback is: every way of reaching a frame (from frame 1,
    from frame 2, ...) can't be tried, so a frame checked once counts, whatever pass it was
    in. A **Check parity** checkbox, off at launch and kept for the app session (not saved).
  - **A failure stops everything.** A MISMATCH (or ERROR) stops playback of both cores at
    that frame. The comparison core's picture is shown faded, as while seeking, with the
    failure message over it. No resync and no carrying on: the next frame isn't tested
    until playback is started again, which syncs the comparison core first, as after a
    jump. The same goes for a failed `dj_sync` (its `reason` is the message) and for a
    failed tick (below). Later: a setting to keep playing after a failure (both cores run
    on, unsynced, with later frames marked as following a failure, not tested), since how
    far and how fast they drift apart can say a lot.
  - **Which blocks a frame ran.** Coverage flags are cleared only when a segment starts, so
    they can't say what ran in one frame. The player clears them every frame and keeps the
    segment's running union itself (cheap for the NES; for large ROMs, a connector call
    listing the pages touched since the last clear). Only the baseline core's coverage is
    used.
  - **Which build.** The comparison core's library is hashed (sha256) when it's loaded, as
    `[[compare]] sha256` pinning already does. That hash is the build: the same file is the
    same build, a rebuilt one has a new hash. No version query is needed from the core.
  - **Failures record how to reproduce them:** the build, the segment, the frame playback
    was synced at (where this run started) and the failing frame, with the reason. Starting
    from another frame can give another result, so the record keeps the exact run.
  - **Block verdicts from frame verdicts.** A block (or byte) is **parity** (purple) when it
    ran on at least one checked frame and on no failing one; **untested** when it ran only
    on unchecked frames; not run; **failed** (red) when a frame that ran it failed with the
    current build: a pass of the same frame later (from another start, say) doesn't clear
    it, since nothing changed; and **failed before** (orange) when it failed with an older
    build. Orange doesn't turn purple by itself: a later pass from another start doesn't
    show the old run is fixed. Each old failure offers **Re-run** (sync at the recorded
    frame of the recorded segment and play to the failing frame with the current build)
    and **Clear**; a Re-run that passes offers to clear it, and a project-wide "Re-run old
    failures" does all of them. A function has parity when every block of it that ran
    has parity.
  - **Stored per comparison core.** In the local database, per `[[compare]]` entry, keyed
    by segment content and frame: the frames checked, the failing frames with their reason
    and the build (`sha256`) they failed with, and per frame the blocks run (or per segment,
    "ran on a passing frame" and "ran on a failing frame" sets, if per-frame sets are too
    large). A new build keeps the old failures, which turn orange once the frame passes.
    Counts per block ("passed on X frames, failed on Y") and a link to the first failing
    frame, like the Functions tab's "Runs in".
  - **Shown** on the coverage map and the function list (purple, orange, red), and later
    from `dj parity` and an MCP tool.
- **Revisit: frames that don't apply.** Many NES games aren't fully set up for the first
  frames after power-on, and a decomp may not want parity there. For now those frames are
  left to `dj_check_parity`'s SKIP, and the code only they run shows as "not checked". To
  decide later: a "not applicable" colour of its own, kept out of the not-checked count, and
  showing skip counts so skips can't hide gaps.
- **More in the Compare menu.** Pause on first failure, the later keep-playing-after-a-failure
  setting, and a Clear all failures action (Clear is per failure, in the Functions tab).
- **Comparison core error messages.** Done: `dj_run` ([connectors.md](../docs/connectors.md#parity)), and the same faded message for failed syncs, ticks and parity checks.
- **Shipped: OpenGL cores (HW_RENDER).** A private offscreen context per core slot (CGL on
  macOS, EGL elsewhere), read back on captured frames ([install.md](../docs/install.md#cores-that-draw-with-opengl)).
  Next: Vulkan (the negotiation interface, `ash`, MoltenVK on macOS), then maybe OpenGL ES
  through EGL, Windows (WGL), and asynchronous readback (PBOs) or showing the texture
  directly if readback costs too much at high internal resolutions.
- **Portable `[[compare]]` entries.** A core `path` may now leave out the suffix
  ([config.md](../docs/config.md#core-paths)). Still open: keeping machine-specific parts
  out of the shared entry (config.toml carries `name`, `sync`, `options`, `build` and
  `sha256`, and each machine's `.local/local.toml` adds `path`; entries already merge by
  `name`); per-platform paths (`path.macos`, `path.linux`); `sha256` per platform, since
  builds differ; and an entry with no `path` here showing in the Compare menu as "not set
  up on this machine" instead of failing on pick. The same applies to `[core]`.
- **N64 follow-ups** (the connector and the MIPS decoder are in; details and results in
  [CONNECTOR_PLAN.md](CONNECTOR_PLAN.md), steps 3 and 4):
  - **Playback speed.** dj plays the N64 core at about 52 fps against 103 fps for the bare
    core. Likely cause: reading the connector's flags every frame for first hits, about
    40 MB, 32 MB of it the `ROM` region, which has no flags yet. Hand out no `ROM` flags
    until provenance sets them, and scan only what changed (the connector could keep a
    per-page dirty bit, or the player could compare only pages a frame touched).
    Measured 2026-11-09: the first-hit scan in `player.rs` (`Recorder`, 64-byte chunks,
    byte by byte) takes about 28 ms for 40 MB on Apple silicon, more than a frame's budget
    by itself. Cheap fixes, any CPU: skip regions that have set no flags; compare 8 bytes
    (or SIMD) at a time; with a fixed-width decoder, look only at aligned positions
    ([DECODER_PLAN.md](DECODER_PLAN.md#performance)). Parity's per-frame flags gain the same.
  - **Provenance and overlays.** Overlays share addresses, so code that ran there is
    credited to whatever the map names. `dj_origin` (which ROM offset a RAM word was DMA'd
    from), flags on the `ROM` region, and a way for symbol maps to say which overlay an
    address belongs to (splat's segment list). Code the game decompresses or relocates
    can't be traced this way.
  - **A fuller symbol map and a real match.** Only 237 main-segment functions are named, so
    each runs on to the next name; the recomp's `syms/dump.toml` (1,874 function starts)
    would give real bounds. Record a segment that plays into a match so `ovl_e` is the
    overlay that ran.
  - **Rebuild the core for `dj_forced_options`**, so `[core.options]` records the pure
    interpreter and the other forced options instead of what the options said.
  - **`[WRITE]` on the N64 baseline is byte-swapped.** The core keeps RDRAM in host-endian
    32-bit words, so `[WRITE] RAM 0x10 01 02 03 04` lands as `04 03 02 01` on a
    little-endian host (from reading the code: `retro_get_memory_data(SYSTEM_RAM)` returns
    `g_dev.rdram.dram`, a `uint32_t` array; not tried yet). The connector should say how a
    region's bytes are ordered (or hand out a write function), and the player should swap.
- **Writing RAM in a decomp with another layout.** `[WRITE]` lines reach a comparison core
  through `dj_write(baseline, region, address, bytes, len, reason)` (0 done, 1 ignored on
  purpose, else an error); without it, a `dj_sync` core is synced again after the frame.
  The port decides what a write means, so a decomp that keeps the original layout (the
  Tetris decomp keeps the NES's 2 KB of RAM 1:1) copies the bytes, and one with its own
  layout can already translate a baseline address to its own variable inside `dj_write`.
  That puts the translation in each port's code. Options to share it:
  - **Leave it in `dj_write`** (now): fine while ports are few and hand-written.
  - **An address map** in the project (baseline address range → decomp symbol, with type
    and byte order), read by the player, which calls a new `dj_write_symbol(name, offset,
    bytes)`. The same map would serve parity's RAM compare for ports with another layout
    (see "Emulator vs decomp mismatches").
  - **`[WRITE]` by symbol name** (`[WRITE] RAM g_match_clock 00 10`): resolved to an address
    through the baseline's symbol map (`[[data]]` entries) and passed to the decomp by name,
    so a segment says what it pokes rather than where. Needs a format addition (older tools
    reject unknown forms).
  Lean: keep `dj_write` as the contract and add the name: `[WRITE]` by symbol name, with
  the name passed alongside the address so a port can use either.
- **Later:** the comparison core in the CLI and MCP (`dj parity`), its coverage, sound
  (an A|B toggle), and more than one comparison core at a time.
- **Emulator vs decomp mismatches** (decided: stop at the first failing frame, no resync;
  see parity above). Still open: what counts as a match when the decomp's RAM layout
  differs from the original game's (a map between addresses, or only watched values), and
  whether video differences count or are only logged.
- **Comparison cores can define their own parity.** A decomp may be a port with a completely
  different architecture (a PC port of an N64 game, say), so comparing RAM byte for byte
  means nothing. Let a comparison core's connector decide what a match is: an optional
  function in the connector API ([connectors.md](../docs/connectors.md#the-api)), found
  the same way as the others through the proc address callback, for example
  `bool dj_check_parity(const struct dj_parity_input *in)`. The input would carry pointers
  to the baseline's memory regions (with names and sizes), the frame and tick, and maybe the
  baseline's video frame; the core compares against its own state and returns true for
  parity. It could also fill in a short reason or the first mismatch it found, for the
  report. A core without the function falls back to the default check (RAM while
  stepping, below). Open: how much of the baseline to hand over each frame without
  slowing stepping down, and whether the check runs every frame or only at ticks.
- **Open question: how parity is checked.** Nothing is decided yet. The options are:
  - **RAM while stepping.** Run both cores frame by frame and compare RAM (or the
    watched regions) after each frame. It's cheap, and it works with any core. A
    block run on a frame where everything matched gets parity. A mismatch flags
    everything executed on that frame as a suspect, but doesn't say which block
    caused it. It also needs the decomp's RAM layout to match, or an address map.
  - **Tracing.** Hook function entry and exit in both cores (using the symbol maps)
    and compare arguments, return values and memory writes for each call. It finds
    the exact function that diverges, but it's costly, and it needs a mapping
    between baseline addresses and decomp functions.
  - A likely path is RAM while stepping to find the first frame that differs, then
    tracing just that frame to find the function. That also fits the plan of
    stopping at the first mismatch and reporting it.

## Code

- **Refactor the Rust before release (maybe).** Once the tool is close to a first working
  release, consider splitting the longest files by feature. `gui/src/main.rs` is about
  3,000 lines and holds playback, editing, undo, the tree, cleaning and the transport
  bar. It could follow `roll.rs` and `cov.rs` into modules such as `playback.rs`,
  `edit.rs`, `tree.rs` and `transport.rs`. In the player, `format.rs` and `libretro.rs`
  are the next largest. This changes no behaviour, so it may not be worth doing at all.
  It also pairs naturally with the terminology rename below.
- **Terminology in the code.** The docs and the GUI use the
  [terms in concepts.md](../docs/concepts.md#terms). The code and the CLI's help still say "replay", "collection"
  and "trunk" in places. (Segment files start with `dj-segment 1` since 2026-11-10.)

## Set aside

Not planned for now. They're kept so the idea and its reasoning aren't lost if it comes up again.

- **Markers tab: group repeated texts.** Collapse markers with the same text into one row.
- **A `[[compare]]` setting to skip a power-on's first frames** (say `parity_from = 60`).
  Not needed: the comparison core decides which frames to skip, with `dj_check_parity`'s SKIP.
- **Finding functions without a symbol map, and a jump-table helper.** A recursive-descent
  disassembly from the vectors and `JSR` targets, refined by coverage, writing a generated
  map; and working out a jump table's end from the words after its `jsr`. Set aside because
  users bring their own symbols: dj doesn't do static analysis. The linear sweeps the MCP
  tools used without a map were removed for the same reason.
