# The CLI

`dj` is the headless player. It plays segment trees on any libretro core, checks state
hashes, fills the local database and reports coverage. Building it is in
[install.md](install.md).

| Command | What it does | More |
| --- | --- | --- |
| `dj init <dir> --name NAME --rom ROM --core CORE [--option KEY=VALUE]... [--symbols FILE]` | Creates a project and pins the setup in `config.toml` | [install.md](install.md#a-first-project), [config.md](config.md) |
| `dj new <dir> [--parent ID] [--title TEXT] [--frames N]` | Adds a segment and prints its path | [editing.md](editing.md) |
| `dj status <dir>` | The tree with clean and dirty marks | [playback.md](playback.md#clean-and-dirty) |
| `dj repin <dir>` | After rebuilding or updating the core: plays every segment from power-on and, if all still match, records the core's build in `[core] build` | [config.md](config.md#core) |
| `dj upgrade <dir>` | Rewrites every segment file in this dj's format, and records this dj in config.toml | [segments.md](segments.md#older-projects) |
| `dj play <dir> <id> [--screenshots DIR] [--shot 60,120] [--no-cache] [--accept]` | Plays one segment and its chain | [playback.md](playback.md#checking-hashes) |
| `dj run <dir> [--from-scratch] [--accept] [--screenshots]` | Plays everything, checks hashes, records coverage | [playback.md](playback.md#checking-hashes) |
| `dj hydrate <dir> [SEGMENT [--children]] [--force]` | Fills in what the local database is missing, without writing files | [database.md](database.md#from-the-command-line) |
| `dj clean <dir> [SEGMENT] [--accept] [--all]` | Plays the segments that aren't clean or aren't in the local database (or SEGMENT and those below it; `--all`: every one), top down, and writes the hashes they reach into their files. Changed endings are listed and not written (the command fails) unless `--accept` | [playback.md](playback.md#recording-hashes) |
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
