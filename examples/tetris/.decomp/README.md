# Tetris C decompilation, with a 3D port

Tetris (NES, USA) as C, translated from the annotated disassembly
[CelestialAmber/TetrisNESDisasm](https://github.com/CelestialAmber/TetrisNESDisasm) at
commit `94581c0`; a port that draws it in 3D; and a libretro core that puts the two
together for decomp-dj. None of it needs the ROM.

| Path | What it is |
| --- | --- |
| [decomp/](decomp/) | The game logic as a library: `tetris_init()`, `tetris_tick()`, all state in one struct. Builds to `libtetris.a` |
| [port/](port/) | Draws a game state in 3D with [small3dlib](https://gitlab.com/drummyfish/small3dlib) (CC0). Builds to `libtetris_port.a` |
| [libretro/](libretro/) | The libretro core: decomp + port, with decomp-dj's `dj_sync` and `dj_write`. Builds to `tetris_decomp_libretro.dylib` / `.so` |
| [tools/gen_decomp.py](tools/gen_decomp.py) | Regenerates the RAM layout and the game's data tables from a build of the disassembly |
| [tools/gen_sites.py](tools/gen_sites.py) | Regenerates `decomp/src/sites.h`, the resume points the snapshot loader matches against the stack |
| [tools/dbg2csv.py](tools/dbg2csv.py), [tools/ramdiff/](tools/ramdiff/) | A decomp-dj symbol map from the reference build; a RAM differ for two cores |
| [_old/](_old/) | The previous ROM-based port and core, its plan (PLAN.md, QUESTIONS.md, functions.csv) and its header generator, kept for reference |

## Building

From the repository root:

```sh
make -C examples/tetris/.decomp/libretro   # examples/tetris/build/decomp/tetris_decomp_libretro.dylib (.so on Linux)
```

That builds the two libraries too. They also build on their own (`make -C .../decomp`,
`make -C .../port`). A C11 compiler is all it needs.

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
is costed in CPU cycles, so the game skips the same frames the NES does.

Dropped: drawing to the PPU, sprites, the mapper and the audio engine. Scratch bytes those
leave in RAM are not reproduced: the OAM page, `oamStagingLength`, the sprite offsets
(`$A0`-`$A2`), the audio area (`$0680`-`$06FF`, `$E0`-`$EF`), and `generalCounter*` and
`originalY` values left by sprite staging during play.

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

For decomp-dj:

- **`dj_sync`**: with `DJ_SYNC_BOOT` it powers on. Otherwise it loads the baseline's system
  RAM with `tetris_load_nes_snapshot`, returning 1 with the reason when it can't.
- **`dj_write`**: region `RAM` is written into the game's RAM. Other regions are ignored (it
  returns 1).

In `examples/tetris/config.toml`:

```toml
[[compare]]
name = "decomp (3D)"
sync = "dj_sync"
content = false
```

And in `.local/local.toml`:

```toml
[[compare]]
name = "decomp (3D)"
path = "/Users/andrew/Source/emu_coverage/examples/tetris/build/decomp/tetris_decomp_libretro.dylib"
```

## Status

In the container, the decomp's RAM matched Mesen's (`mesen_ramstate = "All 0s"`) frame for
frame from power-on through the legal and title screens to the start of the demo. That
excludes the frames where the decomp has finished NMI-off work the NES is still doing.
The first differences after that were the dropped sprite-staging scratch above, and
gameplay wasn't checked further. `tetris_load_nes_snapshot` accepted Mesen's RAM at every
frame it was tried on (every 7th, through the attract mode and into a game). The runs that
followed matched Mesen up to that same scratch.
