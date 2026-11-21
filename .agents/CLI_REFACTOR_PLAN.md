# Plan: bring the CLI up to date

Written 2026-11-19, from a review the developer asked for: is `dj`'s command line clean,
and does it match the code after parity moved from one check per frame to events and
breakpoints ([yield-parity/PLAN.md](yield-parity/PLAN.md))? The GUI got most of the
attention during that work. The engine under `dj parity` is the new one
(`parity::Checker`, `check_span`), but the flags, the wording and the output still
describe the earlier model, and the commands that play have piled up.

dj is not released, so commands and flags change in place, with no aliases kept.

Status: all six steps are built (2026-11-19). What is left is at the end of step 6.

## What the review found

### Behind the events work

- **The breakpoint's name does not reach the text output.** yield-parity/PLAN.md,
  section 9, says the tools show where in the frame the verdict came. `dj parity`'s
  check lines, its failure list and the text `--report` print only the frame. The JSON of
  a check or re-run (and the `--events` lines the MCP parity jobs read) has no `at`,
  though `docs/cli.md` says it does. Only `--report --json` and `--explain` carry it.
- **"Synced" is the earlier model.** dj does not sync a core that takes events; a failure
  records the last jump before it. The CLI prints "(synced at frame N)" and its JSON has
  `synced`, `synced_segment` and `synced_frame`. The MCP tool descriptions say the same.
- **"Frame by frame".** The help and the docs say each frame is checked. That holds for a
  `sync = "state"` core. A core that takes events gives its verdicts at its own events.
- **Warnings from inside a frame are easy to lose.** A `[WRITE]` at an instruction that
  did not run, a write that could not be made, and a breakpoint switched off by the cap
  each print one warning in the middle of a batch. Nothing at the end says they happened.
- **Nothing inspects a comparison core.** `dj connector-check` checks the baseline's
  connector. No command loads a `[[compare]]` entry and lists its breakpoints, whether
  their regions exist, whether it handles writes, and its `preroll`.
- **Links to plans that were archived.** `docs/cli.md` links `PARITY_PLAN.md`;
  `database.md`, `connectors.md`, `coverage.md` and `decoders.md` link others; comments
  in `main.rs`, `parity_cmd.rs` and `hydrate.rs` name them too. They are in `_archive/`.

### General drift

- **Four commands that play:** `play`, `run`, `hydrate`, `clean`.
  - `run` and `play` judge and write hashes with their own `judge` and `accept`
    (`player/src/main.rs`), which repeat `hydrate::judge_hashes` and leave out
    `try_record`'s rule that the start must be trusted. So `run --accept` writes hashes
    that `clean --accept` would block.
  - `run` writes the `runs` and `run_results` tables. Nothing reads them.
  - `repin` calls `run` and edits its error message as text.
- **`dj parity` is five commands behind flags:** check, `--report`, `--rerun`, `--clear`,
  `--explain`. `--limit` belongs to one, `--frames-back` to another, and `main.rs` works
  out from the flags whether a core will run.
- **Flags that mean different things:**

  | Topic | Today |
  | --- | --- |
  | A segment's children | `clean SEGMENT` always includes them; `hydrate` and `parity` need `--children` |
  | "Play it again anyway" | `--all` on `clean`, `--force` on `hydrate`. `--all` on `parity` is "check again"; `--force` on `cache gc` is "ignore the GUI's lock" |
  | JSON | `report --json FILE` writes a file; everywhere else `--json` prints to stdout |
  | Screenshots | `play --screenshots DIR`; `run --screenshots` is a switch with a fixed folder |
  | The database | `play --no-cache` beside the global `--no-db` |
  | The segment argument | `play <id>`; `SEGMENT` elsewhere |

- **Flags that do nothing.** `report` and `annotations` take `--core` and `--no-db`;
  `import-tas` and `export-tas` take `--core`. The queries define a `--rom` of their own.
- **`report` beside `summary` and `gaps`.** Two code paths (`coverage::build_report`,
  `query::Project`) answer the same question.
- **Docs.** `dj annotations` is not in `docs/cli.md`'s table. `concepts.md`,
  `coverage.md` and `segments.md` still describe `unreachable.txt`, which
  `annotations.toml` replaced. `dj new --snapshot` and `--sram` (ROADMAP.md, "Root
  segments from a snapshot") are not built.
- **The CLI and the MCP tools name things differently:** `xref` and `where_runs`,
  `summary` and `coverage_summary`, `snapshot` and `snapshot_at`.

## Steps

### 1. Parity output catches up with events (built 2026-11-19)

No command or flag changes.

- **Where the verdict came** (`at`) is in the check lines, the failure list, the text
  `--report`, and the JSON of a check and a re-run.
- **"Jump" replaces "synced" in what is printed.** Text: "from the jump at frame N". JSON:
  `jump: {segment, frame}` in a check, a re-run and `--explain`; `jump_segment` and
  `jump_frame` in `--report --json` and the MCP `parity_failures` tool. The database
  columns and the `Failure.synced` field keep their names: a rename there touches the
  GUI and the stored tables for no gain to a user.
- **The help, the MCP descriptions and the docs** say "at each verdict" where they said
  "frame by frame", and "the last jump" where they said "synced".
- **A batch ends with a count** of writes not made and breakpoints switched off by the
  cap, on stderr, for `play`, `run`, `clean`, `hydrate` and `dj parity`. The exit status
  does not change: a dropped write is repeatable, and the developer set its handling
  aside (yield-parity/PLAN.md, section 3).
- `docs/cli.md`'s dead link to `PARITY_PLAN.md` goes.

The GUI's own "synced at" text followed in step 6.

### 2. `dj parity` becomes subcommands (built 2026-11-19)

`dj parity check | report | rerun | clear | explain | core`, each with only its own
flags (`player/src/parity_cmd.rs`).

- **`rerun [ID...]`, `clear ID...` and `explain ID`** take the ids as arguments.
- **`report` and `clear` load no core.** `clear` used to load the baseline only to reach
  the database. `report` takes `--rom`; `clear` takes neither.
- **`core` is new.** It loads the comparison core beside the baseline and prints its
  `sync`, whether it has `dj_handle_write`, its `preroll`, its build, and each breakpoint
  with whether the baseline's connector has its region. It exits 1 when one doesn't.
- **Changed with it:** the CI step, the MCP parity jobs and `parity_explain`
  (`player/src/mcp/jobs.rs`), `player/tests/compare.rs`, and the docs.

### 3. One way to play (built 2026-11-19)

- **`clean`** is the one command that writes hashes.
- **`hydrate`** is `clean --no-write`.
- **`run`** is `check`: plays every segment, compares, writes no file, with
  `--from-power-on` for what was `--from-scratch`.
- **`play`** plays one chain and saves screenshots. It lost `--accept`; `--no-cache` is
  `--from-power-on`.
- **`judge` uses `hydrate::judge_hashes`; `accept` is gone.** So are `Cache::begin_run`
  and `run_result`, and the schema drops the `runs` and `run_results` tables from a
  database that has them.
- **`repin`** calls the check directly, and names `dj clean --force --accept` as the way
  to record hashes that changed with the core.

Decided here, for the developer to overrule:

- **Writing hashes for the whole tree exists only as `dj clean --accept`.** `run --accept`
  and `play --accept` wrote without `try_record`'s rule that the start be trusted.
- **`check` still plays through `Session::play_all`, not `Hydrator`.** The plan said
  everything would go through `Hydrator`, but it needs the local database and cannot play
  a chain from power-on, and `check` must do both (`--no-db`, `--from-power-on`). What was
  wanted is there: one judge, and one writer.

### 4. One vocabulary for flags (built 2026-11-19)

- **`SEGMENT`** is the argument's name everywhere (`play` said `<id>`).
- **`--children` everywhere.** A command given a segment works on it alone. This changes
  `dj clean SEGMENT`, which included the segments below: that is now
  `dj clean SEGMENT --children`.
- **`--force`** is "play even what is stored" (`clean --all` before).
  **`parity check --again`** is "check even what this build checked" (`--all` before).
- **`--json`** always prints to stdout. The one that took a file went with `report`.
- **`--screenshots DIR`** on both `play` and `check`.
- **`--rom`, `--core` and `--no-db` only where they do something.** `Local` is now a plain
  struct, filled from `CoreArgs` (`--rom`, `--core`) or `RomArg` (`--rom`). `--no-db` is a
  flag of `check`, `play` and `snapshot` alone; `clean` and `parity` always needed the
  database and refused it. `report`, `annotations` and the TAS commands lost `--core`.

Left as it was: `cache gc --force` ("run while dj-gui has the project open").

### 5. `report` folds into the queries (built 2026-11-19)

- **`dj summary`** says what `report`'s header said: the segments left out (dirty, not
  played, unsaved), what the unsaved and the dirty ones would add, opcode bytes per
  region, and the smallest set (in `--json`, and so in the MCP `coverage_summary`).
- **`dj summary --segments`** prints `report`'s table, a row per segment, and the
  smallest set. In `--json` it adds `per_segment`. It is a flag so that the MCP tool's
  answer stays short.
- **`dj gaps --csv FILE`** replaces `report --gaps`.
- **`report` and `coverage::print_report` are gone.** `report --json FILE`, the whole
  report as one file, has no replacement: `summary`, `gaps` and `function` with `--json`
  give its parts.

### 6. Docs, names and tests (built 2026-11-19)

- **`docs/cli.md`** has its table and its parity section written from the new commands,
  with `annotations` added. The other docs, the examples' READMEs (commands only), the CI
  workflow and the MCP descriptions follow.
- **Links to archived plans** point at `.agents/_archive/`: in the docs, in comments in
  the code and the tests, and in `ROADMAP.md`, `RELEASE_PLAN.md` and `yield-parity/`.
- **`unreachable.txt` is out of the docs.** `concepts.md`, `coverage.md` and `segments.md`
  describe `annotations.toml`. `config.md` still says what it replaced, and the code still
  stops on a project that has the old file.
- **MCP tools take the CLI's names:** `where_runs` is `xref`, `snapshot_at` is `snapshot`,
  and `parity_check`'s `all` is `again`. `coverage_summary` keeps its name: beside
  `parity_summary`, a bare `summary` would not say which.
- **The GUI says "the jump"** where its failure list and its stopped message said "synced
  at", and the failure's hover text gives where the verdict came.
- **Tests.** `player/tests/compare.rs`: the `at` text and the `jump` keys, `parity core`,
  and that `parity report` refuses `--core`. `player/tests/batch.rs`
  (`clean_check_and_the_queries`): `clean --no-write`, `clean SEGMENT` with and without
  `--children`, `--force`, `check --from-power-on --no-db`, `summary --segments`,
  `gaps --csv` without a symbol map, and the count of writes not made.

### After the review (2026-11-19)

A critical pass after the commit, and the developer's answers to it.

- **One set of exit statuses.** Every command: 0 all is well, 1 it ran and found
  something wrong, 2 an error. Before, `play`, `check`, `clean`, `repin`, `annotations`
  and the connector check gave 2 for what they found, the same as for an error, so CI
  couldn't tell them apart. `docs/cli.md` says so.
- **The smallest set is only in `summary --segments`**, so the MCP `coverage_summary`
  answer doesn't grow with the project.
- **`dj check --strict`** fails on a segment with no recorded hashes ("new"), which
  otherwise passes with a note. CI uses it.
- **`check-connector` is `connector-check`**, so that it doesn't read as a sibling of
  `dj check`. (The text above says `connector-check` throughout; it was `check-connector`
  when the review was written.)
- **`Cache::open_readonly` is `Cache::open_existing`.** It was never read only: it brings
  the schema up to date, and `parity clear` deletes through it.
- **MCP tools `parity_clear` and `parity_core`**, as the commands. `parity_core` runs
  `dj parity core --json` in a child, through the runner `parity_explain` now shares
  (`run_dj_json` in `player/src/mcp/jobs.rs`).
- **The comparison worker is dead code**, kept alive by its own test. Not removed: it is
  the first case in ROADMAP.md's "Remove dead code before the release", and in
  RELEASE_PLAN.md's clean-up table.
- **`scripts/mcp_smoke.py`** asks `frontier` about the whole ROM. It passes end to end.
- Left as it is (the developer): `dj parity core` doesn't run in a watched child, so a core
  that crashes while loading takes the command down. The MCP tool runs it in a child.

Not done, and not planned here:

- **`gaps --csv` with a symbol map has no test** (the test core has no map). It was run by
  hand on the Thwaite example.
- **The CI workflow has not run** with the new commands.

## Second pass (2026-11-20)

The developer chose two of three simplifications; the third, a global `-C DIR` in place of
every command's `<dir>`, was left out.

- **`dj play` is folded into `dj check`,** which takes the same choice of segments as
  `dj parity check` and `dj cache export` (`SEGMENT...`, `--tag`, `--children`). A chosen
  segment plays from the nearest stored state above it (`Session::play_chain`), or with
  `--from-power-on` from power-on; only the chosen ones are reported and in the JUnit
  report. `--shot` needs `--screenshots`. With no choice it plays the whole tree as before.
  A single segment now runs in a watched child too, so a crash is a failure (exit 1).
- **`dj clean --no-write` is `dj cache fill`,** so `clean` always writes hashes.
- **The project folder is `-C DIR`, not the first argument** (the developer, after the
  first two). dj finds the project from the current folder up: the nearest folder whose
  `config.toml` has `[rom]` and `[core]` tables (so another tool's config.toml isn't taken),
  or an older project's `collection.toml`. `-C` is global, and changes only which project:
  other relative paths stay relative to the current folder. `dj init DIR` keeps its
  argument. The old form (`dj check examples/thwaite`) is refused with a hint, exit 2. The
  MCP jobs, the MCP worker, the tests, `scripts/mcp_smoke.py`, CI and the docs pass `-C`.
