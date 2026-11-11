# The CLI

`dj` is the headless player. It plays segment trees on any libretro core, checks state
hashes, fills the local database and reports coverage. Building it is in
[install.md](install.md).

| Command | What it does | More |
| --- | --- | --- |
| `dj init <dir> --name NAME --rom ROM --core CORE [--option KEY=VALUE]... [--symbols FILE]` | Creates a project and pins the setup in `config.toml` | [install.md](install.md#a-first-project), [config.md](config.md) |
| `dj new <dir> [--parent ID] [--title TEXT] [--frames N]` | Adds a segment and prints its path | [editing.md](editing.md) |
| `dj import-tas <dir> FILE [--title TEXT] [--strict]` | Imports a BizHawk TAS movie (`.bk2`) as a new root segment and prints its id | [tas.md](tas.md#importing) |
| `dj export-tas <dir> SEGMENT --out FILE [--through FRAME] [--author NAME] [--strict]` | Writes the chain from the root down to SEGMENT (through FRAME of it) as a BizHawk `.bk2` movie, leaving out what it can't carry, with a warning (`--strict`: refuse) | [tas.md](tas.md#exporting) |
| `dj status <dir>` | The tree with clean and dirty marks | [playback.md](playback.md#clean-and-dirty) |
| `dj repin <dir>` | After rebuilding or updating the core: plays every segment from power-on and, if all still match, records the core's build in `[core] build` | [config.md](config.md#core) |
| `dj upgrade <dir>` | Rewrites every segment file in this dj's format, and records this dj in config.toml | [segments.md](segments.md#older-projects) |
| `dj play <dir> <id> [--screenshots DIR] [--shot 60,120] [--no-cache] [--accept]` | Plays one segment and its chain | [playback.md](playback.md#checking-hashes) |
| `dj run <dir> [--from-scratch] [--accept] [--screenshots]` | Plays everything, checks hashes, records coverage | [playback.md](playback.md#checking-hashes) |
| `dj hydrate <dir> [SEGMENT [--children]] [--force]` | Fills in what the local database is missing, without writing files | [database.md](database.md#from-the-command-line) |
| `dj clean <dir> [SEGMENT] [--accept] [--all]` | Plays the segments that aren't clean or aren't in the local database (or SEGMENT and those below it; `--all`: every one), top down, and writes the hashes they reach into their files. Changed endings are listed and not written (the command fails) unless `--accept` | [playback.md](playback.md#recording-hashes) |
| `dj parity <dir> [SEGMENT [--children]] [--compare NAME] [--all] [--dirty] [--json]` | Checks segments against a comparison core frame by frame, each from its own start; exits 1 when a frame fails. `--report`, `--rerun [ID...]` and `--clear ID...` work with the results kept | [below](#parity) |
| `dj report <dir> [--json FILE] [--gaps FILE.csv]` | Coverage of the current, clean, saved segments (drafts are listed apart) | [coverage.md](coverage.md#from-the-command-line) |
| `dj xref <dir> ADDR` (alias `where`) | Which segments first run that code, at which frame, and the nearest snapshot | [coverage.md](coverage.md#from-the-command-line) |
| `dj summary <dir>` | Instructions, blocks and functions covered, and the segments that add most | [coverage.md](coverage.md#from-the-command-line) |
| `dj gaps <dir> [--sort size\|nearness] [--only all\|never\|partial]` | Functions with uncovered code, largest or nearest first | [coverage.md](coverage.md#from-the-command-line) |
| `dj function <dir> ADDR` | One function's blocks (covered or not, first run, where they go), callers and the segments that enter it | [coverage.md](coverage.md#from-the-command-line) |
| `dj frontier <dir> [ADDR]` | Covered branches and calls that lead into uncovered code, with the segments that reach them | [coverage.md](coverage.md#from-the-command-line) |
| `dj check-connector <dir> [--json]` | Developer tool: checks the coverage connector's flags in the local database against the decoder, inside the symbol maps' functions. Lists opcode flags where the decoder starts no instruction, operand flags that don't belong to an instruction that ran, operand bytes missing their flag, and opcodes run in both widths of a mode pair; fails when any disagree. For writing or updating a connector, not for normal use | [connectors.md](connectors.md#writing-one-for-another-core) |
| `dj mcp <dir> [--allow query\|draft\|save]` | Serves the project to an AI agent over MCP (stdio) | [mcp.md](mcp.md) |
| `dj snapshot <dir> SEGMENT FRAME --out FILE` | Writes the core state at that frame, to start a trace from | [coverage.md](coverage.md#from-the-command-line) |
| `dj sql <dir> "SELECT ..."` | Queries the local database | [database.md](database.md) |
| `dj cache stats <dir>` / `dj cache gc <dir> [--force]` | Database size, or drop what no segment uses (not while dj-gui has the project open, unless `--force`) | [database.md](database.md#from-the-command-line) |

Common to every command:

- Ids can be shortened to any unique prefix.
- `--rom` and `--core` are remembered per project in `.local/local.toml`, so later commands don't need them.
- Every command that plays takes `--no-db` to run without the local database.
- `dj run`, `dj clean` and `dj hydrate` play in a child `dj` process that the command
  watches, so a core that crashes or hangs can't stop the batch. A crash, or a frame that
  doesn't finish within 30 s (a minute while the core loads), ends the child; the command
  prints `crashed  <id> <title> at frame N: the core crashed (signal 11, SIGSEGV)` (or "made
  no progress"), then runs again without that segment and the ones below it (shown as
  `blocked`), and ends with a list of what was left out and exit code 1. `clean` and
  `hydrate` keep what they stored before the crash; `run` plays from the start again.
  `DJ_WORKER_TIMEOUT_SCALE` multiplies the time limits (say for a very slow core);
  `DJ_IN_PROCESS=1` plays in `dj`'s own process, with no watching (for a debugger).
- The queries (`xref`, `summary`, `gaps`, `function`, `frontier`) take `--json` for scripts, `--limit` and `--cursor` for paging, and an address as `PRG:0x1F3A`, `0x1F3A` (a PRG offset), `$C123` (a CPU address) or a symbol, optionally `name+0x10`. They read the local database, so clean first (`dj clean`).

## Parity

`dj parity` does what **Check parity** does in dj-gui ([gui.md](gui.md#parity)) without
anyone watching, so a decomp's CI can run it after building its comparison core:

```
dj parity <dir> --compare "decomp"        # every clean segment
dj parity <dir> SEGMENT --children        # one branch
dj parity <dir> --report                  # what the results say, per function; no core runs
dj parity <dir> --rerun                   # failures from an older build, played again
dj parity <dir> --clear 12 13             # forget failures by id
dj parity <dir> --explain 12              # what differed on failure 12, and who wrote it
```

- **Which comparison core:** `--compare NAME` picks a `[[compare]]` entry; it can be left
  out when there is only one.
- **Each segment on its own.** The comparison core is synced at the segment's frame 0 (from
  the stored end state of its parent) and every frame is checked to the end, or to the first
  failure, which ends that segment's check; the others go on. Segments that aren't clean are
  left out unless `--dirty` (clean them first: `dj clean`).
- **Done once per build.** A segment whose content was checked with this build of the
  comparison core (the sha256 of its library) is skipped, with how it went then; `--all`
  checks it again. A rebuilt core is a new build, so everything is checked again.
- **Results** go into the local database the GUI reads: the code frames that passed ran,
  and each failure with the segment and frame, where the comparison core was synced, and the
  reason. A failure found again replaces the earlier one.
- **`--report`** counts the instructions that ran by parity (with parity, failed, failed
  with an older build, not checked), per function with a symbol map, and lists the
  failures with their ids, newest first.
- **`--rerun`** plays failures again the way they were found, with the build there is
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
  bytes that differ. `--report` shows the first few under each failure.
- **`--explain ID`** plays a failure again as `--rerun` does, but keeps no results: it
  traces the original (the baseline core) over the failing frame and the `--frames-back N`
  before it (10 by default), and for each difference with an address gives the original's
  last instruction that wrote it, with its function. That is the code to compare with the
  decomp's. It also says whether the frame still fails and which functions it ran. Tracing
  needs the trace hook in the baseline's connector (Mesen's has it). Exit status 0 when it
  passes now, 1 when it still fails.
- **`--json`** prints the segments and failures (or the report, or the explanation) as JSON.

The same checks are MCP tools for agents ([mcp.md](mcp.md#parity)). Not yet: several
segments at once (`--jobs`) and checking whole chains from power-on
([PARITY_PLAN.md](../.agents/PARITY_PLAN.md)).

