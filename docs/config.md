# Configuration

A project has two settings files:

| File | Committed | What it holds |
| --- | --- | --- |
| `config.toml` | yes | The pinned setup everyone shares: ROM, core build, core options, ports, hash rules, ticks, symbol maps and cache defaults. |
| `.local/local.toml` | never | This machine's paths and preferences, and anything from `config.toml` overridden on this machine. |

`dj init` writes both. Edit them by hand afterwards; the tools read them each time they open the project.

## Console defaults

`dj init` loads the core to see which one it is, and for the cores below starts the project with what suits the console. It prints the console it found and anything it chose that changes hashes. All of it is ordinary `config.toml` content, yours to edit.

| Core | Console | `[hash] regions` | `[cache] preset` | `[[ports]]` | `[core.options]` |
| --- | --- | --- | --- | --- | --- |
| Mesen | NES | `RAM`, and `SRAM` when the game has it | `nes-sparse` | Two joypads; `hide`: the buttons a NES pad doesn't have | as the core has them |
| mGBA, a GBA game | GBA | IWRAM and EWRAM through the memory maps (`BUS:0x03000000-0x03007FFF`, `BUS:0x02000000-0x0203FFFF`), and `SRAM`: mGBA's `RAM` is only the first 32 KB of EWRAM | none (so `nes-sparse`) | One joypad; `hide`: the buttons a GBA doesn't have | as the core has them |
| Mesen-S | SNES | `RAM` (its 128 KB of WRAM), and `SRAM` | none (so `nes-sparse`) | Two joypads; `hide`: the buttons a SNES pad doesn't have | `mesen-s_ramstate = "All 0s"`: RAM starts as zeros, not random bytes |
| Mupen64Plus-Next | N64 | `RAM`, and `SRAM` | `n64-sparse` | Four joypads; `hide` and `labels`: the N64's names for the buttons, the stick and the C buttons | as the core has them |
| SameBoy | Game Boy, Game Boy Color | `RAM` (WRAM, every bank), HRAM through the memory maps (`BUS:0x0000FF80-0x0000FFFE`), and `SRAM` when the game has a battery | none (so `nes-sparse`) | One joypad, with only the Game Boy's buttons | as the core has them; its connector forces `sameboy_rtc = "accurate"` ([connectors/sameboy](../connectors/sameboy/README.md)) |
| Genesis Plus GX, a Mega Drive game | Genesis | `RAM` (the 68000's 64 KB, in the host's byte order: [connectors/genesis-plus-gx](../connectors/genesis-plus-gx/README.md#for-a-project)), and `SRAM` when the game has it | none (so `nes-sparse`) | Two joypads, not the eight the core names for multitap adapters; `labels`: the pad's A, B, C, X, Y, Z and Mode | as the core has them |

- **The N64 keeps the core's own options**, the ones most games work with. With them the GPU copies its picture into the RAM dj hashes (`mupen64plus-EnableCopyColorToRDRAM`, `mupen64plus-EnableCopyDepthToRDRAM`), and that copy isn't the same after a jump as from power-on. If a segment's hash differs after a jump, leave the game's framebuffers out with [`[hash] exclude`](#hash) and [`[cache] exclude`](#cache-exclude); where they are depends on the game. Turning both options `Off` (`--option`, or `[core.options]`) also does it, for a game that doesn't read its framebuffers back. `dj init` prints this.
- **A joypad for every port the core names inputs on**, each with the same `hide` and `labels`, so the second NES pad is trimmed like the first. `dj init` says what that comes to ("2 joypads, 8 buttons each").
- **Any other core** (and a Game Boy game on mGBA) gets one joypad on port 1 with every control the core names, `RAM` and `SRAM` to hash, and no `[cache]` table.

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
| `shareable` | no | `true` says the game's code and data may be published with the project (homebrew, open source, a game you own), so the tools don't warn before making a snapshot ([editing.md](editing.md#a-root-from-a-snapshot)). It changes that warning and nothing else: dj never refuses a snapshot or decides what a project shares. |
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

A relative path is from the project's folder, and an absolute path is used as written. A name with no folder in it (`mesen_libretro`, not `./mesen_libretro` or `cores/mesen_libretro`) that isn't in the project's folder is then looked for where dj keeps cores: `$DJ_CORES`, `cores/` next to the program, `build/cores/` of a source checkout, and `build/cores/` in the current folder. So a `config.toml` can name a core that comes with dj and work on every machine, and a path in `local.toml` still replaces it. A path with a folder is never looked for anywhere else.

### `[core.options]`

Every option the core reads, as `key = "value"` strings. `dj init` records all of them (not only the ones you changed), so the setup stays pinned when a core's defaults change. Pass `--option KEY=VALUE` to `init` to set one up front.

### `[[ports]]`

One table per controller port, giving the device plugged in at power-on. Segments can change it later with a `[DEVICE]` line.

| Key | Meaning |
| --- | --- |
| `port` | Port number, starting at 1. |
| `device` | `NONE`, `JOYPAD`, `MOUSE`, `KEYBOARD`, `LIGHTGUN`, `ANALOG`, `POINTER`, or a raw libretro device number (for subclassed devices). |
| `labels` | Optional. What to call this port's controls in the GUI, instead of the core's names: a table of control to label. |
| `hide` | Optional. Controls the console doesn't have, left out of the GUI: a list of controls. |

Ports that aren't listed keep the core's default.

#### Naming the controls

A segment file names inputs as libretro does (`P1 JOYPAD B 1`), whatever the console calls
them. The GUI shows better names when it has them: a core usually says what each input is
on its console, and `labels` and `hide` correct or trim that. They only change what the GUI
shows; no segment, hash or stored state depends on them.

A control is a joypad button as segment lines spell it (`B`, `Y`, `SELECT`, `START`, `UP`,
`DOWN`, `LEFT`, `RIGHT`, `A`, `X`, `L`, `R`, `L2`, `R2`, `L3`, `R3`), a stick's axis
(`LEFT X`, `LEFT Y`, `RIGHT X`, `RIGHT Y`), or one direction of an axis (`RIGHT X-`,
`RIGHT X+`, `RIGHT Y-`, `RIGHT Y+`, and the same for `LEFT`: minus is left and up).

A label on a direction makes that axis two buttons in the GUI, for a console whose buttons
a core reads from a stick: the N64's C buttons are the right stick on Mupen64Plus-Next. A
press on such a button writes the axis pushed all the way (`P1 ANALOG RIGHT Y -32767`),
and the button shows as held wherever the axis is more than half way over.

`dj controls` lists each port's controls with
the core's name and the project's label for each, which is where to start from.

```toml
[[ports]]
port = 1
device = "JOYPAD"
# Mupen64Plus-Next calls these "(C1)", "(C4)" and "C Buttons Mode".
hide = ["A", "X", "R2"]

[ports.labels]
B = "A"
Y = "B"
L2 = "Z"
"LEFT X" = "Stick X"
"LEFT Y" = "Stick Y"
"RIGHT Y-" = "C Up"
"RIGHT Y+" = "C Down"
"RIGHT X-" = "C Left"
"RIGHT X+" = "C Right"
```

The GUI shows, for each controller: the controls the core or `labels` name, less those in
`hide`; for a core that names nothing, the usual eight joypad buttons; and always whatever
the segment uses. "All buttons" in the piano roll shows every control.

### `[hash]`

What a state hash covers ([playback.md](playback.md#state-hashes)).

| Key | Meaning |
| --- | --- |
| `algorithm` | `blake3`. The only value in format 1. |
| `regions` | Memory to hash, in order. Named regions are `RAM`, `SRAM`, `VRAM` and `RTC` (whatever the core exposes through `retro_get_memory_data`). `BUS:0x0000-0x07FF` hashes a range of the CPU bus through the core's memory maps. |

| `exclude` | Optional. Parts of the regions above to leave out, as `REGION:0xSTART-0xEND` (offsets into the region, both ends included) or `REGION:0xOFFSET` for one byte. For memory that isn't game state, such as a framebuffer an emulator's GPU writes into RAM. |

`dj init` picks `RAM`, plus `SRAM` when the game has it, or the console's memory through the bus where `RAM` isn't all of it ([Console defaults](#console-defaults)). Changing `regions` or `exclude` changes every hash, so all segments turn dirty until you accept new hashes.

```toml
[hash]
algorithm = "blake3"
regions = ["RAM", "SRAM"]
exclude = [
    "RAM:0x38F800-0x3FFFFF",   # WWF No Mercy: three 320x240 framebuffers
]
```

- An entry must name a region that `regions` lists whole (`RAM`, `SRAM`, `VRAM` or `RTC`, not a `BUS:` range), or the project doesn't open.
- A range that runs past the end of the region stops the hash with an error; it isn't cut short.
- Excluded bytes are hashed as zero, so the region keeps its size ([playback.md](playback.md#state-hashes)).
- `exclude` hides bytes from dj, not from the game. If the game reads memory that differs from run to run, its play can still differ. Where an emulator option stops the difference at its source, prefer that: on the N64, turning off `mupen64plus-EnableCopyColorToRDRAM` and `mupen64plus-EnableCopyDepthToRDRAM` ([connectors/mupen64plus](../connectors/mupen64plus/README.md)).
- It belongs in `config.toml`, not `local.toml`: everyone on a project must hash the same bytes.

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
sync = "events"           # the default
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
| `sync` | no | How the core follows the baseline. `"events"` (the default) is a core that takes events: it has `dj_handle_event` ([connectors.md](connectors.md#comparison-cores)), copies what it needs from the baseline itself and runs itself; a core without the function doesn't load. `"state"` loads the baseline's save state: only for the same emulator (or one with the same state format), to compare builds or options. |
| `preroll` | no | For a core that takes events: how many frames dj plays before a place it jumps to for a parity check, so the core has copied the baseline by then ([connectors.md](connectors.md#pre-roll)). Default 0. A core that copies the baseline between frames needs none; one that copies at a breakpoint inside the frame needs enough frames to be sure of reaching it, such as one full logic tick. |
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
| `exclude` | `[]` | Parts of memory regions to zero in stored snapshots, as in [`[hash] exclude`](#hash): `REGION:0xSTART-0xEND`. For memory that changes every frame and isn't game state, such as framebuffers ([below](#cache-exclude)). |

The presets:

| Preset | `page_kb` | `diff_every` | `previews` | For |
| --- | --- | --- | --- | --- |
| `nes-sparse` | whole | 0 | `thumb` | Small states; the default. |
| `nes-aggressive` | whole | 60 | `full` | Fast seeking anywhere in a segment. |
| `n64-sparse` | 64 | 18000 | `thumb` | Large states, few snapshots. |
| `n64-aggressive` | 64 | 1800 | `thumb` | Large states, faster seeking. |

All presets set `diff_at_markers = true` and `preview_at = ["start", "marker"]`.

Values are resolved in this order, later winning: the preset (from `local.toml` if it names one, else `config.toml`, else `nes-sparse`), then each key set in `config.toml`, then each key set in `local.toml`.

#### Cache exclude

Large snapshots are stored in pages, and a page two snapshots share is kept once. Memory
that changes every frame, such as a framebuffer, makes its pages new in every snapshot.
`exclude` zeroes those bytes in the snapshots dj stores, so the pages are shared.

```toml
[hash]
regions = ["RAM", "SRAM"]
exclude = ["RAM:0x38F800-0x3FFFFF"]

[cache]
exclude = ["RAM:0x38F800-0x3FFFFF"]
```

- **Every range must also be left out by `[hash] exclude`,** or the project doesn't open
  for playing. A snapshot loaded without those bytes puts zeros where the game had data. If
  the hash covered them, a segment that starts from a stored state would get another start
  hash than its parent's end.
- **The game sees the zeros after a load,** until it writes that memory again. That is
  right for a framebuffer the game only draws to. It is wrong for memory the game reads
  back before it rewrites it.
- **It needs the core to keep the region whole in its save state.** dj finds the region
  in the state by its contents. A core that compresses its states, or keeps the region in
  another byte order, gets its snapshots stored whole, with a note in the console.
- **Snapshots already in the database stay as they are.** Only new ones are zeroed.
- Unlike `[hash] exclude` it changes no hash, so it can be set per machine in
  `local.toml`.

Check it before relying on it: play a segment that has a parent twice (`dj check
<segment>`). The first time fills the database; the second starts from the parent's
stored state and must report the same hashes.

**The range must hold the buffers and nothing else.** Work it out from their size: No
Mercy's three 320 by 240 buffers of 16-bit pixels are 3 × 320 × 240 × 2 = 0x70800 bytes
and end at 0x400000, so they start at 0x38F800. This page used to give 0x38F000, 2 KB too
low. Those 2 KB are the end of the game's heap, and they hold the record of the graphics
task the game is in the middle of sending. With them zeroed in a stored state, a segment
started from it went wrong by its second frame and its match never began. The buffers
themselves are never read back.

Tried with WWF No Mercy (N64), a snapshot every 120 frames over 4,800 frames, with the
GPU's copies to RDRAM on, and the earlier, too wide range: the database was 70.6 MB
without it and 35.4 MB with it. The 2 KB make little difference to that.

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
| `database` | `true` | `false` plays without the local database, like `--no-db` on the commands that take it. The GUI's Project > Use the local database writes it. |
| `max_memory_mb` | `512` | How much memory the GUI's in-memory snapshots for the current segment may use (Emulation > Snapshot memory). |
| `[cache]` | none | Any key from `config.toml`'s `[cache]`, overriding it on this machine only. |
| `[keyboard]`, `[record]` | defaults | The GUI's recording keys and options ([gui.md](gui.md#recording)). |
| `[gui] hide_snapshot_warning` | `false` | "Don't show this again" on the GUI's warning before it makes a snapshot ([editing.md](editing.md#a-root-from-a-snapshot)). |
| anything else | | Any setting from `config.toml`, overriding it on this machine ([layering](#layering)). |
| `[mcp] allow` | `"draft"` | What `dj mcp` lets an agent do: `"query"`, `"draft"` or `"save"` ([mcp.md](mcp.md#what-an-agent-may-do)). `--allow` overrides it. |
| `[mcp] gui` | `false` | Whether the GUI works with agents: their drafts, sessions and the shared view. MCP > Enable MCP writes it. |

Next to it in `.local/`, the tools keep `cache.db` (the [local database](database.md)), and
`dj mcp` keeps `drafts/` (agent [drafts](mcp.md#drafts)), `traces/`, `snapshots/` and
`mcp/` (running sessions and the view shared from the GUI).

## Other project files

These aren't settings, but they sit next to `config.toml` and change results:

- `annotations.toml`: what you know about the ROM that the symbol maps don't say ([below](#annotationstoml)).
- `blobs/`: SRAM and snapshot files a root segment starts from, each file a name and 8 digits of its content hash ([segments.md](segments.md#blobs)).

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
the lists in `dj summary`, `dj gaps` and the Functions tab, and the map. The
report says how many were left out. A function only partly annotated stays, counted on
what's left.

If code runs inside a `[[data]]` or `[[unreachable]]` range, the annotation is wrong (or the
segment does something the game can't): `dj summary` and the Functions tab warn,
naming the entry and the segments. `dj annotations` lists every entry, what each
`[[function]]` changed, and which ranges ran, and fails if any did. A mistake in the file is
reported with the entry it is in, such as `annotations.toml, data entry 3: range ...`.

This replaces `unreachable.txt`. A project that still has one and no `annotations.toml`
stops with an error, rather than counting what it left out: move each line into an
`[[unreachable]]` entry, with its comment as `why`. In the local database, `dj sql` sees
the entries in the `annotation` table (`kind`, `region`, `start`, `end`, `name`, `why`).
