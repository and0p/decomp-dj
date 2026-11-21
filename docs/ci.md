# Checking parity in CI

A port's repository can block a pull request when the port loses parity with the original
game, on every segment or on a chosen few. dj does the checking; the pipeline fetches what
dj can't keep in the repository (the ROM, and optionally states to start from), builds the
port, and reads dj's results.

## What a job runs

```bash
dj cache seed "$STATES"                                   # optional: states to start from
dj parity check --compare port --tag ci --junit parity.xml
```

- **The exit status** gates the job: 0 when nothing failed, 1 when a segment failed (or
  crashed or hung a core), 2 when dj couldn't do its job. Every dj command uses these
  three ([cli.md](cli.md)).
- **The text output** is coloured in CI logs: red for a failure, green for a pass, yellow
  for what was left out. `NO_COLOR` turns it off.
- **`--junit FILE`** writes a JUnit XML report, one test case per segment, which most CI
  systems show as test results. Setting `DJ_JUNIT=DIR` instead has every dj command in the
  job write its report into that folder (`dj-check.xml`, `dj-parity-<compare>.xml`).

## Choosing the segments

`dj parity check` takes segment ids (or unique prefixes), `--tag NAME` (repeatable: the
segments whose `tags:` header has any of them), and `--children` to add the segments below
each one chosen. With none of these it checks every clean segment.

Tag the segments that gate a pull request (`tags: ci` in their header, or in the GUI's
Properties): a handful that reach the code the port has done, rather than every long
segment. The rest can run on a schedule or before a release.

Segments must be clean (their hashes recorded in the files). One that isn't is left out
and listed as skipped. Clean them locally (`dj clean`) and commit.

## The ROM and the cores

The ROM can't live in the repository. Keep it somewhere private (a private repository's
release, a bucket, a server) and put a URI that reaches it in a CI secret; a step downloads
it to a temporary path. dj reads that path from `DJ_ROM`, or `--rom`. `[rom] sha256` in
config.toml pins the ROM, so any other file is refused.

`DJ_CORE` names the baseline core the same way. Neither variable is written into
`.local/local.toml`, as `--rom` and `--core` are, so they can also be set on a developer's
machine without changing its settings. The port's own core is named by a `[[compare]]`
entry ([config.md](config.md#compare-optional-repeatable)): its `path` can be in
config.toml, or in a `.local/local.toml` the job writes (entries are merged by name).

The job holds the ROM and builds the pull request's code, so the code in a pull request can
read the ROM. Run the job for pull requests you trust: GitHub gives secrets to pull requests
from branches of the same repository, and not to those from forks.

## States to start from

Each checked segment starts from its parent's end. Without a local database, dj plays every
segment above it from power-on to get there, which on a deep tree can take most of the job.
States exported on a developer's machine save that:

```bash
dj cache export --tag ci --compare port --out states/   # on a machine that has played them
```

writes the states that checking the chosen segments starts from, each named
`<state hash>.state`: each segment's start, and when the `[[compare]]` entry has a
`preroll`, the start of the segment where its pre-roll begins. A root segment needs none.
In CI:

```bash
dj cache seed states/
```

loads each state, works out its state hash, and stores it only when that is a clean
segment's recorded end hash. Anything else is refused and named, and the command exits 1:
a state from an older version of the project, from a core build that plays differently, or
not a state at all. So a stale archive fails the job early rather than giving wrong results.

- **Export again** when a segment above the chosen ones changes (its end hash changes), or
  the baseline core is rebuilt.
- **A state from another system** loads only if the core's save states carry across builds
  and systems. The hash check refuses one that doesn't load or plays differently.
- **A state holds the game's memory** (on the N64 its code too). Keep the states where the
  ROM is kept, never in the repository, and don't cache `.local/` between runs where a pull
  request can restore it.

On the Thwaite example, checking its deepest segment took 20 s from an empty database and
1.4 s after seeding one state of 36 KB.

## GitHub Actions

A sketch for an NES port checked against Mesen. `ROM_URL` and `STATES_URL` are secrets.

```yaml
name: parity
on: [push, pull_request]

jobs:
  parity:
    runs-on: ubuntu-22.04
    env:
      DJ: decomp-dj-0.1.0-linux-x86_64
      DJ_ROM: ${{ runner.temp }}/game.nes
      DJ_CORE: ${{ github.workspace }}/cores/mesen_libretro
    steps:
      - uses: actions/checkout@v5

      # dj, from a release ZIP. Its connectors/ folder builds the baseline core.
      - name: Get dj and the baseline core
        run: |
          curl -sSLO https://github.com/and0p/decomp-dj/releases/download/v0.1.0/$DJ.zip
          unzip -q $DJ.zip
          bash $DJ/connectors/mesen/build.sh "$PWD/cores"

      # Your port, built as a libretro core with dj_handle_event (connectors.md).
      - name: Build the port's core
        run: make libretro && cp build/mygame_libretro.so cores/

      - name: Get the ROM and the states
        run: |
          curl -fsS "${{ secrets.ROM_URL }}" -o "$DJ_ROM"
          curl -fsS "${{ secrets.STATES_URL }}" | tar -xz -C "$RUNNER_TEMP"

      - name: Point dj at the port
        run: |
          mkdir -p .local
          cat > .local/local.toml <<'TOML'
          [[compare]]
          name = "port"
          path = "cores/mygame_libretro"
          TOML

      - name: Check parity
        run: |
          $DJ/dj cache seed "$RUNNER_TEMP/states"
          $DJ/dj parity check --compare port --tag ci --junit parity.xml

      # Test results, a summary and annotations in the pull request.
      - uses: mikepenz/action-junit-report@v5
        if: always()
        with:
          report_paths: parity.xml

      # What failed, and which instruction of the original wrote each difference.
      - name: Explain the failures
        if: failure()
        run: |
          $DJ/dj parity report --compare port --json > parity-report.json
          for id in $(jq -r '.failures[] | select(.current_build) | .id' parity-report.json); do
            $DJ/dj parity explain "$id" --compare port --json > "explain-$id.json" || true
          done
      - uses: actions/upload-artifact@v5
        if: failure()
        with:
          name: parity
          path: "*.json"
```

A URI with a password in its query string ends up in server and proxy logs; a header
(`curl -H "Authorization: Bearer $TOKEN"`) doesn't. A port that draws with OpenGL needs EGL
on a Linux runner, which Mesa's software renderer gives (`libegl1 libegl-mesa0
libgl1-mesa-dri`). This repository's own CI checks the Thwaite example's port this way
(`.github/workflows/ci.yml`), with a ROM built from the game's source.

## GitLab CI

The same steps; GitLab reads the JUnit report itself.

```yaml
parity:
  script:
    - curl -fsS "$ROM_URL" -o "$CI_PROJECT_DIR/game.nes"
    - export DJ_ROM="$CI_PROJECT_DIR/game.nes" DJ_CORE="$CI_PROJECT_DIR/cores/mesen_libretro"
    - dj cache seed states/
    - dj parity check --compare port --tag ci --junit parity.xml
  artifacts:
    when: always
    reports:
      junit: parity.xml
```

## Notes

- **Results are kept per build of the port** (the sha256 of its library). A fresh build
  checks every chosen segment; a segment already checked with the same build is skipped,
  unless `--again`.
- **A crash or hang in the port's core** is a failure at that frame, not an error. dj plays
  in a child process it watches, records the failure, keeps the frames that passed before
  it, and goes on to the other segments.
- **`dj parity explain`** needs the trace hook in the baseline's connector, which Mesen's
  and Mupen64Plus's have. It plays each failure again, so leave it out for long segments or
  many failures.
- **Two builds of the same emulator** can be compared with no port: `sync = "state"` in the
  `[[compare]]` entry.
- **`dj check --strict`** checks the hashes themselves (no comparison core), failing also
  on a segment with none recorded: a determinism check for the baseline. It takes the same
  segment ids, `--tag` and `--children`, and starts from seeded states the same way.
