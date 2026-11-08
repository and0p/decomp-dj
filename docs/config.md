# Configuration

A project has two settings files:

| File | Committed | What it holds |
| --- | --- | --- |
| `config.toml` | yes | The pinned setup everyone shares: ROM, core build, core options, ports, hash rules, ticks, symbol maps and cache defaults. |
| `.local/local.toml` | never | This machine's paths and preferences, and anything from `config.toml` overridden on this machine. |

`dj init` writes both. Edit them by hand afterwards; the tools read them each time they open the project.

## Layering

Either file may hold any setting described on this page, and `local.toml` wins. The two are merged key by key: tables merge table by table (so `local.toml` can set one `[core.options]` key and keep the rest), arrays of tables whose entries all have a `name` merge entry by entry on that name, and any other value in `local.toml` replaces the one in `config.toml`.

That allows both kinds of project. One can commit everything, paths included:

```toml
# config.toml
[rom]
sha256 = "…"
path = "roms/game.nes"          # relative paths are from the project folder

[core]
name = "mesen"
build = "…"
path = "cores/mesen_libretro.so"
```

Another can commit only the pinned setup and the segments, and leave paths and anything else personal to each `local.toml`. In `local.toml`, `rom = "path"` and `core = "path"` are short for `path` in the `[rom]` and `[core]` tables.

Settings the tools write (Set ROM…, Set core…, `--rom`, `--core`, the GUI's toggles, key bindings) always go to `local.toml`, and writing them keeps whatever else is there.

## config.toml

A complete example, from `examples/thwaite/` (core options shortened):

```toml
format = 1
name = "Thwaite"

[rom]
sha256 = "ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173"
file = "thwaite.nes"

[core]
name = "mesen"
build = "mesen@0102910c+dj1"
sha256 = ""

[core.options]
mesen_ramstate = "All 0s"
mesen_region = "Auto"
# ...every option the core reports

[[ports]]
port = 1
device = "JOYPAD"

[hash]
algorithm = "blake3"
regions = ["RAM"]

[tick]
exec = "PRG:0x197"

[[symbols]]
file = "build/thwaite.dbg"
format = "ca65-dbg"
```

### Top level

| Key | Required | Meaning |
| --- | --- | --- |
| `format` | yes | Project format version. Always `1` for now. |
| `name` | yes | The game or project name, shown in the GUI title and reports. |

### `[rom]`

| Key | Required | Meaning |
| --- | --- | --- |
| `sha256` | yes | SHA-256 of the ROM file exactly as the core loads it (headers included). The tools refuse a ROM that doesn't match. |
| `file` | no | A file name hint for people. The hash is what counts. |
| `path` | no | Where the ROM is, usually set in `.local/local.toml` ([layering](#layering)). |

### `[core]`

| Key | Required | Meaning |
| --- | --- | --- |
| `name` | yes | The libretro core's name, for example `mesen`. |
| `build` | yes | A free-form id of the exact build, such as `mesen@<commit>+dj1`. State hashes are only meaningful for one build, so a different build is a different setup. |
| `sha256` | no | Hash of the core library. Usually empty, because it differs per OS. |
| `path` | no | Where the core library is, usually set in `.local/local.toml` ([layering](#layering)). |

### `[core.options]`

Every option the core reads, as `key = "value"` strings. `dj init` records all of them (not only the ones you changed), so the setup stays pinned when a core's defaults change. Pass `--option KEY=VALUE` to `init` to set one up front.

### `[[ports]]`

One table per controller port, giving the device plugged in at power-on. Segments can change it later with a `[DEVICE]` line.

| Key | Meaning |
| --- | --- |
| `port` | Port number, starting at 1. |
| `device` | `NONE`, `JOYPAD`, `MOUSE`, `KEYBOARD`, `LIGHTGUN`, `ANALOG`, `POINTER`, or a raw libretro device number (for subclassed devices). |

Ports that aren't listed keep the core's default.

### `[hash]`

What a state hash covers ([playback.md](playback.md#state-hashes)).

| Key | Meaning |
| --- | --- |
| `algorithm` | `blake3`. The only value in format 1. |
| `regions` | Memory to hash, in order. Named regions are `RAM`, `SRAM`, `VRAM` and `RTC` (whatever the core exposes through `retro_get_memory_data`). `BUS:0x0000-0x07FF` hashes a range of the CPU bus through the core's memory maps. |

`dj init` picks `RAM`, plus `SRAM` when the game has it. Changing `regions` changes every hash, so all segments turn dirty until you accept new hashes.

### `[tick]` (optional)

What counts as one game logic tick ([playback.md](playback.md#logic-ticks-draft)).

| Key | Meaning |
| --- | --- |
| `exec` | A ROM location, such as `PRG:0x197`. Each time it runs is one tick. Needs a coverage connector. |
| `write` | Reserved: each write to a RAM address. Not implemented. |

### `[[symbols]]` (optional, repeatable)

Symbol maps that roll coverage up into functions and basic blocks ([coverage.md](coverage.md#symbol-maps)). When two maps describe the same bytes, the first one listed wins.

| Key | Required | Meaning |
| --- | --- | --- |
| `file` | yes | Path relative to the project root. |
| `format` | yes | One of the formats in coverage.md. `dj init --symbols FILE` guesses it from the file name. |
| `rom_header` | no | Bytes before the region's data in the ROM file. Default: 16 for an iNES ROM, otherwise 0. Subtracted from file offsets for `ca65-dbg`, `splat`, `ld-map` and `elf`. |
| `region` | no | The connector region the offsets belong to. Default `PRG`. Only `PRG` is rolled up for now. |
| `cpu_base` | no | CPU address of region offset 0, for formats that don't carry CPU addresses (`mesen-mlb`, `csv` without the column). For example `0x8000` for a 32 KB NROM game. Without it, blocks split only at branches, jumps and returns. |

The formats (`ca65-dbg`, `csv`, `splat`, `mesen-mlb`, `ld-map`, `elf`) and how each is read are in [coverage.md](coverage.md#symbol-maps).

### `[[compare]]` (optional, repeatable)

Comparison cores: other libretro cores to run beside the baseline, picked in the GUI's
**Compare** dropdown ([gui.md](gui.md#comparison-cores)). Only one is loaded at a time, and
none is until you pick it. Entries merge by `name` across `config.toml` and
`.local/local.toml` ([layering](#layering)), so the shared entry can carry `name`, `sync`
and `options` and each machine's `local.toml` adds its own `path`.

```toml
[[compare]]
name = "decomp"
sync = "dj_sync"          # the default
build = "a1b2c3d"

[[compare]]
name = "mesen, other settings"
path = "cores/mesen_libretro.dylib"
sync = "state"
[compare.options]
mesen_ntsc_filter = "Composite"
```

| Key | Required | Meaning |
| --- | --- | --- |
| `name` | yes | What the dropdown shows, and the key entries merge by. |
| `path` | yes, on each machine | The core library. Relative paths are from the project folder. It may be the baseline's own library: it is then loaded from a private copy in `.local/compare/`, so the two don't share globals. |
| `build` | no | A free-form id of the build, for people. |
| `sha256` | no | Pins the library, like `[core] sha256`: a different file doesn't load. |
| `sync` | no | How the core is brought to the baseline's state after a jump. `"dj_sync"` (the default) calls the core's own `dj_sync` ([connectors.md](connectors.md#comparison-cores)), and a core without it doesn't load. `"state"` loads the baseline's save state: only for the same emulator (or one with the same state format), to compare builds or options. |
| `content` | no | What the core loads: `true` (the default) is the project's ROM, which a decomp may need for its assets or may ignore; `false` loads nothing; a path loads that file. |
| `[compare.options]` | no | Core options, like `[core.options]`. |

### `[cache]` (optional)

Defaults for the local database (`.local/cache.db`) that everyone on the project starts from. Any key can be overridden per machine in `.local/local.toml`.

| Key | Default | Meaning |
| --- | --- | --- |
| `preset` | `nes-sparse` | Starting values for the keys below. |
| `page_kb` | from preset | Split snapshots into pages of this many KB, each stored once by hash. Missing, or the snapshot size or more, keeps snapshots whole. |
| `diff_every` | from preset | Also store a snapshot every N frames. `0` means only at frame 0, markers and segment ends with children. |
| `diff_at_markers` | `true` | Store a snapshot at each marker. |
| `previews` | from preset | `thumb` (half width and height), `full` or `off`. |
| `preview_at` | `["start", "marker"]` | Where previews are taken. |
| `budget_gb` | none | `dj cache stats` warns when the database is larger than this. |
| `exclude` | `[]` | Address ranges that churn every frame (framebuffers). Recorded but not applied yet. |

The presets:

| Preset | `page_kb` | `diff_every` | `previews` | For |
| --- | --- | --- | --- | --- |
| `nes-sparse` | whole | 0 | `thumb` | Small states; the default. |
| `nes-aggressive` | whole | 60 | `full` | Fast seeking anywhere in a segment. |
| `n64-sparse` | 64 | 18000 | `thumb` | Large states, few snapshots. |
| `n64-aggressive` | 64 | 1800 | `thumb` | Large states, faster seeking. |

All presets set `diff_at_markers = true` and `preview_at = ["start", "marker"]`.

Values are resolved in this order, later winning: the preset (from `local.toml` if it names one, else `config.toml`, else `nes-sparse`), then each key set in `config.toml`, then each key set in `local.toml`.

## .local/local.toml

Never committed. When the project has no `.gitignore`, the tools create one that lists `.local/`.

```toml
rom = "/Users/me/roms/thwaite.nes"
core = "/Users/me/cores/mesen_libretro.dylib"
database = true
max_memory_mb = 512

[cache]
diff_every = 300
```

| Key | Default | Meaning |
| --- | --- | --- |
| `rom` | none | Path to the ROM (short for `[rom] path`). Set by `--rom`, remembered after the first use. |
| `core` | none | Path to the core library (short for `[core] path`). Set by `--core`, remembered after the first use. |
| `database` | `true` | `false` plays without the local database, like `--no-db` on every command. The GUI's Emulation > Use the local database writes it. |
| `max_memory_mb` | `512` | How much memory the GUI's in-memory snapshots for the current segment may use (Emulation > Snapshot memory). |
| `[cache]` | none | Any key from `config.toml`'s `[cache]`, overriding it on this machine only. |
| `[keyboard]`, `[record]` | defaults | The GUI's recording keys and options ([gui.md](gui.md#recording)). |
| anything else | | Any setting from `config.toml`, overriding it on this machine ([layering](#layering)). |
| `[mcp] allow` | `"draft"` | What `dj mcp` lets an agent do: `"query"`, `"draft"` or `"save"` ([mcp.md](mcp.md#what-an-agent-may-do)). `--allow` overrides it. |
| `[mcp] gui` | `false` | Whether the GUI works with agents: their drafts, sessions and the shared view. MCP > Enable MCP writes it. |

Next to it in `.local/`, the tools keep `cache.db` (the [local database](database.md)), and
`dj mcp` keeps `drafts/` (agent [drafts](mcp.md#drafts)), `traces/`, `snapshots/` and
`mcp/` (running sessions and the view shared from the GUI).

## Other project files

These aren't settings, but they sit next to `config.toml` and change results:

- `unreachable.txt`: ROM ranges left out of coverage totals, one per line (`0x8000-0x80FF`, `0x8123`, optionally `PRG:` first, `#` comments).
- `blobs/`: SRAM and snapshot files a root segment starts from, named by content hash.
