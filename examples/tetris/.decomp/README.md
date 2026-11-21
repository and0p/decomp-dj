# Tetris C decompilation, with a 3D port

Tetris (NES, USA) as C, translated from the annotated disassembly
[CelestialAmber/TetrisNESDisasm](https://github.com/CelestialAmber/TetrisNESDisasm) at
commit `94581c0`; a port that draws it in 3D; and a libretro core that puts the two
together for decomp-dj. The code holds none of the game's data: its tables are read from
your own ROM the first time it is built.

| Path | What it is |
| --- | --- |
| [decomp/](decomp/) | The game logic as a library: `tetris_init()`, `tetris_tick()`, all state in one struct. Builds to `libtetris.a` |
| [port/](port/) | Draws a game state in 3D with [small3dlib](https://gitlab.com/drummyfish/small3dlib) (CC0). Builds to `libtetris_port.a` |
| [libretro/](libretro/) | The libretro core: decomp + port, a decomp-dj comparison core that takes events (`dj_handle_event`). Builds to `tetris_decomp_libretro.dylib` / `.so` |
| [tools/gen_data.py](tools/gen_data.py) | Writes the game's data tables (`decomp/src/data.c`, not in git) from your ROM; the Makefile runs it |
| [tools/gen_decomp.py](tools/gen_decomp.py) | Regenerates the RAM layout, `data.h` (which tables, where, how long) and the tables from a build of the disassembly |
| [tools/gen_sites.py](tools/gen_sites.py) | Regenerates `decomp/src/sites.h`, the resume points the snapshot loader matches against the stack |
| [tools/measure_draws.py](tools/measure_draws.py) | Measures, in a 6502 emulator, how long the original takes to draw each screen with NMIs off (needs the ROM and py65) |
| [tools/dbg2csv.py](tools/dbg2csv.py), [tools/ramdiff/](tools/ramdiff/) | A decomp-dj symbol map from the reference build; a RAM differ for two cores |

## Building

From the repository root:

```sh
make -C examples/tetris/.decomp/libretro ROM=../../build/tetris_usa.nes   # examples/tetris/build/decomp/tetris_decomp_libretro.dylib (.so on Linux)
```

That builds the two libraries too. They also build on their own (`make -C .../decomp`,
`make -C .../port`). It needs a C11 compiler, and Python 3 the first time.

`ROM` is your own dump of Tetris (USA), needed the first time only. The 29 tables the game
logic reads (piece orientations, drop speeds, the demo's inputs and pieces, the default
high scores, sprite lists; nothing graphical or musical) are the game's own data, so they
are not in this repository. `tools/gen_data.py` reads them from the ROM into
`decomp/src/data.c`, after checking the ROM's sha256, and `data.h` says where each one
is. Once `data.c` is there, plain `make` is enough.

## The decomp

```c
#include "tetris/tetris.h"

struct tetris t;
tetris_init(&t);                    // power on
tetris_tick(&t, pad1, pad2);        // one frame: the NMI, then the game until it waits
tetris_load_nes_snapshot(&t, nesRam, why, sizeof why);   // take over from a real Tetris
```

`struct tetris` is the whole state, so copying it is a save state:

- `ram`: the original's 2 KB of RAM, byte for byte (`ram.b[addr]`), with every variable
  named as in the disassembly at its original address (`ram.v.levelNumber`).
- `ctl`: where the code is. Routines that wait for a frame part-way through are resumable
  functions; each remembers the return address the original's JSR left on the stack.
- `screen`: what the TV shows of the playfield, which lags the RAM (the playfield is drawn
  four rows a frame, and line clears are animated on screen only).

Kept: everything that decides what the game does from one frame to the next. That
includes menu timers, the RNG, the demo (its input and pieces are tables in `data.c`), the
high score table and the endings' timing. Long work with NMIs off (drawing a new screen)
is costed in CPU cycles, measured by running the original's code, so the game skips the
same frames the NES does. The level menu and the name entry screen each take more than a
frame to draw, and the NES loses one there.

Dropped: drawing to the PPU, sprites, the mapper and the audio engine. Scratch bytes those
leave in RAM are not reproduced: the OAM page, `oamStagingLength`, the sprite offsets
(`$A0`-`$A2`), the audio area (`$0680`-`$06FF`, `$E0`-`$EF`), and `generalCounter`-
`generalCounter5` and `originalY` as sprite staging leaves them. What the PPU copies leave
in zero page is reproduced, because the demo reads it as input on its first frames.

`tetris_check_nes_parity(&t, nesRam, why, sizeof why)` compares the game with a real
Tetris at the same frame (see `src/parity.c`): where the code is (the original's stack
against the decomp's resume points, so a frame lost on either side shows on that frame)
and every byte of RAM above that is kept.

`tetris_load_nes_snapshot` copies the RAM, then walks the stack page from `$01FF` down,
matching each return address to a resume point. It declines (returns 1, with a reason)
when the original is between frames' work.

## The port

```c
#include "tetris/port.h"
tetris_port_draw(&t, pixels, 640, 480, 640);   // 0x00RRGGBB, any size up to 1280x960
```

- Playfield squares and pieces are bevelled cubes, flat-shaded under one directional light,
  in the level's colours (the original's `colorTable` on a computed NTSC palette).
- The walls beside and under the playfield are boxes as deep as the cubes. They flash
  white during a Tetris, as the original's frame colour does.
- Text is font8x8 (public domain) extruded into voxels. It sits where the original's
  nametables put it, on every screen. Menu choices come forward and take a colour.
- The camera is a perspective one, up and to the right of the middle.
- No backgrounds or menu pictures. The A-type ending is a small block rocket, and the
  B-type ending shows nothing.

The scene (`tetris/scene.h`) is a plain list of boxes with a camera and a light, built by
`tetris_scene_build()`. The software rasteriser (`src/raster_s3l.c`) is one way to draw it;
a GPU backend can draw the same list.

## The libretro core

- It needs no content and ignores a ROM if one is given.
- The core option **Resolution** offers 640x480 (the default), 320x240, 960x720 and
  1280x960.
- There is no sound: the core sends silence.
- Save states are the `struct tetris` of this build. Reset powers on.
- `retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM)` gives the game's 2 KB of RAM.

For decomp-dj it is a comparison core that takes events (decomp-dj's
`docs/connectors.md`, "Comparison cores"), built on the helper in decomp-dj's
`include/dj_compare.h` (the Makefile's `DJ_INCLUDE` says where that is). dj
tells it where the original has got to; it runs itself to the same place and compares with
`tetris_check_nes_parity`.

- **Where it judges.** The core option **Compare with the original** (`tetris_judge`):
  - **at its wait for the next frame** (the default). Two breakpoints, one in each of the
    original's wait routines (`$AA36` and `$AA4C`, the NOP after `verticalBlankingInterval`
    is cleared), reached once per wait. There the decomp runs one tick and every byte it
    keeps is compared. A third breakpoint at the NMI (`$8005`) takes the controllers as the
    original reads them, which matters for a tick that ends a frame or two after its NMI.
  - **at frame ends.** One tick per frame. When the original is partway through a draw
    that takes more than a frame (the power-on clear, the level menu, name entry), the few
    bytes that draw writes are left out of that one frame (`parity.c`, `inFlight`).
- **After a jump** it loads the baseline's system RAM with `tetris_load_nes_snapshot`, at
  the jump when judging at frame ends and at the next wait otherwise. At a power-on it
  powers on instead.
- **The 15 frames with no wait** in the example's 30 segments are all long work with NMIs
  off: 11 screen draws that lose a frame, and two frames at each of the two power-ons. No
  NMI runs in the middle of a tick, so judging at the wait compares all of them in full.
  If one ever did (a real lag frame), the decomp answers "unjudged" and takes the
  original's state from there.
- **`dj_parity_diffs`**: after a mismatch, the bytes of RAM that differed (the first 32),
  one difference each, so that `dj parity explain` can name the instruction of the
  original that wrote each.
- **`dj_handle_write`**: a write to region `RAM` made between frames goes into the game's
  RAM. One made at an instruction can't be followed: the decomp copies the original again.
- **It halts** where it can't follow the original: gameModeState 9 (the two-player game
  over, which has no handler), or a wait for an NMI with NMIs off.

In `examples/tetris/config.toml`:

```toml
[[compare]]
name = "decomp (3D)"
content = false
preroll = 3
```

`preroll` is for judging at the wait: the decomp copies the original at the first wait
after a jump, and the longest stretch with no wait is two frames (at power-on). With it,
`dj parity` checks frame 0 of every segment that isn't a root.

And in `.local/local.toml`:

```toml
[[compare]]
name = "decomp (3D)"
path = "build/decomp/tetris_decomp_libretro"   # from the project folder; the suffix is added
```

## Status

In the container, the decomp's RAM matched Mesen's (`mesen_ramstate = "All 0s"`) frame for
frame from power-on through the legal and title screens to the start of the demo. That
excludes the frames where the decomp has finished NMI-off work the NES is still doing.
The first differences after that were the dropped sprite-staging scratch above, and
gameplay wasn't checked further. `tetris_load_nes_snapshot` accepted Mesen's RAM at every
frame it was tried on (every 7th, through the attract mode and into a game). The runs that
followed matched Mesen up to that same scratch.

With `tetris_check_nes_parity`, the built core matched Mesen on every frame (code position
and RAM) of 9,000-frame scripted A-type and B-type games, including level 9, high-score
name entry and both endings, from power-on and after syncs at seven points along the way.

`frameCounter` matched Mesen frame for frame through a scripted A-type game (from the menus
through play, game over and name entry with up and down held) and through B-type games,
including level 9. The name being entered matched on every frame.
