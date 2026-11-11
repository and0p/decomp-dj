# Plan: parity from the CLI and MCP (`dj parity`)

Status: written 2026-11-10. All eight steps are done (2026-11-10/11), apart from `--jobs`,
`--from-power-on` and resolving data symbols; what's left is in the ROADMAP ("Still to do
for parity"). Parity itself shipped in
the GUI (ROADMAP, "Comparison and parity"; [gui.md](../docs/gui.md#parity),
[connectors.md](../docs/connectors.md#parity)); this plan is its "Still to do: the CLI and
MCP".

## Goal

Check a decomp against the original game without anyone watching: `dj parity` in a
terminal or in CI, and MCP tools so an agent can run checks, read the results and find out
why a frame failed. Results go into the same local database tables the GUI reads, so a
check run from the command line colours the GUI's map, and the other way round.

What it should answer:

- **Does the decomp still match?** For every segment (or one branch), play both cores and
  check each frame. Exit non-zero on a failure, so it can gate a CI job.
- **Where does it not match yet?** Per function: instructions with parity, failed with this
  build, failed with an older build, and run but never checked (the GUI's four states).
- **Why did this frame fail?** The connector's reason, the bytes that differ, and which
  baseline code wrote them on that frame.

## What exists now

- `player/src/compare.rs`: `Comparison` (in process) and `RemoteComparison` (the
  comparison core in a worker process, `CompareWorker`), with `sync`, `step`, `check`
  (`dj_check_parity` or the RAM fallback) and `Verdict::{Match, Mismatch, Skip}`.
- `player/src/parity.rs`: the database side. `parity_passed` (one set of PRG flags per
  core and build, OR'd over every passing frame) and `parity_failures` (core, build, where
  it was synced, segment, frame, reason, that frame's flags).
- `gui/src/main.rs` `run_one`: **the per-frame loop lives in the GUI**. It syncs after a
  jump, steps both cores, calls `check`, takes the frame's own PRG flags
  (`gui/src/cov.rs` `take_prg_flags`), and adds a pass or a failure.
  `gui/src/parity.rs` keeps the results in memory, works out the colours, and runs Re-run.
- `player/tests/compare.rs`: a test core with `testcore_break = "mismatch"` and
  `testcore_sync = "fail"` options, good for testing failures without a decomp.
- MCP (`player/src/mcp/`): query tools read the database, sessions and jobs drive
  `dj worker` processes, `fuzz` and `seek_goal` are background jobs with `job_status`.

Two things the CLI can't build on as they are:

1. **The loop is GUI code.** It is mixed with the picture, the playback state and the
   GUI's snapshots.
2. **The GUI overwrites passes.** `Parity::save` writes its whole in-memory flags with
   `INSERT OR REPLACE` every 3 s. A `dj parity` run while the GUI has the project open
   would be wiped out by the GUI's next save, and the reverse.

## Design

### One engine in the library

A `parity::Checker` in `player/src/parity.rs` (or a new `parity_run.rs`) owns one check
run:

- **Holds:** the baseline `Session` and a `RemoteComparison`. Use the worker so a
  crashing decomp can't take `dj` down; that's the same reason the GUI uses it.
- **`start(segment, frame)`:** syncs the comparison core there and records the sync point.
- **`step()`:** runs one frame of both cores, checks it, and takes the frame's flags. It
  returns `Passed(flags)`, `Failed(Failure)`, `Skipped(reason)` or `Unsynced` (a
  `[WRITE]` the core couldn't follow: sync again before the next checked frame).
- **Stops at a failure:** the run ends there and the next `start` syncs again. This is the
  rule already decided for the GUI: no resync, no carrying on.

`take_prg_flags` moves from `gui/src/cov.rs` into the library. The GUI's `run_one` then
calls the `Checker` for its comparison step, so both use the same code. Do that in the
same change, or the two will drift.

### Batch runs: where each check starts

A frame that was checked once counts, however it was reached (the GUI's rule). So a batch
run doesn't need to play from power-on:

- **Each segment is checked on its own:** sync at its frame 0, from the stored end state
  of its parent, or its root start. Then play to its end or to the first failure.
- **Segments are independent**, so they can run in parallel: `--jobs N`, each job a
  baseline plus a comparison worker.
- **A failure ends that segment's run only.** The command goes on with the others, then
  reports them all and fails.
- **`--from-power-on`** syncs once at the root and plays the whole chain, for decomps
  whose sync can't start mid-game. Its failures then stop the rest of that chain.

A segment's start state comes from the local database, so `dj parity` hydrates what's
missing first, as `dj clean` does, and refuses dirty segments (like `dj report`, which
only counts clean ones) unless `--dirty`.

### Not redoing work: a runs table

Passes are kept as one OR'd flag set per core and build, so the database can't say which
frames were checked. Add a table:

```sql
parity_runs(core, build, content TEXT,      -- the segment's content key
            frames_checked, frames_skipped,
            failed_at INTEGER NULL,         -- frame of the failure, if any
            created, PRIMARY KEY(core, build, content))
```

- **Skipping:** a segment already checked with this build and unchanged content is
  skipped unless `--all`.
- **Counts:** this is also where "passed on X frames" counts would start (ROADMAP: counts
  per block). Keep those out of this plan.
- **Schema:** bump the cache schema the usual way; the table is cheap to drop and refill.

### Passes merge instead of replacing

- **`set_passed` becomes `add_passed`:** read, OR, then write, in one transaction.
- **GUI:** keeps a "new since last save" set and adds it, rather than writing its whole
  copy.
- **Reloading:** the GUI picks up passes and failures written by another process when its
  `.local/` fingerprint changes (or the file watcher, once there is one), as it does for
  drafts.

### The CLI

```
dj parity <dir> [SEGMENT [--children]] [--core NAME] [--all] [--dirty]
                [--from-power-on] [--jobs N] [--json]
dj parity <dir> --report [--core NAME] [--json]       # no cores run
dj parity <dir> --rerun [ID...] [--core NAME]         # old failures (the GUI's Re-run)
dj parity <dir> --clear ID...
```

- **`--core`:** defaults to the only `[[compare]]` entry; with several, it's required.
- **Run output:** one line per segment (`checked 600, skipped 12` or
  `FAILED at frame 233: <reason>`), then the report.
- **Report:** instructions and functions with parity, failed now, failed before, and
  untested, plus the failures with how to reproduce them.
- **`--json`:** for scripts, like the query commands.
- **Exit status:** 0 when nothing failed, 1 on a parity failure, 2 on an error (a core
  that won't load, a failed sync). That makes "Automate regression testing" in the README
  true: a decomp's CI builds its comparison core and runs `dj parity`.
- **Supervised:** like the other batch commands (`isolate::supervise`), so a crash is
  reported with the segment and frame.

### MCP

**Query tools (`--allow query`):** they read the database and run no cores.

| Tool | Returns |
| --- | --- |
| `parity_summary(core?)` | The four counts, overall and for the functions with most failed or untested code |
| `parity_failures(core?, addr?, old?)` | Failures (optionally only those whose frame ran `addr`), with segment, frame, sync point, build, reason and whether the build is current |

`function(addr)` and `gaps` gain each block's parity state when a comparison core has
results, so an agent sees parity where it already looks at coverage.

**Running checks (`--allow draft`):**

| Tool | Does |
| --- | --- |
| `parity_check(segment?, children?, core?, all?)` | A background job like `fuzz`: returns a job id; `job_status` gives segments done, frames checked, failures so far |
| `parity_rerun(ids?)` | Re-runs old failures (all of them by default) as a job; a pass clears the failure |
| `parity_explain(failure)` | Replays to the failing frame and says why it failed (below) |

**Why a frame failed (`parity_explain`).** This is the "exactly when and why" from the
README:

1. **Replay:** sync at the failure's recorded sync point and play to the frame before the
   failure, then run the failing frame.
2. **Diff:** return the connector's reason and, when both cores expose system RAM with the
   same layout, the differing ranges (address, baseline bytes, comparison bytes, capped).
3. **Trace:** trace the baseline's failing frame with `TRACE_WRITE` (the `trace`
   tool's hook). For each differing address, give the last baseline instruction that wrote
   it: function, address, value. That points at the code the decomp got wrong, or at
   least the first place to look.
4. **Context:** the failing frame's own coverage (the failure's `flags`), as functions
   that ran.

Tracing the comparison core is out of scope: its connector's trace hooks aren't used, and
a decomp often has no such hook.

### What differed, from a decomp's own check

The RAM diff above only works when the decomp keeps the original's memory layout. A port
with its own `dj_check_parity` returns a verdict and a reason string, so dj can't tell what
was compared. The decomp is the only side that knows how its state maps to the
original's, so it reports the differences in the original's terms, through a new optional
connector function:

```c
// After dj_check_parity said mismatch: every difference found, in the original game's
// terms. Fills up to `cap` entries and returns how many there are in all (dj keeps the first
// 100 and counts the rest). Pointers stay valid until the
// next dj_check_parity call.
size_t dj_parity_diffs(struct dj_parity_diff *out, size_t cap);

struct dj_parity_diff {
    const char    *label;     // optional plain-text name: "Player 2 x coordinate"
    const char    *region;    // the original's region name: "RAM", "SRAM", ...
    uint64_t       address;   // in that region (ignored when `symbol` is set)
    const char    *symbol;    // optional: a data symbol in the original's symbol map
    const uint8_t *expected;  // the original's bytes
    const uint8_t *actual;    // the decomp's bytes
    size_t         len;
};
```

- **All differences, not the first:** one frame often gets several values wrong. They
  usually share a cause, and seeing them together helps find it.
- **`label`:** what the value means, from the port's own names. A diff with only a label
  and no address (a value the original keeps in no one place) is allowed: it's shown,
  but nothing can be traced for it.
- **`symbol`:** resolved to an address through the original's symbol map.
- **The RAM fallback:** fills the same list from the byte compare: one entry per run of
  differing bytes, labelled from the symbol map where a data symbol covers it.
- **Stored with the failure:** a `parity_diffs` table (failure id, label, region,
  address, symbol, expected, actual), so `parity_failures`, the GUI and `--report` show
  them without replaying. Kept to a cap per failure (100?) with the total count.

`parity_explain` then traces each diff that has an address (above): the last instruction
in the original that wrote it on the failing frame, or in an earlier frame back to the
sync point (a value can go wrong before the check notices). If the original wrote it and
the decomp didn't, the original's writer is still the function to match.

When no diff can be traced (a check that compares the picture), the fallbacks are:

1. **Suspects by frame:** code that ran on the failing frame and on no passing one, from
   the coverage dj already keeps.
2. **Comparing calls:** the decomp exports a trace of function calls with its own names;
   dj matches them to the original's symbols and compares the two call sequences,
   arguments and return values, stopping at the first difference. This is the ROADMAP's
   open "Tracing" option and needs a trace hook in the decomp, so it comes later.

**In sessions, later.** An `open_session(..., compare=NAME)` option where every `step`
also reports the frame's verdict. Useful for an agent poking at a fix, but it means a
comparison worker per session. Leave it until the tools above are used.

### Building the comparison core

`dj parity` loads whatever `[[compare]] path` names (with `sha256` pinning as today). It
doesn't build the decomp. The docs should show a CI job: build the decomp's libretro core,
then `dj parity`.

## Steps

1. **Library engine.** Done: `parity::Checker` (sync, step, failure records),
   `parity::check_span` (sync, check frames, stop at the first failure), and
   `parity::take_code_flags` (moved from the GUI); dj-gui's `run_one` and `compare_sync` use
   the `Checker`. Tested in `player/tests/compare.rs` (`checking_spans`). As planned:
   - Move the per-frame comparison logic and `take_prg_flags` into the player crate as
     `parity::Checker`.
   - Switch the GUI's `run_one` to it.
   - Tests in `player/tests/compare.rs` with the test core: a run that passes, one that
     fails at a known frame (`testcore_break`), and a failed sync.
2. **Merge passes.** Done: `parity::add_passed` ORs in a transaction (`set_passed` is
   gone); dj-gui adds only what passed since its last save, and reloads parity results when
   the database's `PRAGMA data_version` changes (another connection committed), checked
   once a second. As planned: `add_passed` with OR in a transaction, the GUI saving only what's
   new, and the GUI reloading parity when another process wrote it.
3. **`parity_runs` table.** Done: added to the schema (`CREATE TABLE IF NOT EXISTS`, so no
   version bump was needed), with `run_of` and `record_run`; failures found again replace
   the earlier record (`replace_failure`). As planned: the table and the cache schema bump.
4. **`dj parity`:** Done in `player/src/parity_cmd.rs` ([cli.md](../docs/cli.md#parity)):
   checks per segment, skips what this build checked, `--report` (no core; counts per
   function, or per opcode byte without symbols; failures alone without coverage),
   `--rerun`, `--clear`, `--json`, exit codes, supervised, a crashed comparison core
   reloaded for the next segment. `--compare NAME` instead of `--core`, which the CLI
   already uses for the baseline. Tested end to end (`dj_parity_command` in
   `player/tests/compare.rs`) and on Thwaite (12 segments, 11,140 frames in 36 s with Mesen
   against itself). **Not done:** `--jobs` (the baseline core is one per process, so
   parallel checks need a child process per job) and `--from-power-on`. As planned: run, report, rerun, clear, `--json`, exit codes, `--jobs`; supervised.
5. **MCP query tools:** Done ([mcp.md](../docs/mcp.md#parity)): `parity_summary` and
   `parity_failures` on `query::Project` (`parity_report`, `parity_failures`, shared with
   `dj parity --report`), each block's state in `function` and each function's counts in
   `gaps`, per comparison core with results. As planned: `parity_summary`, `parity_failures`, and parity in `function` and
   `gaps`.
6. **MCP jobs:** Done: `parity_check` and `parity_rerun` run `dj parity --events` (a hidden
   flag: a JSON line per segment) as a child process and report through `job_status`. Not
   on the fuzz jobs' worker framework as planned: the CLI already isolates both cores and
   keeps the results, so the job only reads its lines. Tested in `mcp_parity_tools`
   (`player/tests/compare.rs`) and tried on Thwaite. As planned: `parity_check` and `parity_rerun` on the jobs framework.
7. **Diffs and `parity_explain`:** Done ([connectors.md](../docs/connectors.md#parity),
   [cli.md](../docs/cli.md#parity), [mcp.md](../docs/mcp.md#parity)):
   - `dj_parity_diffs` in the connector API and the test core (a labelled byte diff and a
     label-only one). dj asks once with room for 100 and keeps the total; it doesn't call
     again for the rest.
   - The RAM fallback lists one diff per run of differing bytes (up to 64 bytes of each).
   - `parity_diffs` table, loaded with the failures; shown in dj-gui's failure message and
     hover, `dj parity` output, `--report` and `parity_failures` (first 10).
   - `dj parity --explain ID [--frames-back N]` and the MCP `parity_explain` (a waiting
     tool, running the CLI as a child): replays as `--rerun` does without keeping results,
     traces the baseline over the last N+1 frames, and gives each addressed diff the last
     write's instruction and function. Tried on Thwaite with a hand-made failure: a
     zero-page diff traced to `updateVillagers+0x10`.
   - **Not done:** `symbol` isn't resolved (dj loads no data symbols yet), so the RAM
     fallback's diffs aren't labelled from the symbol map; the fallbacks for untraceable
     diffs ("suspects by frame", comparing call traces); per-byte writers for a long diff
     (the last write to any of its bytes is given). Not tested with a real failing decomp:
     Mesen won't load a save state into a patched ROM, so no real mismatch was at hand.
8. **Docs.** Done:
   - `cli.md`: the command, `--explain` and what differed.
   - `mcp.md`: the tools, and explaining.
   - `gui.md`: results from the CLI and from agents' jobs appear in the GUI, and what
     differed shows in the failure message.
   - `connectors.md`: `dj_parity_diffs`, and "Checking parity in CI" (a GitHub Actions
     sketch: dj from a release ZIP, Mesen from `connectors/`, the decomp's core, the ROM from
     a secret, `dj parity`, and the report and explanations as artifacts on failure).
   - `database.md`: `parity_runs` and `parity_diffs`.
   - README: the regression-testing bullet (side by side, frame by frame, in the GUI or CI)
     and the agents' bullet (the frame, what differed, and the original's instruction that
     wrote it).
   - ROADMAP: "Shipped: parity without the GUI (phase 3)" and "Still to do for parity".

Steps 1 and 2 are worth doing even if the rest waits: they fix the GUI-overwrites-passes
problem and make the loop testable.

## Open

- **Default start:** each segment from its own start (fast, parallel) or from power-on
  (closer to how a player reaches a frame). The plan picks per segment, with
  `--from-power-on` for decomps that need it.
- **Dirty segments:** whether to check them at all (their coverage isn't counted in
  `dj report`).
- **Comparison core without `dj_sync`:** syncing by save state only works for the same
  emulator. A decomp that can only boot (`DJ_SYNC_BOOT`) forces `--from-power-on`; should
  `dj parity` choose that by itself?
- **How much of the RAM diff to return** before it floods an agent's context; a cap plus a
  count, probably.
- **More than one comparison core** in a single run (the ROADMAP's later item). `--core`
  could take several; each needs its own worker.
