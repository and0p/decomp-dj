# Roadmap

Ideas and open questions that are not part of format version 1, and the current limits.
When one ships, its user-facing description moves into `docs/` and the entry here goes.

## Known limits

- Only one core has a connector (Mesen, NES).
- Block and function rollup is 6502-only. Other CPUs get raw executed-byte counts until their disassembler is added.
- Determinism has been checked on Linux x86_64 and macOS arm64; Windows still needs a run.
- The GUI is a first prototype: button inputs only (no analog, lightgun or pointer rows yet), keyboard-only recording for one controller at a time, one core at a time. Tested on Linux and macOS; Windows not tried yet.
- `exclude` in `[cache]` is recorded but not applied to snapshots yet.

## GUI
- **Replay everything as a pausable list play.** Segment > Replay everything runs as a
  one-off job today. Make it ordinary playback in **List** end mode, started from frame 0 of
  the first root segment (the first branch in tree order), so it can be paused, resumed and
  watched like any other playback, and the transport's controls work on it.
- **Rewinding up the chain.** Stepping or seeking back past frame 0 goes into the
  parent's last frames (loading a snapshot of the parent, from memory or the local
  database), and playing forward from there follows the sticky segments back down. A
  possible next step is one timeline across the whole chain, with the segment
  boundaries drawn on the ruler.
- **Drag to draw in the piano roll.** Today a click makes a press as long as the
  last one made or resized, and a drag on empty space selects. Holding the button down
  on an empty cell and dragging could set the new press's length instead, with
  selecting moved to a modifier or to drags that start outside the rows.
- **A file watcher instead of polling.** The GUI learns about drafts, sessions and the
  shared view by fingerprinting `.local/` about once a second, so following an agent shows
  one step in several (a 60-frame `step` takes about 100 ms at 500 to 700 fps) and new
  drafts arrive late. Watch `segments/`, `.local/drafts/` and `.local/mcp/` with the
  `notify` crate (inotify, FSEvents, ReadDirectoryChangesW) and react within milliseconds,
  keeping the fingerprint poll as a slow fallback for file systems that don't notify
  (network mounts). Since every writer writes to a temporary file and renames, a change event always finds a whole file.
  This keeps the no-daemon design and costs about a day; it fixes the lag and the
  half-written reads, which are the file-system part of the draft problems. Ownership
  (agent-owned segments) isn't, and stays as it is. Sockets
  ([below](#project-and-cli)) are then only needed for attach.
- **Stop at a changed ending.** An opt-in toggle (off by default) that pauses playback as
  soon as a segment records an ending that differs from its saved file (the **!**), so the
  author can undo before the children below it replay. By default playback carries
  straight on, since a child always starts from where its parent really ends.
- **Recording follow-ups** (recording itself is in docs/gui.md#recording).
  - **Gamepads.** Device discovery, hot-plugging, analog dead zones and per-OS differences
    make them a larger job than the keyboard.
  - **Per-console default bindings.** The first version has one default layout (WASD for the
    d-pad, J = B, K = A, U = Select, I = Start) for one controller. Each console (NES, SNES,
    N64, ...) should get its own default that covers its buttons, picked from the core or
    the project's system.
  - **Two players in real time.** Bindings for a second controller at once.
- **Audio playback (GUI).** Play the core's audio while a segment plays, with a volume
  slider and a mute button in the playback toolbar. When more than one core is running
  (side-by-side comparison), an A|B toggle picks which core you hear.
- **More ways to join.** Join with only child (the same operation, offered on the parent);
  merge into children (copy the parent's frames onto the front of every child, then remove
  the parent: no hash changes, but the frames are duplicated); and join a chain (select a run
  of only-children and join them all at once).
- **Instead of deleting,** merge a segment into its children, or move its children up to the
  grandparent, which turns everything below dirty.
- **Moving segments** to another parent from the GUI.
- **Markers tab:** group repeated texts, follow the tree's Focus here, and list a marker's
  "first frame where a watched value differs" once that exists.
- **A warning before losing unsaved edits** on Open, Reload or Quit, and cancelling a long
  load without selecting another segment. MAX speed off the UI thread.

## Coverage

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
- **Coverage in the GUI:** the Coverage tab ([coverage.md](../docs/coverage.md#in-the-gui)) shows the report, a ROM map
  and live new coverage while playing. It rebuilds the whole report after every edit,
  which takes milliseconds at NES sizes. For large games, keep coverage per segment and
  per function or larger block, sum it in a cache (`coverage_sum`, [CACHE_PLAN.md](CACHE_PLAN.md)), and
  update only the sums for segments whose clean or dirty status changed. Parity colors
  (purple and red) join the map once comparison cores exist.
- **Tracking coverage lost.** When an edit, a delete or a re-recorded segment means code
  that used to be covered no longer is, nothing says so: the totals just drop. Keep the
  previous coverage per segment (or per report), and show what a change cost: in the GUI
  after a save or Verify ("−14 instructions in `doStateNewLevel`"), and in `dj report`
  against the last committed report. Pairs with the committed report above, whose diff
  would show it in review.
- **"First runs in" needs its own view.** The function details in the Coverage tab list
  every segment where the function first runs (capped at six today). For code that runs
  every tick (the NMI handler, the main loop) that is every segment in the project, so
  the list is noise. Move it into a separate view built for large result sets, for
  example a searchable table of segment, frame and depth, grouped by root segment, or
  just "earliest, shallowest, and N more", with the full list one click away.
- **Symbol maps during active decompilation.** Symbol maps in the formats a decomp already
  keeps are supported ([coverage.md](../docs/coverage.md#symbol-maps)), so the decomp stays the source of truth. Still to do:
  let `.local/local.toml` point a `[[symbols]]` entry at a file in the decomp's own
  checkout, since that path differs per machine; reload when the file changes, so a
  rename shows on the coverage map and list at once; and maybe an opt-in "Name this
  function" action in the Coverage tab that appends a line to `symbol_addrs.txt` (or
  the `.mlb`). Rolling up regions other than `PRG`, and a MIPS sweep for N64, come
  with the N64 connector.

## Project and CLI

- **Versioning projects.** A project should say which version of the format and the
  tools it needs, and the tools should handle the difference both ways. Today
  `config.toml` has `format = 1` and `dj-replay 1` heads each segment, and older
  layouts are upgraded silently on open ([older projects](../docs/segments.md#older-projects)).
  To decide:
  - A newer tool opening an older project: upgrade in place, as now, or ask first and
    show what changes (a diff or a backup), since the upgrade lands in the author's git
    history. A `dj upgrade <dir>` command could do it explicitly.
  - An older tool opening a newer project: refuse with a clear message naming the
    version needed, rather than misreading it. Which changes need a version bump and
    which can be new optional header keys that older tools ignore.
  - The core and setup: rebuilding a connector core changes the setup id, so every
    segment's hashes and the database stop matching (this happened with the trace hook).
    Options: a project records the connector version it was made with, a rebuild that
    keeps emulation identical keeps the setup id, or `dj` re-pins and re-verifies the
    tree after a core change and reports which segments still match.
  - Recording which `dj` version last wrote a project, for bug reports.
- **Backups without version control.** `dj backup <dir>` (and File > Back up… in
  the GUI) writes the project's shared files to a ZIP: `config.toml`, `segments/`,
  `blobs/`, `symbols/` and `unreachable.txt`, never `.local/` (the database can be
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
  - Stable SQL views (`v_first_hits`, `v_function_coverage`) so agents can query the database without knowing its tables.

## Comparison and parity

- **Comparison cores in the project.** The baseline stays fixed, but a project
  can list several comparison cores (for example two decomps, or a decomp and
  another emulator) in `config.toml`, in version control, each with a name and
  the core it uses. Parity is tracked per comparison core.
- **Side-by-side comparison** with a second libretro instance, namely a decompilation
  built as a core. Its video output may or may not match. The same inputs drive both,
  and the GUI compares state hashes and video.
- **To discuss: emulator vs decomp mismatches.** When the decomp core and the
  emulator disagree (state hash, RAM, or video), the comparison should stop ticking
  and report the first difference it caught: the frame, the regions or addresses
  that differ, and both values. It should not try to resynchronize. Open questions:
  - What counts as a match when the decomp's RAM layout differs from the original
    game's (a map between addresses, or only watched values)?
  - Should video differences stop the run, or only be logged?
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
- **Open question: frame parity vs block parity.** A comparison core's `dj_check_parity`
  gives a verdict per frame: after frame N, it matches the baseline or it doesn't. The
  coverage map's parity colours want a verdict per block or instruction: this code ran and
  behaved the same. Settle whether parity is one, the other or both, and if blocks are
  judged from frame verdicts, the rule (for example, every block run only on matching frames
  has parity, and every block run on a failing frame is a suspect). Decided so far: loading,
  syncing and ticking a comparison core come first, then parity.
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
  and "trunk" in places, and segment files start with `dj-replay 1` (`dj-segment 1`
  would match the `.segment` extension; the tools would read both).
