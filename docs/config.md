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
dj = "0.1.0"
name = "Thwaite"

[rom]
sha256 = "ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173"
file = "thwaite.nes"

[core]
name = "mesen"
build = "mesen@0102910c+dj.34266c1a"
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
| `format` | yes | Project format version: `1`. A dj refuses a project in a format newer than it reads, saying which; it reads every older one. A change that would make an older dj play a project differently raises it; settings it could safely ignore don't. |
| `dj` | no | The newest dj that has written the project (`0.1.0`): written when a dj saves something and is newer than what's there, for bug reports. An older dj opening it warns that settings or features the newer one added may be ignored. So do settings it doesn't know at all (a typo, or a newer dj's). |
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
| `build` | yes | Which build of the core the segments' hashes were recorded with. A core with a connector reports it (`dj_build_id`): the upstream commit and the connector patch, such as `mesen@0102910c+dj.34266c1a`, the same on every platform; other cores, their name and version (`Mesen 0.9.9`). `dj init` writes it. A core reporting another build plays, with a warning: hashes may not match. After rebuilding or updating the core, `dj repin` plays every segment from power-on with it and, if all still match, records the new build. |
| `sha256` | no | Pins one exact library file: a different file doesn't load. A build for another OS is another file, so this suits a project used on one machine; `build` is the portable pin. `dj repin` updates it when it's set. |
| `path` | no | Where the core library is, usually set in `.local/local.toml` ([layering](#layering)). It may leave out the suffix ([core paths](#core-paths)). |

#### Core paths

A core library's `path` (in `[core]`, `core = "path"` in `local.toml`, `--core`, and `[[compare]]`) may leave out the platform's suffix, so one path works on every OS: `cores/mesen_libretro` is `cores/mesen_libretro.dylib` on macOS, `.so` on Linux and other Unixes and `.dll` on Windows. A path that ends in `.so`, `.dylib` or `.dll`, or names a file that exists as written, is used as it is. Any other path gets the suffix added to the whole name, so `cores/foo.v2` is `cores/foo.v2.dylib` on macOS. Nothing is added to the name itself (no `_libretro`). Errors name the file looked for.

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
| `rom_header` | no | Subtracted from the map's file offsets (`ca65-dbg`, `splat`, `ld-map`) or load addresses (`elf`) to give region offsets: the bytes before the region's data in the ROM file, or, for an ELF, the address of the ROM file's first byte (`0x08000000` on the GBA). Default: where the connector says the region lies in the ROM file; else 16 for an iNES ROM, otherwise 0. The bytes the decoder reads are always taken from where the connector says, when it has said. |
| `region` | no | The connector region the offsets belong to. Default `PRG`. Only `PRG` is rolled up for now. |
| `cpu_base` | no | CPU address of region offset 0, for formats that don't carry CPU addresses (`mesen-mlb`, `csv` without the column). For example `0x8000` for a 32 KB NROM game. Without it, blocks split only at branches, jumps and returns. |

The formats (`ca65-dbg`, `csv`, `splat`, `mesen-mlb`, `ld-map`, `elf`) and how each is read are in [coverage.md](coverage.md#symbol-maps).

### `[[compare]]` (optional, repeatable)

Comparison cores: other libretro cores to run beside the baseline, picked in the GUI's
**Compare > Comparison core** menu ([gui.md](gui.md#comparison-cores)). Only one is loaded at a time, and
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
| `name` | yes | What the menu shows, and the key entries merge by. |
| `path` | yes, on each machine | The core library. Relative paths are from the project folder. It may leave out the suffix ([core paths](#core-paths)). It may be the baseline's own library: it is then loaded from a private copy in `.local/compare/`, so the two don't share globals. |
| `build` | no | A free-form id of the build, for people. |
| `sha256` | no | Pins the library, like `[core] sha256`: a different file doesn't load. |
| `sync` | no | How the core is brought to the baseline's state after a jump. `"dj_sync"` (the default) calls the core's own `dj_sync` ([connectors.md](connectors.md#comparison-cores)), and a core without it doesn't load. `"state"` loads the baseline's save state: only for the same emulator (or one with the same state format), to compare builds or options. |
| `content` | no | What the core loads: `true` (the default) is the project's ROM, which a decomp may need for its assets or may ignore; `false` loads nothing; a path loads that file. |
| `[compare.options]` | no | Core options, like `[core.options]`. |

### `[[decoders]]` (optional, repeatable)

Decoder plugins: shared libraries that decode instruction sets dj doesn't, or stand in for a
built-in decoder ([decoders.md](decoders.md)).

| Key | Required | Meaning |
| --- | --- | --- |
| `path` | yes | The library. Absolute as written; relative from the project folder; a bare name is looked for in the project folder, then in `decoders/` next to `dj`. It may leave out the suffix ([core paths](#core-paths)). |
| `sha256` | no | Pins the library: a different file stops the project from opening. |
| `name` | no | When every entry has one, local.toml's entries merge with config.toml's by name; otherwise local.toml's list replaces config.toml's. |

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
| `database` | `true` | `false` plays without the local database, like `--no-db` on every command. The GUI's Project > Use the local database writes it. |
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

- `annotations.toml`: what you know about the ROM that the symbol maps don't say ([below](#annotationstoml)).
- `blobs/`: SRAM and snapshot files a root segment starts from, named by content hash.

## annotations.toml

Optional, next to `config.toml`, and meant to be committed. It marks code no input can
reach, bytes that aren't code, and functions the symbol maps get wrong or miss. Every entry
can say `why`, which the GUI shows over the coverage map and `dj annotations` lists.

```toml
[[unreachable]]                 # code that can't run: left out of the totals
range = "PRG:0x018C-0x0197"
why = "two-player mode, which the US release can't start"

[[data]]                        # bytes that aren't code
range = "PRG:0x0050-0x0059"
why = "jump table after jsr switch_s_plus_2a"

[[function]]                    # name a function, or add one
at = "PRG:0x0161"
name = "branchOnGameMode"
size = 0x13                     # optional
```

Ranges are region offsets, inclusive: `PRG:0x8000-0x80FF`, or one byte, `PRG:0x8123`. The
region is `PRG` when left out.

| Entry | Effect |
| --- | --- |
| `[[unreachable]]` | Its instructions (bytes, without a symbol map) are left out of every total and shown blue on the map. |
| `[[data]]` | Never read as code: disassembly skips it and starts again after it, so a jump table inside a function doesn't throw off the instructions that follow. Left out of every total, with or without a symbol map, and shown blue, like unreachable code. Where it overlaps an `[[unreachable]]` range, data wins. |
| `[[function]]` | At the offset of a map's function, renames it, and resizes it given a `size`. Elsewhere it adds a function: with no `size`, it runs to the end of the map's function it falls inside (which then ends where it starts), or else to the next function. Map functions it overlaps give way to it. |

A function the symbol maps describe that these entries leave nothing of (every instruction
unreachable, or all of it data) isn't counted at all: it drops out of the function totals,
the lists in `dj report`, `dj summary`, `dj gaps` and the Functions tab, and the map. The
report says how many were left out. A function only partly annotated stays, counted on
what's left.

If code runs inside a `[[data]]` or `[[unreachable]]` range, the annotation is wrong (or the
segment does something the game can't): `dj report`, `dj summary` and the Functions tab warn,
naming the entry and the segments. `dj annotations <dir>` lists every entry, what each
`[[function]]` changed, and which ranges ran, and fails if any did. A mistake in the file is
reported with the entry it is in, such as `annotations.toml, data entry 3: range ...`.

This replaces `unreachable.txt`. A project that still has one and no `annotations.toml`
stops with an error, rather than counting what it left out: move each line into an
`[[unreachable]]` entry, with its comment as `why`. In the local database, `dj sql` sees
the entries in the `annotation` table (`kind`, `region`, `start`, `end`, `name`, `why`).
