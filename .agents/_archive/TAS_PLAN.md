# Plan: importing and exporting TAS movies

Status: proposed 2026-11-10. Done 2026-11-10: `player/src/tas/` reads and writes `.bk2`,
with profiles for NES, SNES, GB/GBC, GBA and N64; `dj import-tas` and `dj export-tas`;
[docs/tas.md](../docs/tas.md). Checked by exporting and importing chains on Thwaite (NES),
the GBA test and an N64 game: each reaches the chain's end hash.

Scope cut 2026-11-10: `.bk2` only. Readers and writers for `.fm2`, `.lsmv`, `.mmo`, `.m64`
(with its play-along input hook) and `.bsv` were written and checked, then dropped to keep
the surface small; they were never committed. The research on them below stays for if
they come back.

Left for `.bk2`: see "Finishing `.bk2`" at the end.

## The goal

TASVideos and its community have thousands of finished input movies that drive a game
from power-on to the credits, and each one, played back, runs most of the game's code.
For a decomp project that is a cheap way to get a large share of the ROM covered on
day one. The other way round, a chain of segments is already a TAS. Exporting it lets a
person check a route in BizHawk or FCEUX, or hand it to people who use those tools.

Two features:

1. **Import** a TAS movie as a new **root segment**, starting from power-on, with no
   hashes yet (unrecorded, dirty). Playing it once records its hashes, as for any new
   segment.
2. **Export** the chain from the root segment down to the selected segment as one
   movie: up to the **playhead** (when the playhead is in the selected segment) or to
   the **end of the selected segment**.

## How decomp-dj and TAS movies differ

| | decomp-dj segment | TAS movie |
| --- | --- | --- |
| Clock | One line per `retro_run()` | One record per emulated frame (`.fm2`, `.bk2`), or per input **poll** (`.m64`) |
| Storage | Sparse: a value holds until a later line changes it | Dense: every frame lists every button |
| Inputs | libretro `(port, device, index, id)` | The console's own buttons, named per emulator and core |
| Start | Power-on, an SRAM blob, or a core snapshot (roots only) | Power-on, SaveRAM, or that emulator's savestate |
| Proof it plays the same | State hashes of chosen memory | None in the file; people watch it |
| Notes | `[EVENT]` lines | Subtitles, comments |
| Extras | `[WRITE]`, `[RNG]`, `[DEVICE]`, `[RESET]` | Reset and power buttons, disk swaps, coin inserts |

The input mapping is a lookup table. The hard part is **sync**. A TAS only plays the same
on the emulator, core and settings it was made with, because it relies on exact timing,
lag frames and RNG. dj plays a ROM on one pinned libretro core. An imported movie will
match the original run only when that core emulates the console the same way as the
recording emulator, at frame and lag level. Two things follow:

- The best sources are movies made on the **same core family**. BizHawk's GBA core is
  mGBA and its N64 core is Mupen64Plus, the same cores the mGBA and Mupen64Plus-Next
  connectors patch. For NES, the baseline is Mesen, and neither BizHawk (NesHawk,
  QuickNES) nor FCEUX is Mesen, so NES imports will desync more often.
- An import is always **best effort**, and the tools must say so. The segment is valid
  and plays deterministically on dj's core either way. It's still useful when it desyncs,
  because coverage up to the desync is real. But we can't promise "this reaches the
  credits". See [checking sync](#checking-sync).

## Formats

Researched on TASVideos and in the emulators' docs. Ranked by how much they're used,
whether they match dj's systems, and how likely they are to sync on dj's cores.

| Format | Emulator | Systems we care about | Container | Clock | Priority |
| --- | --- | --- | --- | --- | --- |
| `.bk2` | BizHawk | NES, GBA, GB/GBC, N64, SNES, … | zip of text files | frame | **1**: most new TASVideos submissions, same cores for GBA/N64 |
| `.fm2` | FCEUX | NES | text (or binary log) | frame | **2**: the classic NES archive, trivial to parse |
| `.m64` | Mupen64 / Mupen64-rr(-lua) | N64 | binary, 1024-byte header | input **poll** | **3**: most of the N64 archive, needs poll alignment |
| `.mmo` | Mesen | NES | zip | frame | stretch: the same emulator as the NES baseline; check that Mesen 1's format is documented well enough |
| `.bsv` | RetroArch | any libretro core | binary | frame | later: libretro-native, but its layout changes between RetroArch versions (BSV1, then a v2 with compression and checkpoints) |
| `.lsmv`, `.ltm`, `.dtm`, `.vbm`, `.smv`, `.gmv` | lsnes, libTAS, Dolphin, VBA, Snes9x, Gens | systems dj has no connector for | | | out of scope until there's a core for them |

### `.bk2` (BizHawk)

A zip archive with these members:

- `Header.txt`: `key value` lines. The keys we use are `MovieVersion`, `Platform`
  (`NES`, `GBA`, `N64`, `GB`, …), `Core`, `GameName`, `SHA1` (despite its name it can
  hold a CRC32, MD5 or SHA1 in hex, so tell them apart by length), `BoardName`,
  `rerecordCount`, `Author`, `emuVersion`, `StartsFromSavestate` and `StartsFromSaveRam`.
- `Input Log.txt`: an `[Input]` section, then a **LogKey** line
  (`LogKey:#Reset|Power|#P1 Up|P1 Down|…`) that names every column in order, then one
  `|…|` line per frame, closed by `[/Input]`. A button is `.` when released and any
  other character when pressed. An analog value is a number padded to 5 characters
  followed by a comma. BizHawk itself ignores the LogKey when it loads a movie, but it
  always writes one, so it is the reliable way to work out the columns.
- `SyncSettings.json`: the core's settings that affect sync (N64 controller paks,
  NES region and so on).
- Optional `Comments.txt`, `Subtitles.txt` (`subtitle <frame> <x> <y> <length> <ARGB> <text>`),
  `CoreState` (the movie starts from a savestate) and `SaveRam`.

The parser works from the LogKey and never from fixed column offsets. Each
`(Platform, column name)` pair maps to a dj input through a [profile](#profiles).
The exact LogKeys for NesHawk, mGBA, GBHawk/Gambatte and Mupen64Plus come from real
`.bk2` files kept as test fixtures. Don't write them from memory.

### `.fm2` (FCEUX)

A text header (`key value`; `version 3` must come first) and one record per frame:
`|commands|port0|port1|port2|`.

- `commands` is a decimal bit field: 1 is a soft reset, 2 a hard reset (power), 4 and 8
  are FDS disk insert and select, 16 inserts a VS coin.
- A gamepad field is 8 characters in the order `RLDUTSBA`. Anything other than `' '` or
  `'.'` counts as pressed. With `fourscore 1` there are four pad fields.
- `binary 1` means a fixed-size binary log: a command byte, then one byte per pad with
  bits `A B Select Start Up Down Left Right` from bit 0.
- We use these keys: `romChecksum` (base64 of the ROM's MD5, without the iNES header),
  `romFilename`, `palFlag`, `port0`/`port1` (0 none, 1 gamepad, 2 Zapper),
  `savestate`, `length`, `comment author …` and `subtitle <frame> <text>`.
- Zapper fields (`XXX YYY B Q Z`) are left for later. Import refuses them with a clear
  error.

### `.m64` (Mupen64)

A 1024-byte little-endian header. The fields we use: the signature `M64\x1A`, version 3,
VI count, input sample count, start type (1 snapshot, 2 power-on, 4 EEPROM),
controller flags (present, mempak and rumblepak bits), the internal ROM name, the ROM
CRC32, country, author and description. The input begins at `0x400` (`0x200` in
versions 1–2).

Each sample is 4 bytes: a 16-bit button field (C-pad, R, L, D-pad, Start, Z, B, A),
then a signed X and a signed Y (`-128..127`, up is positive in the file). The samples
are **not per frame**: the stream holds one sample per controller, in the order the
game polls them. Lag frames have no sample, and a frame that polls twice has two. The
reset flag is the two reserved bits (0x0040 and 0x0080).

## Mapping inputs

### Profiles

A **profile** says how one movie platform's buttons map to libretro inputs on one
core. It's a small table in code, keyed by `(movie platform, dj core name)`:

```rust
struct Profile {
    platform: &'static str,          // "NES", "GBA", "N64"
    core: &'static str,              // matches config.toml [core] name: "mesen", "mgba", "mupen64plus…"
    buttons: &'static [(&'static str, InputTarget)],  // "P1 A" -> JOYPAD A on port 1
    analog: &'static [(&'static str, AnalogTarget)],  // "P1 X Axis" -> ANALOG LEFT X, with a scale
    console: &'static [(&'static str, ConsoleAction)], // "Reset" -> [RESET], "Power" -> reject or root start
    required_options: &'static [(&'static str, &'static str)], // core options this mapping assumes
}
```

The core name selects the profile, because cores map consoles to the RetroPad
differently. Mupen64Plus-Next puts the C buttons on the right stick or on face
buttons depending on a core option, and Z on L2. The profile lists the core options it
depends on. Import and export check them against `config.toml` and warn when they differ.

Three profiles to start with, each checked against a hand-made segment in the matching
example project:

- **NES / mesen**: `A B Select Start Up Down Left Right` to `JOYPAD` on ports 1–2. Check
  against how Thwaite's segments use `JOYPAD A`/`B`. Four Score and Zapper come later.
- **GBA / mgba**: `A B L R Select Start` and the D-pad. The `Power` column only
  imports when it's on frame 0.
- **N64 / mupen64plus-next**: the buttons as the core option set maps them, and the stick
  as `ANALOG LEFT X/Y`. The N64 range `-128..127` maps to libretro's `-32768..32767`
  through the core's own scaling. To make that exact, the profile requires the core's
  deadzone at 0 and sensitivity at 100%, and imports each value as the libretro value the
  core turns back into the same N64 value. A unit test checks the round trip for all 256
  values, against the core's source. Without that the stick drifts and the movie desyncs
  at once, so the plan doesn't treat this as optional.

### Dense to sparse, and back

- **Import** keeps the last value of each input and writes a line only when it changes:
  `P1 JOYPAD A 1` when the button goes down and `… 0` when it comes up. Analog values work
  the same way, so a stick held still costs nothing. A 100 000-frame movie typically ends
  up with a few thousand lines.
- **Export** walks the chain from the root down to the end point and keeps the
  current value of every input. Each segment starts with **all inputs released**
  ([segments.md](../docs/segments.md#frames)), so at each segment boundary the export
  releases everything before it applies the child's frame-0 lines. The frames it writes
  are exactly what the core sees, which is what the movie has to hold. It writes one
  dense record per frame.

### Commands and markers

| dj | Import from | Export to |
| --- | --- | --- |
| `[RESET]` | `.fm2` command bit 1, `.bk2` `Reset` column, `.m64` reset bits | the same |
| Power cycle | `.fm2` command bit 2, `.bk2` `Power` (only on frame 0, which is power-on anyway) | — (dj has no power line). Refused later in the movie |
| `[EVENT]` | `.bk2` subtitles, `.fm2` `subtitle` lines | the same. Exported events are what make a movie navigable in the other tool |
| `[DEVICE]` | port settings in the header | header only. A change mid-chain is refused |
| `[WRITE]`, `[RNG]` | — | **Refused**: a poked chain is not a real run, and no movie format can carry it |
| `[SCREENSHOT]` | — | dropped |
| FDS disk and VS coin commands | refused, with the frame | — |

## Starting state

- **Power-on**: imports as a root segment with neither `snapshot` nor `sram`.
- **SaveRAM** (`.bk2` `SaveRam` member with `StartsFromSaveRam`, `.m64` start type 4
  with an EEPROM file next to it): written as an `sram` blob on the root segment. On
  export, a root segment with `sram` writes `SaveRam` into a `.bk2`. `.fm2` and `.m64`
  exports refuse it.
- **Savestate** (`.bk2` `CoreState`, `.fm2` `savestate`, `.m64` start type 1):
  **refused**. Those states belong to another emulator and dj can't load them. The
  error says so and names the format. On export, a root segment with a `snapshot` is
  refused for the same reason in reverse.

## The ROM check

`config.toml` pins the ROM by SHA-256. Movies carry other hashes:

- `.bk2` `SHA1`: a SHA-1, MD5 or CRC32 in hex, depending on the platform. For NES it is
  usually the hash without the 16-byte iNES header.
- `.fm2` `romChecksum`: base64 of the MD5, without the iNES header.
- `.m64`: CRC32 plus the internal name and country from the N64 header. The CRC is the
  checksum in the cartridge header, not a hash of the whole file.

Import works out the matching hash from the project's ROM (it needs the ROM path,
which `.local/local.toml` already has) with and without a header. It warns when neither
matches, and refuses only with `--strict`. A mismatch is often a different revision or
dump (PRG0/PRG1), which guarantees a desync, so the warning names both hashes. Export
writes these same hashes into the movie, so the other tool accepts the file.

New dependencies: `zip` (`.bk2`, `.mmo`), `sha1`, `md-5` and `crc32fast`. They're all
small and pure Rust. `base64` is already a dependency.

## The segment an import makes

```
dj-replay 1
id: <new uuid>
parent: none
title: <GameName or file name> (TAS by <Author>)
frames: <frame count>
tags: tas imported
tas-source: bk2 BizHawk 2.9.1 NesHawk
tas-file: smb-warps.bk2
tas-rerecords: 41234
---
0 [EVENT] Imported from smb-warps.bk2 (BizHawk, NesHawk); sync not checked
…input lines…
```

Unknown header keys are kept by every tool ([segments.md](../docs/segments.md#header-fields)),
so `tas-*` keys need no format change. In the GUI the segment arrives **unsaved**, the
same as an agent draft: File > Save makes it part of the project.

A very long movie as a single segment works, but it is clumsy in the piano roll and in
the database (one content key for the whole run). Import takes `--split-every N` and
`--split-at-subtitles`, which cut it into a chain with the [split](../docs/editing.md#tree-edits)
semantics. The default is one segment, as asked.

## Lag frames and the `.m64` poll clock

`input_poll_cb` is a no-op today, and the player doesn't count polls. `.m64` needs both:

1. **Count polls and reads.** The player counts `input_poll` calls and the
   `input_state` reads of each port on every frame. That's useful beyond TAS: a frame
   with no reads is a **lag frame**. The GUI can show lag frames on the ruler, and the
   piano roll can warn when a press falls only on lag frames.
2. **A play-along import.** An `.m64` can't be turned into frames without running it.
   The importer plays the ROM from power-on. On each frame it hands the core the next
   sample for every controller the game reads, and records the values as segment lines on
   that frame. Frames where the game reads no input use no samples. The import ends when
   the samples run out. If the game polls a controller twice in a frame and the two
   samples differ, that frame can't be represented: dj takes one value per frame. In that
   case the import stops there and keeps what it has, which matches what Mupen's rerecording
   plugins did in practice anyway.
3. **Export to `.m64`** plays the chain and writes one sample per read. The header's VI
   count is the chain's frame count.

The play-along import runs the core, so it goes through the same worker as the batch
commands ([ISOLATION_PLAN.md](ISOLATION_PLAN.md), step 3), because a desynced movie can
put the core somewhere strange. `.bk2` and `.fm2` imports are pure parsing and don't run
the core.

## Checking sync

There's no hash in the movie to check against, so dj checks what it can and says what
it can't:

- **At import:** `dj import-tas --play` plays the new segment once, which records its
  hashes and coverage. When the movie names its length, it reports the frames and lag
  frames reached.
- **Signals in the movie:** a TASVideos movie usually ends at the credits, and its
  subtitles name moments. The import report shows the coverage of the last 10% of the run.
  When that's almost nothing new, the movie has probably desynced into a loop or a game
  over. It's a hint, not a verdict, and is labelled as one.
- **Later, by hand:** the author watches it in the GUI and adds an `[EVENT]` at the
  moment it desyncs, which is a good place to split and fix things by hand.

## Export

Export takes a segment and an end frame. It writes the chain from the root down to that
segment, cut at the end frame. The end frame is either the playhead or the segment's
last frame.

Before it writes anything, it checks:

- The chain must have no `[WRITE]`, `[RNG]` or mid-chain `[DEVICE]` line. The error names
  the first segment and frame that has one.
- The root must have no `snapshot` (`sram` is fine for `.bk2`).
- The profile for the project's core and the target format must exist.
- **Dirty segments:** the export is still allowed, but it warns that the chain hasn't
  been verified.

What it writes:

- **`.bk2`**: `Header.txt` with `MovieVersion BizHawk v2.0`, `Platform`, `GameName`
  (the project name), `SHA1` (the hash the platform uses), `Author` (from
  `--author`, never filled in from git), `rerecordCount 0`, `emuVersion` naming dj, and
  `Core` set to the BizHawk core that matches. `Input Log.txt` with that core's LogKey.
  `Subtitles.txt` from the events. `SyncSettings.json` from the profile's defaults for
  that core, which is the most likely file to need hand edits.
- **`.fm2`**: `version 3`, `emuVersion`, `romFilename`, `romChecksum`, a fresh `guid`,
  the port settings and `length`, then the records. `subtitle` lines come from the events.
- **`.m64`**: needs a play, as above.

The header records what the movie was exported from, so a later import can recognise it
as the same chain: `comment` for `.fm2`, `Comments.txt` for `.bk2`, the description for
`.m64`. Round trip (export and then import) has to give identical input lines. A test
checks that for every format.

## Where it goes

| Piece | Where |
| --- | --- |
| Parsers and writers | `player/src/tas/{mod.rs, bk2.rs, fm2.rs, m64.rs, profile.rs}`, in the `decomp_dj` library so the GUI and MCP share them |
| A movie in memory | `tas::Movie { platform, frames: Vec<Frame>, start: Start, meta, markers }`. Every format reads into it and writes from it, and segments go through it as well, so N formats need N parsers, not N² converters |
| Segment ↔ movie | `tas::from_chain(col, segment, end_frame, profile) -> Movie` and `tas::to_segment(movie, profile) -> Replay`, using `Collection::chain` and `Replay::lines` |
| CLI | `dj import-tas <dir> FILE [--title T] [--split-every N] [--split-at-subtitles] [--play] [--strict]` prints the new segment's id. `dj export-tas <dir> SEGMENT [--frame N] --out FILE [--format bk2\|fm2\|m64] [--author NAME]`, with the format taken from the extension by default |
| GUI | File > **Import TAS movie…** (a file picker, then a summary dialog: platform, frames, author, ROM check, what was refused, and **Import** or **Import and play**). File > **Export TAS movie…** with **To the playhead (frame N)** or **To the end of "<title>"**, a format choice and a save dialog. The same two exports go on the segment's right-click menu. Import is one undo step |
| MCP | later: `import_tas` as a draft, for agents that look for a movie of the game |
| Docs | a new `docs/tas.md`, plus rows in `cli.md` and `gui.md` and a line in the README's docs table |

## Steps

1. **Movie model, `.fm2`, NES/mesen profile, CLI import and export.** It's the simplest
   format, it can be tested with Thwaite, and a hand-made `.fm2` covers the parser.
   Round-trip tests: segment → `.fm2` → segment, and a published FCEUX movie as a fixture.
2. **`.bk2`** for NES, GBA and N64, with real BizHawk fixtures for each LogKey. Add
   `SaveRam` to `sram`.
3. **GUI** import and export, with the end point as either the playhead or the end of the
   segment.
4. **Count polls and mark lag frames** in the player. That's useful even without TAS.
5. **`.m64`** play-along import and export, and the N64 analog round-trip test.
6. Maybe later: `.mmo` (Mesen), `.bsv` (RetroArch), Zapper and Four Score, splitting at
   subtitles, MCP.

## Open questions

- **NES desync on Mesen:** how often do FCEUX and BizHawk NES movies stay in sync on the
  Mesen core? Try 3–5 published movies early in step 1, before investing in the GUI. If
  almost none sync, the docs should present NES import as "coverage until it desyncs" and
  push `.mmo` up the list.
- **RAM at power-on:** Mesen's `mesen_ramstate = "All 0s"` against FCEUX's and NesHawk's
  default patterns. Should import propose a core option change? That would be a different
  pinned setup, so no: it warns, and the author decides.
- **Movie identity:** should exported movies carry the chain's end hash in their
  comments, so a later import can reconnect to it? That's cheap and harmless, so probably
  yes.

## Finishing `.bk2`

Done 2026-11-10:

- 22 published BizHawk movies (TASVideos, every supported console) import. They showed
  BizHawk's whole-file NES SHA-1 (the export had hashed without the iNES header),
  zstd-compressed members (`MovieSaveRam.bin.zst`), and BizHawk 2.6 to 2.11's N64
  `A Up`-style stick helpers, which are now ignored. The fixtures aren't kept in the repo.
- Super Painter (NES, freeware) syncs to its ending on Mesen. Exporting it again matches
  BizHawk's file frame for frame.
- Four Score export and import, with BizHawk's port-based player numbering (its P2 is the
  console's player 3), and `SyncSettings.json` for NES ports.

Left:

1. Open an exported movie in BizHawk (skipped for now).
2. Sync for SNES, GB, GBA and N64 movies: no free game with a published BizHawk movie
   was found, and there is no SNES or GB core here.
3. **GUI** import and export (step 3 above).
