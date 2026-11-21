# Plan: gating pull requests on parity

Written 2026-11-19, from a conversation with the developer: a game port's repository should
be able to block a pull request when the port loses parity with the original, on every
segment or on a chosen few, on any CI system. Follows [CLI_REFACTOR_PLAN.md](CLI_REFACTOR_PLAN.md).

Status: steps 1 to 6 built (2026-11-19). Open questions at the end.

## Decisions

- **No CI-specific output.** dj writes JUnit XML, which GitLab, Jenkins, Azure DevOps,
  CircleCI and Buildkite read as test results, and GitHub through a reporter action
  (`dorny/test-reporter`, `mikepenz/action-junit-report`), which also adds the job summary
  and annotations in the pull request. No `::error` lines. (The developer, 2026-11-19.)
- **JUnit is opt-in;** coloured text is the default. A report is a file, and a file
  written unasked lands in the checkout. (The developer, 2026-11-19.)
- **The pipeline fetches the ROM, not dj.** A secret holds a URI to it; a step downloads it
  to a temporary path and hands dj the path. `[rom] sha256` in config.toml refuses any
  other file. dj gets no HTTP client and never sees a credential. Snapshots go the same
  way. (The developer, 2026-11-19.)

## Steps

### 1. `DJ_ROM` and `DJ_CORE`

Every command that takes `--rom` or `--core` reads these environment variables when the
flag is left out, before `.local/local.toml`. A path from the environment is not saved
into `.local/local.toml`, unlike one given as a flag: a developer with the variables set
keeps their own settings. The MCP server's sessions read them too.

### 2. Choosing the segments

`dj parity check <dir> [SEGMENT...] [--tag NAME]... [--children]`:

- **Several segments,** each an id or a unique prefix.
- **`--tag NAME`,** repeatable: the segments whose `tags:` header has any of them
  (docs/segments.md). A project tags its pull-request gate, `tags: ci`.
- **`--children`** adds the segments below each one chosen, by id or by tag.
- **None of either** checks every clean segment, as now.
- The MCP `parity_check` tool takes `segments` and `tags` the same way.
- `dj cache export` (step 4) takes the same choice.

### 3. Output

- **Colour by default** on the lines that give a verdict: red for a failure, green for a
  pass, yellow for what was left out. On when the output is a terminal or `CI` is set; off
  with `NO_COLOR`, forced with `CLICOLOR_FORCE`. Never in JSON, `--events` or the MCP
  server. `warning:` and `failed:` on stderr likewise.
- **`--junit FILE`** on `dj check` and `dj parity check`, or `DJ_JUNIT=DIR` to give every
  command one folder (`dj-check.xml`, `dj-parity-<compare>.xml`).
  - One `<testsuite>` per command run (per comparison core for parity), one `<testcase>`
    per segment: its short id and title, with `file` the segment file (relative to the
    working directory) and its time.
  - A failure is a `<failure>`: its message is the first line of what the text output
    says; its body has the frame, where in the frame the verdict came, the last jump, the
    reason and the first diffs. `line` points at the segment's line for the failing frame
    where there is one.
  - A segment that crashed or hung the core is a failure. One not checked (dirty, or
    checked with this build before) is `<skipped>` with the reason; one that failed with
    this build before is a failure, as the exit status already counts it.
  - Written in the batch child that finishes the run, so segments left out after a crash
    are in it.

### 4. Snapshots for CI (built as `dj cache export` and `dj cache seed`)

Without a local database each checked segment plays everything above it from power-on.
Seeding gives CI the states it starts from, checked by hash.

- **`dj cache export <dir> [SEGMENT...] [--tag NAME]... [--children] [--compare NAME] --out DIR`**
  (planned as `dj snapshot export`; it went under `dj cache` because `dj snapshot` takes
  `SEGMENT FRAME` as positional arguments, and both halves are about the database's stored
  states. Without `--compare` it allows for the largest `preroll` of all the entries)
  writes the states that checking those segments starts from: each one's start (its
  parent's end), and with a `preroll` in the `[[compare]]` entry, the start of the segment
  the pre-roll begins in. One file per state, named `<state hash>.state`. A root segment
  that starts at power-on needs none. A segment that isn't clean is refused: its start
  isn't known.
- **`dj cache seed <dir> FILE|DIR...`** loads each state into the core, works out its
  state hash, and stores it in the local database only when that hash is a clean segment's
  recorded end hash (what a child's start is loaded by). Anything else is refused and named (another core build, an older
  project, the wrong file), and the command exits 1. Stored states are what `dj parity
  check`, `dj clean` and the rest already look for, so nothing else changes.
- **States from another machine** load only if the core's save states carry across builds
  and systems. The hash check refuses those that don't; whether Mesen's do between macOS
  and Linux is to be tried.
- A state holds the game's RAM (on the N64, code too): keep exported states where the ROM
  is kept, never in the repository or a cache a pull request can restore.

### 5. Docs

- `docs/ci.md`: a GitHub Actions workflow and a GitLab job, fetching the ROM and states
  from a secret URI, seeding, checking the tagged segments, and publishing JUnit; which
  pull requests to run it for (the job holds the ROM, and builds the pull request's code).
- `docs/cli.md`, `docs/connectors.md` ("Checking parity in CI") and `docs/mcp.md`.

### 6. Tests

`player/tests/`: the environment variables (used, and not saved), the choice by several
ids and by tag, the JUnit file (a pass, a failure with its line, a skip, a crash),
export then seed into an empty database then check, and a state that matches nothing.

## Built (2026-11-19)

- `Collection::settings` reads `DJ_ROM` and `DJ_CORE`; `save_local` never sees them.
- `parity_cmd::Pick` resolves ids, tags and `--children`, for `dj parity check`,
  `dj cache export` and the MCP `parity_check` tool (`segments`, `tags`).
- `decomp_dj::style` colours verdicts, `warning:`, `failed:` and `error:`. The tests set
  `NO_COLOR` for the `dj` they start, since `CI` turns colour on.
- `player/src/junit.rs` writes the reports. `dj parity check` builds its cases from its own
  rows (with a `seconds` field, also in its JSON); `dj check` from each segment it plays,
  plus those left out after a crash.
- Tests: `player/tests/batch.rs` (`ci_commands`: also that a seeded start is used, by a root
  that would crash the test core if it were played again) and `player/tests/compare.rs`
  (`dj_parity_command`).
- On the Thwaite example: the deepest segment checked in 20 s from an empty database, and
  1.4 s after seeding its one start state (36 KB).

## Open

- **Fixed 2026-11-19: the first segment a process checks skipped a frame, when its start was
  loaded.**
  Not seeding's fault, and not Mesen's lag flag. Mesen's connector serves
  `dj_read_registers` from a pointer (`DjCoverage::Regs`) it sets only when the CPU runs an
  instruction. A process whose first act is to load a state has run none, so the read
  returns nothing. The Thwaite port decides at the jump by reading the PC: in a wait's spin it
  copies there and then, between frames, and frame 0 is judged; with no registers it waits
  for the next wait breakpoint, inside frame 0, and that frame is skipped. Shown on Thwaite
  (2026-11-19): `bfcd75b1` then `fe791912` in one run skip 1 and 0 frames; in the other
  order, 1 and 0 again; each alone, 1. A start played into (no database) runs instructions
  first, so skips none. It doesn't change pass or fail. mGBA's and Mupen64Plus's connectors
  read the CPU directly and are fine.
  - **The fix** (the developer asked for it at once): `CPU::CPU` sets `DjCoverage::Regs`
    to its state (not in the debugger's dummy CPU), in `connectors/mesen/dj-mesen.patch`.
    New build `mesen@0102910c+dj.3959bd49`. Thwaite and Tetris repinned with every hash
    unchanged. After it, `bfcd75b1` and `fe791912`, each checked alone from a seeded start,
    pass every frame (845 and 299), and all 15 Thwaite segments pass against the port with
    nothing skipped (`fcd1470a`: 119 passed, where it was 118 and 1 skipped). The lag flag
    and the licence split, planned for the next repin, still wait for one.
- **This repository's CI** checks every Thwaite segment and doesn't write a JUnit report or
  use a tag. It could, as the example in docs/ci.md does.
- **Seeding a state loads an outside file into the core.** It is checked by hash after
  loading, not before.
