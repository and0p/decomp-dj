# Agents and MCP

`dj mcp` serves a project to an AI agent over the [Model Context Protocol](https://modelcontextprotocol.io).
An agent can ask where code runs and trace it, find what isn't covered yet, drive the game
frame by frame, write inputs as segment text, and run search jobs. What it plays becomes
**drafts**: segments in `.local/drafts/` that join the project only when someone saves them,
so git stays the safety net.

The same queries are CLI commands with `--json` ([cli.md](cli.md)), for scripts and CI.

## Setting it up

Build `dj` ([install.md](install.md)), clean the project once (`dj clean <dir>`), then
add the server to the agent's client. For Claude Code:

```sh
claude mcp add decomp-dj -- /path/to/dj mcp /path/to/project
```

Or in a client's JSON config:

```json
{ "mcpServers": { "decomp-dj": { "command": "/path/to/dj", "args": ["mcp", "/path/to/project"] } } }
```

The ROM and core come from `.local/local.toml`, like every other command.

### What an agent may do

| `--allow` | Tools |
| --- | --- |
| `query` | Queries only: no core is started |
| `draft` (the default) | Also sessions, drafts, traces and jobs. Nothing in `segments/` changes. |
| `save` | Also `save_draft`, which writes `segments/` |

Pass `--allow` on the command line, or set it per machine in `.local/local.toml`:

```toml
[mcp]
allow = "save"
```

## How it runs

There is no daemon. The agent's client starts `dj mcp`, and it exits when the client closes
its stdin. Anything that needs a core runs in a **worker**: a child `dj` process with one
core (a libretro core has global state, so one core per process). Workers read commands on a
pipe from `dj mcp` and exit when it closes, so they end with it even if it crashes; on Linux
they are also killed with it (`PR_SET_PDEATHSIG`).

- Each session is a worker. Up to 6 can be open; a session idle for 30 minutes is closed (its draft stays).
- Traces, `snapshot_at` and `save_draft` share one more worker.
- A job runs its own workers, 4 by default.

Several `dj mcp` servers and the GUI can use one project at once. The database is in WAL
mode, and writers wait for each other for up to 10 seconds: a worker writes when it starts
(the setup and the current segments) and when a segment plays with recording on, such as
an ancestor that has no stored end state yet. Queries reopen the
project when a segment, symbol map, `config.toml` or the database changed on disk, so an
agent (or you) may edit those files directly between calls.

## Addresses

Every tool that takes an address accepts:

| Form | Meaning |
| --- | --- |
| `PRG:0x1F3A` | An offset in a ROM region |
| `0x1F3A` | An offset in PRG |
| `$C123` or `cpu:0xC123` | A CPU address, found through the symbol maps (the first bank that has it; the result notes others). Outside them, a NES ROM with at most 32 KB of PRG maps it directly, since that is too little to bank |
| `practiceDoClick`, `practiceDoClick+0x2b` | A symbol, optionally plus an offset |

Results are short by default: lists take `limit` (default 10) and return `next_cursor` when
there is more.

## Queries

| Tool | Returns |
| --- | --- |
| `where_runs(addr)` | Segments that run this code, best first (counting before dirty, shallow, early): the first frame that runs it, its depth, and the nearest stored snapshot |
| `coverage_summary()` | Instructions, blocks and functions covered, segments left out, the segments that add most |
| `gaps(sort, only)` | Functions with uncovered code. `sort=size` puts the most uncovered first; `sort=nearness` puts first those a covered branch skips. `only=never\|partial\|all` |
| `function(addr)` | Every block of one function: covered or not, the first segment and frame that runs it, where control goes next; its callers (JSR and JMP sites) and whether they run; the segments that enter it |
| `frontier(addr?)` | Covered branches and calls that lead into uncovered code, with the segments and frames that run that branch. For an address in uncovered code, the ways into its block; for a function never entered, its covered callers; with none, the largest across the ROM |
| `segments(filter?)` | The tree: id, title, parent, frames, status, poked, children |
| `gui_view()` | What the person is looking at in `dj-gui`, when they share it ([below](#the-gui)) |

### Without a symbol map

`where_runs`, `coverage_summary` and the play tools need no symbols. `gaps`, `function` and
`frontier` work from coverage instead, on NES ROMs (they read the bytes as 6502), and say so
in a `note` field; on other ROMs they report that they need `[[symbols]]`.

| Tool | Returns |
| --- | --- |
| `gaps(sort)` | The runs of PRG bytes that no instruction that ran occupies, with their size, `ways_in` (covered branches and calls that lead into them) and `best_way_in`. `sort=size` puts the largest first; `sort=nearness` puts first those with a way in. `only` doesn't apply. These include data, such as tables and padding |
| `function(addr)` | The routine a linear sweep finds around the address, named `sweep_XXXX`: back through instructions that ran and lead to it, forward to the first place control can't pass, up to 1 KB each way. Blocks and first runs as usual; callers are only those that ran, since code that hasn't can't be told from data |
| `frontier(addr?)` | Branches and calls that ran and lead into code that hasn't. With an address, those into the uncovered range that holds it. Its `uncovered_instructions` counts straight on from the target, to the first place control can't pass |

A sweep can read data as code when it starts in uncovered bytes. Targets of covered branches
and calls are real code, so start there. With more than 32 KB of PRG, CPU addresses are
unknown without the bank: results use `PRG:` offsets, and calls aren't followed.

Agents can also read the project directly: segment files, `config.toml`, symbol maps and
`report/` are plain text, and `dj sql` or `sqlite3` read `.local/cache.db`. Writing the
database directly isn't safe; everything in it can be rebuilt by cleaning.

## Traces

| Tool | Returns |
| --- | --- |
| `trace(addr, segment?, frame?, before=20, after=120, frames_before=0, frames_after=0)` | Plays the best segment that runs the code (or the one given) from its nearest snapshot, with the connector's trace hook on, and returns the instructions around the first run: address, disassembly with symbol names, registers before each instruction, and the data it read (`r$0201=05`) and wrote (`w$0300=05`). The whole traced span, every frame of it, goes to `.local/traces/` |
| `calls(addr, limit=3, per_segment=3)` | For the best segments that run a function: each call in its first frame, with registers on entry and on return, instructions and cycles, the memory it read before writing it (its inputs) and what it wrote |
| `snapshot_at(segment, frame)` | Writes the core state there to `.local/snapshots/`, to load in another tool or debugger |

```
$9884 JSR practiceDoClick  A:80 X:FF Y:00 S:FD P:85 w$01FD=98 w$01FC=86
>>> $98A0 LDA $04E4        A:80 X:FF Y:00 S:FB P:85 r$04E4=80
$98A3 CMP #$24             A:80 X:FF Y:00 S:FB P:85
$98A5 BCC $98C2            A:80 X:FF Y:00 S:FB P:05
```

Traces need a connector with the trace hook ([connectors.md](connectors.md#traces)). The
disassembly is 6502 only so far.

## Sessions

A session is a core parked at a point, in its own worker. Everything it plays is written to
its **draft**, so nothing played is lost and any moment can be saved.

| Tool | What it does |
| --- | --- |
| `open_session(from, frame?)` | From a segment: a new draft, either a child at the segment's end (the default) or, with `frame` inside the segment, an alternative that shares its first frames (stored snapshots get it there fast). From a draft: continues it, with the cursor at `frame` or its end |
| `step(frames=1, press?)` | Runs frames, pressing `press` (`["A", "RIGHT"]`, `P2 A` for port 2) for them and releasing afterwards |
| `step(until, max_frames)` | Runs at full speed until a [condition](#conditions) holds, or `max_frames` pass |
| `hold(buttons)`, `release(buttons?)` | Press until released; `release` with no buttons releases all |
| `play_text(text, frames)` | Writes a chunk of segment body at the cursor and plays it |
| `screenshot()` | The picture after the last frame run |
| `read_memory(region, addr, len)` | Bytes of `RAM` (default), `SRAM`, `VRAM` or `BUS` (the CPU address space) |
| `poke(region, addr, bytes)` | Writes now and records a `[WRITE]` line, which makes the draft poked |
| `rewind(frame)` | Goes back without changing the draft; the session keeps a snapshot every 10 frames, so this is cheap |
| `branch(title?)` | Copies the draft up to the cursor into a new draft, to keep a point worth returning to |
| `close_session(keep?)` | Ends the worker. Its draft is deleted unless `keep=true`. A kept draft is played once more with recording on, so the local database has its coverage and end state: the GUI shows what it adds and records its hashes without playing it |
| `sessions()` | Open sessions with their drafts and frames |

Writing anywhere but the end of a draft (a press, `hold`, `poke`, `play_text`) cuts the
draft at the cursor first, the way recording over a tape does. Stepping inside a draft
without writing replays what is there.

Every step returns the frame, the inputs held, ticks, and **new coverage**: instructions that
no counting segment had run (and that this session hadn't yet), by function, such as
`{"instructions": 15, "in": ["practiceDoClick +14", "practice_menu +1"]}`. Pass
`screenshot: true` to any step to get the picture with the result.

### Conditions

`step(until=...)` and `seek_goal` take a small expression language:

```
RAM:0x44 >= 2                    a byte of a memory region (RAM, SRAM, VRAM, BUS)
u16(RAM:0x10) > 1000             two bytes, little-endian
ran(practiceDoClick+0x2b)        that code ran as an opcode since the call started
frame >= 300                     frames since the call started
RAM:0x44 == 2 && !(RAM:0x45 & 0x80)
```

Operators, loosest first: `||`, `&&`, comparisons, `|`, `&`, `+ -`, prefix `!` and `-`.
Numbers are decimal or `0x` hex.

`max_frames` is required for every call that runs until a condition. Reaching it is a normal
result, not an error: the call returns where it stopped, `condition_met: false`, the values
each memory reference took (first, last, min, max), and for a comparison with a number, the
closest it got (`closest_distance`). Deciding when a goal is reached is otherwise the agent's
job, from screenshots and memory reads; conditions are for running at full speed between looks.

## Drafts

A draft is an ordinary segment file in `.local/drafts/`, with a few extra `draft-` header
fields (where it came from, a note, a condition it was searched for). The GUI shows drafts
in the tree as unsaved segments ([below](#the-gui)). They are not counted in coverage or
`where_runs` until saved (reports show what they would add), and `dj hydrate` and
`dj clean` skip them.

| Tool | What it does |
| --- | --- |
| `draft_from_text(text, parent?, frames?, title?, note?)` | Takes a whole segment file, or body lines plus `frames` under `parent` (a segment or draft; none for a root). Plays it in a new session and returns the session, end frame and new coverage. Mistakes come back with the line number |
| `append(draft, text, frames)` | Adds a chunk at the end: body lines numbered from the chunk's start, and how many frames it lasts (so no trailing wait lines). Plays only the new frames. Build long segments this way: write 300 frames, look, write the next 300 |
| `truncate(draft, frame)` | Cuts a draft back, to retry a chunk |
| `drafts()`, `draft(id)` | List them, or read one's text |
| `discard_draft(id)` | Deletes it |
| `save_draft(id, title?, parent?)` | Needs `--allow save`. Plays the draft once with recording on, writes its hashes, and moves it to `segments/` as a normal segment file (so it is a normal diff in git, undone with `git restore` or `rm`). A draft whose parent is a draft waits for that one to be saved first |

Text uses the segment body format ([segments.md](segments.md#input-lines)):

```
0  P1 JOYPAD DOWN 1
20 P1 JOYPAD DOWN 0
30 P1 JOYPAD A 1
34 P1 JOYPAD A 0
```

Held inputs carry across chunks, as they do within a segment, and a later chunk can release
them. A draft built this way saves as the text the agent wrote, plus hashes.

When `dj mcp` exits with sessions still open, drafts that played anything are kept (with a
note) and empty ones are deleted.

## Jobs

Jobs search in the background, across several workers, and leave their results as drafts.

| Tool | What it does |
| --- | --- |
| `fuzz(from, frame?, max_frames, seconds=120, workers, chunk=120, buttons?, max_drafts=20, seed?)` | Random joypad input from a segment or frame. A chunk that runs code nothing had run is kept and saved as a draft titled by what it reached (`fuzz: +25 in practiceDoClick (20), doStateNewLevel (3)`); other chunks are undone and tried again. A worker that keeps failing sometimes keeps a chunk anyway to get somewhere new, and starts over when its draft grows too long |
| `seek_goal(from, condition, frame?, max_frames, ...)` | The same search, climbing toward the condition: a chunk is kept when it brings a comparison closer, or reaches new code when closeness can't be measured. The input that gets there becomes a draft titled `goal: ...` |
| `job_status(job?)` | Frames played, attempts, seconds it has run (they stop counting when it ends), new instructions, drafts made (most new code first), the closest a goal got, `error` and `workers_failed`, and the status: running, goal reached, finished, stopped, failed |
| `job_stop(job)` | Stops it; its drafts stay |

`max_frames` (frames across all workers) is required, and `seconds` caps the wall-clock
time, at most an hour. The search stops within a frame of that time, even inside a chunk,
and keeps what the chunk played. Getting a worker to the starting point comes first and
isn't interrupted, so a start far from any stored snapshot can take longer than `seconds`.
`job_stop` takes effect between chunks.

A worker that dies doesn't stop the others, since each searches on its own. `job_status`
shows `error` (the first failure) and `workers_failed` straight away, and the job ends as
`finished: budget used, on the workers left (1 of 4 died)`. A goal reached or `max_drafts`
stays the status even if a worker died along the way. With no worker left, the job is
`failed: 4 of 4 workers died`.

START and SELECT are pressed rarely, since they pause and leave menus;
pass `buttons` to choose others. A `seed` makes a run repeatable on one worker.

## The GUI

`dj-gui` works alongside agents through files in `.local/`, so the two need no connection.
It is off until you turn on **MCP > Enable MCP**, which is remembered for the project on
that machine (`[mcp] gui` in `.local/local.toml`). The right of the status bar shows the
state: `MCP off`, or the number of agent sessions running, and whether you are following one
or sharing your view.

- **Drafts** appear in the tree as unsaved segments (yellow italics), marked `agent` (and the session they are open in). They are part of the editor's tree like your own unsaved work: playing one to its end records its hashes, Clean all segments cleans it, the Coverage tab shows what it adds, and **File > Save** moves it into `segments/` with any edits you made. **Delete** removes one (its file goes when you save), and undo brings it back. A draft still open in an agent session is light blue and read-only, stays a draft when you save, and isn't recorded into; right-click it to **Follow session**, or **Take over** to have the session let go of it (it closes at the agent's next call, keeping the draft) and make it yours.
- **MCP > Follow an agent session** lists running sessions (`dj mcp` publishes them in `.local/mcp/`). Following one shows its draft at the agent's frame as it plays, with its inputs filling the piano roll. **Stop following** takes the view back.
- **MCP > Share my view with agents** writes the segment and frame you are on to `.local/mcp/gui.json`, which the agent reads with `gui_view`. Ask it to look at what you're looking at, or to take it from there: it opens a session at that point, and you can follow it.
- **MCP > Show agent drafts** hides or shows them.

## Limits

- Buttons are joypad buttons; other devices (mice, analog sticks) go through `play_text` with segment lines.
- Fuzzing and goal searches press joypad buttons at random; they know nothing about the game.
- Traces and their disassembly are 6502 only, and only the Mesen connector has the trace hook.
- The GUI follows and shares through files, a second or so behind. It can't hand its own running core to an agent; the agent opens its own session at the same point.
- On Windows, workers end when `dj mcp` closes their pipe, but there is no job object yet to end them if it is killed. Windows hasn't been tried.
