# TAS movies

A TAS movie (tool-assisted speedrun) is an input log made in another emulator. Played back,
it runs most of a game, so importing one is a quick way to cover a large share of a ROM.
In the other direction, a chain of segments is already a movie, and exporting it lets you
check a route in another emulator or hand it to people who use one.

dj reads and writes BizHawk's `.bk2`, the format most TAS movies are made in today. It
holds NES, SNES, Game Boy / Color, GBA and N64 games. BizHawk's GBA and N64 cores are mGBA
and Mupen64Plus, the same emulators as dj's `mgba` and `mupen64plus-next` cores.

## Importing

```sh
dj import-tas <dir> movie.bk2 [--title TEXT] [--strict] [--rom ROM]
```

This adds one **root segment**, holding the whole movie, and prints its id. It has no
hashes yet. Play it once (`dj play <dir> <id> --accept`, or in the GUI) to record them.

What the segment gets:

- The movie's buttons as input lines, written only on the frames where an input changes.
  A soft reset in the movie becomes `[RESET]`, and each subtitle becomes an `[EVENT]`.
- An event at frame 0 that names the file and says sync hasn't been checked.
- Tags `tas imported`, and the header keys `tas-source`, `tas-file`, `tas-author` and
  `tas-rerecords`. Tools keep unknown header keys, so these stay with the segment.
- A title made from the game and the author, unless `--title` gives one.
- A movie that starts from save RAM (`MovieSaveRam.bin`, which BizHawk may store
  compressed) gets it as the segment's `sram`.

The importer checks the ROM hash the movie names against the project's ROM. BizHawk
names it by the SHA-1 of the whole file, but the importer also tries it without an iNES or
SNES copier header, in case the file has one and BizHawk's didn't. When they differ it warns, or with `--strict` it refuses: a different dump or
revision desyncs at once.

It also warns about:

- Core options that differ from what the button mapping needs.
- PAL timing.
- Inputs the core has no button for, such as BizHawk's GBA tilt and light sensor, or a
  Zapper. They are left out.
- N64 input the core can't read exactly (see below).
- Columns BizHawk records per frame that a segment can't hold (`Input Length`, for
  subframe input).

These movies are refused, with the reason given:

- Movies that start from a BizHawk savestate, which dj can't load
- A power cycle after frame 0
- Consoles other than the five above

### Sync

A movie only plays the same on the emulator it was made with. Lag frames, the RAM
contents at power-on (`mesen_ramstate`) and how each emulator handles the first frame can
all make an imported movie drift away from the run it recorded. BizHawk's NES cores are
not Mesen, and its mGBA and Mupen64Plus are older builds than dj's. When a movie drifts,
the segment still plays the same way every time on dj's core, and the coverage it records
up to the desync is real. Watch it in the GUI, and put an `[EVENT]` where it goes wrong:
that's a good place to split it.

Known causes on the `mesen` core:

- **Opposite directions:** Mesen treats Left+Right, or Up+Down, held together as
  neither. BizHawk and a real NES pass them to the game, and TASes use them (the Tetris
  "Mode B" TAS does). dj's Mesen build adds the core option
  `mesen_allow_invalid_input`: set it to `"enabled"` in `config.toml` for such a movie.
  It's off by default, as in Mesen. The importer warns when a movie needs it and it's
  off.
- **A reset on frame 0:** BizHawk's conversions of FCEUX movies hold Reset on the first
  frame, right after power-on. BizHawk ignores it, but Mesen would start the game 5
  frames late, so the importer leaves it out.
- **A frame of difference:** the Tetris "Mode B" TAS also needed every input moved one
  frame earlier to sync on Mesen; the Super Painter TAS didn't. Compare the run against
  the publication's video: if every screen change comes one frame late or early, shift
  the segment's lines by one, and note it in the segment (the Tetris example's
  `tas-modified` header key).
- **A movie ends at its last input:** TASVideos times a run to its last input, so the
  game may finish after the movie ends with no buttons held. Extend the segment past the
  movie to see it, releasing whatever the last frame holds.

### The N64 controls

N64 input goes through Mupen64Plus-Next's core options:

- **`mupen64plus-alt-map`**:
  - `False`: the C buttons are the right stick.
  - `True`: the C buttons are face buttons.
- **Stick deadzone and sensitivity:** for each stick position, the importer works out a
  libretro value that the core turns back into exactly the movie's N64 value.
- **Positions out of reach:** BizHawk lets a stick go to ±127, but Mupen64Plus-Next
  reaches only ±80 straight ahead at sensitivity 100, and ±120 at its highest, 150.
  Movies that push the stick past that, and frames that hold C-Left and C-Right at once
  (which a stick can't), are counted in a warning. Those frames may play differently.
- BizHawk 2.6 to 2.11 also log `A Up`, `A Down`, `A Left` and `A Right` for each pad.
  They are on-screen helpers for the stick that the core never reads, so they are ignored.

## Exporting

```sh
dj export-tas <dir> SEGMENT --out movie.bk2 [--through FRAME] [--author NAME] [--strict] [--rom ROM]
```

This writes the chain from the root segment down to SEGMENT as one movie, ending with
SEGMENT's last frame, or with frame `--through` of SEGMENT, counted from its start. That
is the frame the GUI's playhead shows. On each frame the movie holds exactly what the
core sees. Every segment starts with all inputs released ([segments.md](segments.md#frames)),
so a button held across a segment boundary is released at that boundary in the movie too,
unless the child sets it again at its frame 0.

- Events become subtitles.
- The movie names the ROM by the SHA-1 BizHawk uses, so a ROM must be set.
- A root segment's save RAM goes in the movie as `MovieSaveRam.bin`.
- For the NES, `SyncSettings.json` plugs pads into both ports, or a Four Score into each
  when the chain uses players 3 and 4. BizHawk numbers a Four Score's pads by port, so
  its P2 is the console's player 3 and its P3 is player 2. Export and import swap them.
- A comment names the segment it was exported from. When dj imports that file again, it
  gives the source as `decomp-dj`.

Some things can't go in a movie, so they are left out:

- `[WRITE]` (a poke), `[RNG]` and `[DEVICE]`
- `[RESET]` for GBA and Game Boy, whose BizHawk cores have no reset
- Inputs that have no button on the console, such as Mesen's turbo buttons (`JOYPAD X`/`Y`)
  or a mouse
- A root segment's snapshot, so the movie starts from power-on

All of this is summed up in one warning:

```
warning: this movie may not play as expected in another emulator, because the chain starts from a snapshot and contains:
  * 4 memory writes
  * 5 RNG advances
```

`--strict` refuses the export instead. A dirty chain is exported with a warning, since
nothing has checked that it still plays as recorded.

### What has been checked

- **Round trips:** on Thwaite (NES, Mesen), the GBA test program (mGBA) and an N64 game
  (Mupen64Plus-Next), a chain was exported, imported again as one root segment, and
  played. It reached the same end hash as the chain.
- **Published movies:** 22 BizHawk movies from TASVideos (NES, SNES, Game Boy / Color,
  GBA and N64, BizHawk 2.1 to 2.11) all import.
- **Sync:** the Super Painter TAS (NES, NesHawk) plays to the game's ending on Mesen.
  Exporting that segment again gives the same ROM hash and the same input on every frame
  as BizHawk's own file.
- **Not checked:** sync for SNES, Game Boy, GBA and N64 movies, for lack of a free game
  with a published movie (and of an SNES or Game Boy core). No exported movie has been
  opened in BizHawk yet.

BizHawk reads a movie's columns by its LogKey, so the order dj writes them in doesn't
have to match BizHawk's own. The names do.

## Button mapping

Columns are named as BizHawk names them. Each one maps to a libretro input on the
project's core:

| Console | Cores | Mapping |
| --- | --- | --- |
| NES | `mesen`, `fceumm`, `nestopia`, `quicknes` | A, B, Select, Start and the D-pad to the same `JOYPAD` buttons, players 1 to 4. On `mesen`, this needs `mesen_shift_buttons_clockwise = "disabled"` |
| SNES | `snes9x*`, `bsnes*`, `mesen-s` | The RetroPad is the SNES pad: every button to its namesake, players 1 and 2 |
| Game Boy / Color | `mgba` (for a GB ROM), `gambatte`, `sameboy`, `gearboy` | A, B, Select, Start and the D-pad |
| GBA | `mgba`, `vba-m`, `gpsp` | As Game Boy, plus L and R |
| N64 | `mupen64plus*` | A to `B`, B to `Y`, Z to `L2`, Start, the D-pad, L and R (or `SELECT` and `R2` with alt-map), the C buttons as above, and the stick to `ANALOG LEFT` |
