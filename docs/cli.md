# The CLI

`dj` is the headless player. It plays segment trees on any libretro core, checks state
hashes, fills the local database and reports coverage. Building it is in
[install.md](install.md).

| Command | What it does | More |
| --- | --- | --- |
| `dj init [dir] --rom ROM --core CORE [--name NAME] [--option KEY=VALUE]... [--symbols FILE]` | Creates a project and pins the setup in `config.toml`. Without `dir`, the folder is named after the ROM file; without `--name`, the project is named after the folder | [install.md](install.md#a-first-project), [config.md](config.md) |
| `dj new [--parent ID] [--title TEXT] [--frames N]` | Adds a segment and prints its path | [editing.md](editing.md) |
| `dj new --snapshot FILE [--sram FILE] [--start-frame N] [--blob-name NAME]` | Adds a root segment that starts from a save state, a save file or both, after checking that the core loads them | [editing.md](editing.md#a-root-from-a-snapshot) |
| `dj new --from SEGMENT --at FRAME` | Adds a root segment that starts where SEGMENT is at that frame | [editing.md](editing.md#a-root-from-a-snapshot) |
| `dj import-tas FILE [--title TEXT] [--strict]` | Imports a BizHawk TAS movie (`.bk2`) as a new root segment and prints its id | [tas.md](tas.md#importing) |
| `dj export-tas SEGMENT --out FILE [--through FRAME] [--author NAME] [--strict]` | Writes the chain from the root down to SEGMENT (through FRAME of it) as a BizHawk `.bk2` movie, leaving out what it can't carry, with a warning (`--strict`: refuse) | [tas.md](tas.md#exporting) |
| `dj status` | The tree with clean and dirty marks | [playback.md](playback.md#clean-and-dirty) |
| `dj clean [SEGMENT [--children]] [--accept] [--force]` | Plays the segments that aren't clean or aren't in the local database (or SEGMENT, with `--children` those below it too; `--force`: even what is stored), top down, and writes the hashes they reach into their files. Changed endings are listed and not written (the command fails) unless `--accept` | [playback.md](playback.md#recording-hashes), [database.md](database.md#from-the-command-line) |
| `dj check [SEGMENT...] [--tag NAME]... [--children] [--from-power-on] [--strict] [--junit FILE] [--screenshots DIR [--shot 60,120]]` | Plays every segment (or those chosen, each from the nearest stored state above it) and compares its hashes with the recorded ones; fails when one differs. A segment with no hashes recorded yet passes with a note, or fails with `--strict` (for CI). Writes no file | [playback.md](playback.md#checking-hashes) |
| `dj repin` | After rebuilding or updating the core: plays every segment from power-on and, if all still match, records the core's build in `[core] build` | [config.md](config.md#core) |
| `dj upgrade` | Rewrites every segment file in this dj's format, and records this dj in config.toml | [segments.md](segments.md#older-projects) |
| `dj controls` | Lists each port's controls: the core's name for each, the project's label, and whether the GUI shows it | [config.md](config.md#naming-the-controls) |
| `dj parity check [SEGMENT...] [--tag NAME]... [--children] [--compare NAME] [--again] [--dirty] [--json] [--junit FILE]` | Checks segments against a comparison core, each from its own start, at every verdict the core gives; exits 1 when one fails | [below](#parity), [ci.md](ci.md) |
| `dj parity report`, `rerun [ID...]`, `clear ID...`, `explain ID`, `core` | Work with the results kept, and inspect the comparison core | [below](#parity) |
| `dj summary [--segments]` | Coverage of the current, clean, saved segments: instructions, blocks and functions covered, what was left out (dirty, not played, drafts), and the segments that add most. `--segments`: each segment's share, and the smallest set that keeps all the coverage | [coverage.md](coverage.md#from-the-command-line) |
| `dj xref ADDR` (alias `where`) | Which segments first run that code, at which frame, and the nearest snapshot | [coverage.md](coverage.md#from-the-command-line) |
| `dj gaps [--sort size\|nearness] [--only all\|never\|partial] [--csv FILE]` | Functions with uncovered code, largest or nearest first. `--csv` writes every uncovered block to a file | [coverage.md](coverage.md#from-the-command-line) |
| `dj function ADDR` | One function's blocks (covered or not, first run, where they go), callers and the segments that enter it | [coverage.md](coverage.md#from-the-command-line) |
| `dj frontier [ADDR]` | Covered branches and calls that lead into uncovered code, with the segments that reach them | [coverage.md](coverage.md#from-the-command-line) |
| `dj annotations` | Lists `annotations.toml`'s entries and checks them against the coverage; fails when an annotated range ran as code | [config.md](config.md#annotationstoml) |
| `dj connector-check [--json]` | Developer tool: checks the coverage connector's flags in the local database against the decoder, inside the symbol maps' functions. Lists opcode flags where the decoder starts no instruction, operand flags that don't belong to an instruction that ran, operand bytes missing their flag, and opcodes run in both widths of a mode pair; fails when any disagree. For writing or updating a connector, not for normal use | [connectors.md](connectors.md#writing-one-for-another-core) |
| `dj mcp [--allow query\|draft\|save]` | Serves the project to an AI agent over MCP (stdio) | [mcp.md](mcp.md) |
| `dj sram SEGMENT FRAME --out FILE` | Writes the game's save RAM (battery save) at that frame, for `dj new --sram` | [editing.md](editing.md#a-root-from-a-save-file) |
| `dj blob list` | Lists the snapshots and save files in `blobs/`, with the segments that start from each | [segments.md](segments.md#blobs) |
| `dj blob rename BLOB NAME` | Renames a blob's file. No segment changes, since segments name its hash | [segments.md](segments.md#blobs) |
| `dj snapshot SEGMENT FRAME --out FILE` | Writes the core state at that frame, to start a trace from | [coverage.md](coverage.md#from-the-command-line) |
| `dj sql "SELECT ..."` | Queries the local database | [database.md](database.md) |
| `dj cache fill [SEGMENT [--children]] [--force]` | Plays into the local database what it doesn't hold yet, top down, and writes no file (records no hashes, so plays segments that aren't clean too) | [database.md](database.md#from-the-command-line) |
| `dj cache stats` / `dj cache gc [--force]` | Database size, or drop what no segment uses (not while dj-gui has the project open, unless `--force`) | [database.md](database.md#from-the-command-line) |
| `dj cache export [SEGMENT...] [--tag NAME]... [--children] [--compare NAME] --out DIR` / `dj cache seed FILE...` | Write the states that checking those segments starts from, named by state hash; store such states in another machine's database, each only if its hash is a clean segment's recorded end | [ci.md](ci.md#states-to-start-from) |

Common to every command:

- **The project** is the nearest folder with a dj `config.toml`, from the current folder up, so dj runs inside a project as git runs inside a repository. `-C DIR` (`--project DIR`), before or after the command, names another. It doesn't change the folder other relative paths (`--out`, `--junit`, `DJ_ROM`) are read from. `dj init [DIR]` takes the folder it creates as an argument (default: one named after the ROM file, in the current folder). A project folder given the old way, as the first argument (`dj check examples/thwaite`), is refused with a hint.
- **Colour.** Verdicts are coloured (red for a failure, green for a pass, yellow for what was left out) when the output is a terminal or `CI` is set. `NO_COLOR` turns it off; `CLICOLOR_FORCE` turns it on. JSON is never coloured.
- **JUnit.** `dj check` and `dj parity check` write a JUnit XML report with `--junit FILE`, one test case per segment, or into the folder `DJ_JUNIT` names ([ci.md](ci.md)).
- **Exit status**, the same for every command: 0 when all is well, 1 when the command ran and found something wrong (hashes that differ, a changed ending, a parity failure, an annotated range that ran, connector flags that disagree), 2 when it couldn't do its job (an error). A segment left out because it crashed or hung the core is a 1.

- Ids can be shortened to any unique prefix.
- `--rom` and `--core` are remembered per project in `.local/local.toml`, so later commands don't need them. `DJ_ROM` and `DJ_CORE` give them too, ahead of `.local/local.toml`, and are never saved into it ([ci.md](ci.md#the-rom-and-the-cores)). Commands that run a core take both; commands that only read the ROM (the queries, `annotations`, `parity report`, the TAS commands) take `--rom`; the rest take neither.
- `dj check`, `dj snapshot` and `dj sram` take `--no-db` to run without the local database. The other commands that play keep their results in it.
- **One meaning per flag.** `SEGMENT` is an id or a unique prefix. A command that takes one works on it alone, and `--children` adds every segment below it. `dj check`, `dj parity check` and `dj cache export` also take several, and `--tag NAME` for the segments with that tag. `--force` plays what the database already holds. `--json` prints JSON to stdout.
- `dj check`, `dj clean`, `dj cache fill` and the `dj parity` commands that play run in a child `dj` process that the command
  watches, so a core that crashes or hangs can't stop the batch. A crash, or a frame that
  doesn't finish within 30 s (a minute while the core loads), ends the child; the command
  prints `crashed  <id> <title> at frame N: the core crashed (signal 11, SIGSEGV)` (or "made
  no progress"), then runs again without that segment and the ones below it (shown as
  `blocked`), and ends with a list of what was left out and exit code 1. `clean` keeps
  what it stored before the crash; `check` plays from the start again.
  `DJ_WORKER_TIMEOUT_SCALE` multiplies the time limits (say for a very slow core);
  `DJ_IN_PROCESS=1` plays in `dj`'s own process, with no watching (for a debugger).
- **Warnings from inside a frame are counted at the end.** A `[WRITE]` line at an
  instruction that didn't run in its frame is not made ([segments.md](segments.md)), and a
  breakpoint that runs more than 1,024 times in a frame is switched off until the frame
  ends. Each prints a warning when it happens; `check`, `clean`, `cache fill` and
  `parity check` end with one line on stderr saying how many there were. The exit status doesn't
  change.
- The queries (`xref`, `summary`, `gaps`, `function`, `frontier`) take `--json` for scripts, `--limit` and `--cursor` for paging, and an address as `PRG:0x1F3A`, `0x1F3A` (a PRG offset), `$C123` (a CPU address) or a symbol, optionally `name+0x10`. They read the local database, so clean first (`dj clean`).

## Parity

`dj parity check` does what **Check parity** does in dj-gui ([gui.md](gui.md#parity))
without anyone watching, so a decomp's CI can run it after building its comparison core.
The other parity commands work with the results it keeps:

```
dj parity check --compare "decomp"        # every clean segment
dj parity check SEGMENT --children        # one branch
dj parity report                          # what the results say, per function; no core runs
dj parity rerun                           # failures from an older build, played again
dj parity rerun 12                        # one failure, played again
dj parity clear 12 13                     # forget failures by id; no core runs
dj parity explain 12                      # what differed on failure 12, and who wrote it
dj parity core                            # what the comparison core is, and its breakpoints
```

Each takes `--compare NAME`, and `--json` where it prints results.

- **Which comparison core:** `--compare NAME` picks a `[[compare]]` entry; it can be left
  out when there is only one.
- **Which segments:** the ids given, the segments with a `--tag` given, and with `--children` those below each; none of either, every clean segment. A tag no segment has is an error.
- **Each segment on its own.** The baseline jumps to the segment's frame 0 (the stored end
  state of its parent) and the comparison core is told of the jump. The segment then plays
  to its end, or to the first failure, which ends that segment's check; the others go on.
- **Where a verdict comes.** A core that takes events ([connectors.md](connectors.md#comparison-cores))
  judges where it chooses: at its own breakpoints in the original, or at frame ends. The
  counts are still in frames: passed (a match vouched for them), skipped (the core said
  it couldn't judge them) and not checked (no verdict covered them). A `sync = "state"`
  core is compared at every frame end. Segments that aren't clean are
  left out unless `--dirty` (clean them first: `dj clean`).
  `SEGMENT` checks that one alone; `--children` adds those below it.
- **Pre-roll.** When the `[[compare]]` entry sets `preroll = N`
  ([connectors.md](connectors.md#pre-roll)), the baseline starts N frames up, in the
  segments above, and plays to the segment's frame 0 with nothing judged, so a core that
  copies the baseline at a breakpoint has done so by then. A core that halts in those
  frames leaves the segment not checked, and it counts as failed. A failure says how many
  frames of its run were pre-roll, and `rerun` and `explain` mark the same ones.
- **Done once per build.** A segment whose content was checked with this build of the
  comparison core (the sha256 of its library) is skipped, with how it went then; `--again`
  checks it again. A rebuilt core is a new build, so everything is checked again.
- **Results** go into the local database the GUI reads: the code frames that passed ran,
  and each failure with the segment and frame, where in the frame the core gave its
  verdict (the name of its breakpoint, or "the frame end"; printed after the frame, and
  `at` in the JSON), the last jump before it (`jump` in the JSON: the run that found it
  started there), and the reason. A failure found again replaces the earlier
  one.
- **`dj parity report`** counts the instructions that ran by parity (with parity, failed, failed
  with an older build, not checked), per function with a symbol map, and lists the
  failures with their ids, newest first.
- **`dj parity rerun`** plays failures again the way they were found, each from its last jump, with the build there is
  now: the ids given, or every failure from an older build. One from an older build that
  passes is cleared; one that fails again is replaced. A failure with this build stays
  until cleared, as in the GUI.
- **Exit status:** 0 when nothing failed, 1 when a frame failed (including a segment that
  failed with this build before and wasn't checked again), 2 on an error. A comparison core
  that crashes or hangs is a failure at that frame, and is loaded again for the next
  segment; a baseline core that crashes leaves that segment and those below it out, as for
  `dj clean`.
- **What differed** is kept with each failure: the comparison core's own list
  (`dj_parity_diffs`, [connectors.md](connectors.md#parity)), or the runs of system RAM
  bytes that differ. `report` shows the first few under each failure.
- **`dj parity explain ID`** plays a failure again as `rerun` does, but keeps no results: it
  traces the original (the baseline core) over the failing frame and the `--frames-back N`
  before it (10 by default), and for each difference with an address gives the original's
  last instruction that wrote it, with its function. That is the code to compare with the
  decomp's. It also says whether the frame still fails and which functions it ran. Exit
  status 0 when it passes now, 1 when it still fails.
  - **The search stops where the core gave its verdict.** A core that judges at a
    breakpoint inside the frame compared the original as it was there. The original then
    ran on to the end of the frame, and what it wrote after the breakpoint is left out.
  - **Tracing needs the trace hook in the baseline's connector.** Mesen's and
    Mupen64Plus's have it. mGBA's doesn't yet: there, `explain` plays the failure again
    and lists what differed, with no writers.
  - **A long trace keeps its newest frames.** The frames are traced one at a time, and
    when they come to more than 12 million events (about 14 frames of an N64 match) the
    oldest are let go, with a line saying so.
  - **The diff's region must be one of the connector's** to be traced: `RAM` on the NES,
    `RDRAM` on the N64, with the address as the console has it.
- **`dj parity core`** loads the comparison core beside the baseline and prints how it
  follows it (`sync`), whether it follows `[WRITE]` lines itself, its `preroll`, its build,
  and each of its breakpoints in the original. A breakpoint in a region the baseline's
  connector doesn't have can't be set (dj leaves it out when it plays): it is marked, and
  the command exits 1. Run it after changing a core's breakpoint list.
- **`--json`** prints the segments and failures (or the report, the explanation, or the
  core's facts) as JSON.

The same checks are MCP tools for agents ([mcp.md](mcp.md#parity)). Not yet: several
segments at once (`--jobs`) and checking whole chains from power-on.

