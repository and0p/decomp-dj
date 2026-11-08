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
- **Backspace in real-time recording.** Today the rewind key (Backspace) only works in
  frame-by-frame recording: `record_keys` returns before reading it when frame-by-frame is
  off. In real-time mode it should also step back while held, at the playback speed, and
  erase the input on the frames it leaves when "rewind clears" is on (Shift for the
  opposite), as it does frame by frame. Recording carries on forward from wherever it's
  released.
- **A console in the app.** A panel (View > Console, or clicking the status bar's message)
  with a scrolling, filterable log of everything that's now only a passing status message
  or stderr: errors and warnings, info messages, jobs (cleans, hydrates, seeks), comparison
  syncs and their failures, and the core's own log (libretro's `GET_LOG_INTERFACE`, which
  the frontend could route here per core slot). Copy and clear buttons; also written to
  `.local/gui.log` for bug reports. The MCP console (under "Project and CLI") could be
  one tab of it.
- **A-B loop.** Loop a chosen stretch of frames, like VLC's A-B repeat: set A and B at the
  playhead (buttons in the transport and keys, or drag a range on the frame ruler), and
  playback jumps back to A on reaching B until the loop is cleared. Shown as a band on the
  ruler with handles to adjust it. Open: whether a loop may cross segment boundaries (from a
  parent into a child, say) or stays in one segment; how it relates to the Loop end mode
  (which loops the whole segment); whether it's per segment and remembered (in the local
  database or `.local/`) or just for the session; and how it works while recording (likely
  re-recording over the loop each pass, with overwrite on). Jumping back to A uses the
  nearest snapshot, so the comparison core re-syncs at A each pass.
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
- **More ways to join.** Merge into children (copy the parent's frames onto the front of every child, then remove
  the parent: no hash changes, but the frames are duplicated). (Join a chain is done:
  Segment > Join chain….)
- **Instead of deleting,** merge a segment into its children, or move its children up to the
  grandparent, which turns everything below dirty.
- **Moving segments** to another parent from the GUI.
- **Choosing a snapshot in a root segment's properties,** the way the save file (`sram`)
  can be chosen now.
- **Record button as an icon.** Make **● Rec** a circle icon, a little wider than tall like
  the play button, instead of the text label.
- **Revert one segment.** Throw away the unsaved changes to a single segment and put back
  its saved file, without File > Reload dropping every other edit. In the File menu (for the
  selected segment) and on right-click in the tree. One undo step; a reverted segment with
  an unsaved parent or children may need a word in the confirmation.
- **Save one segment.** Write only the selected segment's file, leaving other edits unsaved.
  In the File menu and on right-click in the tree.
- **Back and forward, like a browser.** A history of where you've been, as segment and
  frame: selecting another segment (in the tree, the Coverage tab's "First runs in", a
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
- **Markers tab:** group repeated texts, follow the tree's Focus here, and list a marker's
  "first frame where a watched value differs" once that exists.
- **Cancelling a long load** without selecting another segment, and MAX speed off the UI
  thread.

## Coverage
- **Shipped: annotations.toml.** `[[unreachable]]`, `[[data]]` and `[[function]]` entries
  with a `why`, replacing `unreachable.txt`. Data is skipped by the sweep and left out of
  totals in both modes; code that runs inside an annotated range warns; `dj annotations`
  lists and checks the entries. Tetris carries its unreachable two-player code and the
  seven `switch_s_plus_2a` jump tables.
- **Editing annotations in the GUI (later phase).** Right-click a map square or a range
  in the Coverage tab to mark it unreachable or data, or to name or resize a function, and
  write the entry (with a `why` prompt) to annotations.toml. Possibly a jump-table helper
  that, given the `jsr`, works out the table's end from the words that follow.
- **Find functions without a symbol map.** For games with no decomp or disassembly (most
  NES games), generate a symbol map instead of requiring one. Seed a recursive-descent
  disassembly from the reset, NMI and IRQ vectors and every `JSR` target, following branches
  and jumps to find blocks and function bounds. Add the coverage flags: every byte flagged
  as an executed opcode is a proven instruction start, and anything reached from one is
  proven code, which resolves indirect jumps and inline jump tables (Tetris's
  `switch_s_plus_2a`) that static analysis can only guess at. Everything else is "not
  seen", neither code nor data. Output a map in the `csv` format with `auto_8161`-style
  names, kept until a real map replaces it, and regenerate it as coverage grows (new
  coverage can extend or split functions). Builds on `sweep_bounds_6502` and
  `call_target_6502`. Related: a connector flag for ROM bytes read as data (0x04; Mesen's
  code/data logger already tracks this), or importing Mesen `.cdl` files, to mark data
  positively. splat is no help here: it is MIPS only (N64, PS1, PS2, PSP) and needs the
  code/data split written by hand; its output is already readable via `[[symbols]]`. A
  headless Ghidra export (CSV) is another source for larger systems.

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
  dropdown and Run toggle, `dj_sync` (with `DJ_SYNC_BOOT`) or save-state sync after
  jumps, `dj_write` for `[WRITE]` lines, and the pictures side by side
  ([gui.md](../docs/gui.md#comparison-cores), [connectors.md](../docs/connectors.md#comparison-cores)).
  GUI only; the comparison core's coverage hooks are ignored.
- **Next: parity (phase 2).** `int dj_check_parity(baseline, frame, char *reason, size_t)`
  returning MATCH, MISMATCH, SKIP or ERROR with a reason, called after each frame both
  cores ran (redundant right after a sync). A **Check parity** checkbox, off at launch and
  kept for the app session (not saved), and a **Pause on first parity failure** option.
  Parity is tracked per comparison core.
- **Shipped: OpenGL cores (HW_RENDER).** A private offscreen context per core slot (CGL on
  macOS, EGL elsewhere), read back on captured frames ([install.md](../docs/install.md#cores-that-draw-with-opengl)).
  Next: Vulkan (the negotiation interface, `ash`, MoltenVK on macOS), then maybe OpenGL ES
  through EGL, Windows (WGL), and asynchronous readback (PBOs) or showing the texture
  directly if readback costs too much at high internal resolutions.
- **Portable `[[compare]]` entries.** config.toml is shared and may be published, but a
  comparison core's `path` names one machine's file (`cores/foo.dylib`), and the same core
  is `.so` on Linux and `.dll` on Windows. Keep machine-specific parts out of the shared
  entry: config.toml carries `name`, `sync`, `options`, `build` and `sha256`, and each
  machine's `.local/local.toml` adds `path` (entries already merge by `name`). Options: a
  `path` without an extension, with the platform's suffix added (`cores/foo` finds
  `foo_libretro.dylib`/`.so`/`.dll`); per-platform paths (`path.macos`, `path.linux`); and
  `sha256` per platform, since builds differ. `dj` could warn when config.toml holds an
  absolute or platform-specific path, and an entry with no `path` here would show in the
  Compare dropdown as "not set up on this machine" instead of failing on pick. The same
  applies to `[core] path`.
- **Later:** the comparison core in the CLI and MCP (`dj parity`), its coverage, sound
  (an A|B toggle), and more than one comparison core at a time.
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
