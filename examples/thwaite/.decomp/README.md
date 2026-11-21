# Thwaite decomp

A C reimplementation of [Thwaite](https://github.com/pinobatch/thwaite-nes), built as a
libretro core that dj compares against the original ROM. Written from Thwaite's GPL-3.0
source, in its own structure; drawn with OpenGL 3.2 core; no sound. The plan, and what is
done, is in [.agents/_archive/THWAITE_DECOMP_PLAN.md](../../../.agents/_archive/THWAITE_DECOMP_PLAN.md).

**Status: done (milestone 5).** Every screen the segments reach is in parity on every frame: the
legal screen, the title (joypad and mouse), the practice menu, gameplay (pause, game over,
balloons, MIRVs, clearing a level, the silo rebuild, the hour changing), the cutscenes, and
the day changing. Only boot's first frames, before its first wait, aren't judged. The
pictures match the original's pixel for pixel, and a failure can be traced to the original's
code with `dj parity explain`.

## Build

Needs CMake and a C compiler; nothing else (the data tables are committed).

```bash
./build.sh
```

The core is `build/thwaite_decomp_libretro.dylib` (`.so` on Linux, `.dll` on Windows).
`examples/thwaite/config.toml` names it as the comparison core `decomp`.

## Check it

From the repository root:

```bash
target/release/dj parity check examples/thwaite --compare decomp --again
```

Or open the project in dj-gui and pick **Compare > Comparison core > decomp**.

CI does the same on every push and pull request (`.github/workflows/ci.yml`, the Thwaite
job, on Linux and macOS): it builds the decomp after replaying the example, and fails if a
frame differs. On Linux the decomp's OpenGL context comes from Mesa's software renderer
through EGL, so the runner needs no GPU.

Three scratch tools in `examples/thwaite/tools/` (the `thwaite-tools` crate) help:

- `thwaite_pictures` plays a segment with the decomp beside the original, as dj-gui does,
  and saves both pictures side by side at the frames asked for. A last list of frames
  has the decomp copy the original again before each, as after a rewind, to check that
  its picture is whole after a copy.
- `thwaite_breaks` checks the breakpoint table on every segment.
- `thwaite_bot` records a segment by playing the original itself: it clicks the practice
  menu, presses buttons, and shoots down missiles (letting those aimed at one building
  through, if asked), then writes what it pressed into an empty segment from `dj new`.

```bash
cargo run --release -p thwaite-tools --bin thwaite_pictures -- examples/thwaite examples/thwaite/build/thwaite.nes build/cores/mesen_libretro d0c14220 9,119 /tmp/pics
```

```bash
cargo run --release -p thwaite-tools --bin thwaite_bot -- examples/thwaite examples/thwaite/build/thwaite.nes build/cores/mesen_libretro <new segment> click:84,44 click:84,60 press:START defend
```

Run `dj clean` afterwards to record the segment's hashes.

Don't try the decomp with `dj play --core`: `dj play` saves the core it is given as the
project's own (in `.local/local.toml`), and the project's own core is the original's.

When a frame fails, `dj parity explain` says which instruction of the original wrote each byte
that differs: the code to compare with the decomp's. Take the failure's id from `dj parity report`:

```bash
target/release/dj parity report examples/thwaite --compare decomp
target/release/dj parity explain examples/thwaite <id> --compare decomp
```

To check that parity catches a bug, build one with bugs planted (held keys count as new
presses; enemy smoke every 21 frames, not 20) and give it its own entry in
`examples/thwaite/.local/local.toml`:

```bash
cmake -S . -B build/broken -DTHW_BREAK_ON_PURPOSE=ON && cmake --build build/broken
```

```toml
[[compare]]
name = "decomp (broken on purpose)"
path = ".decomp/build/broken/thwaite_decomp_libretro"
content = false
```

## Render styles

The core option `thwaite_decomp_render` picks how the decomp draws. The project sets it in
`examples/thwaite/config.toml`, under the decomp's `[compare.options]`:

- `flat`: the console's picture, 256x240, pixel for pixel.
- `voxel` (the project's choice): every tile and sprite extruded one voxel deep and drawn in
  perspective at 1024x960, each voxel with a faint outline (`src/voxel.c`). The colours are
  still palette indices looked up in the frame's palette, so fades work as on the console.
  The backdrop colour fills the screen's rectangle; around it is black.

Options for the voxel picture:

| Option | Values | |
| --- | --- | --- |
| `thwaite_decomp_voxel_tilt` | `22` (default), `0`-`40`, `-10`-`-30` | Degrees. Positive: the camera below, the ground nearest. Negative: the camera above |
| `thwaite_decomp_voxel_fov` | `50` (default), `10`-`70` | Vertical field of view. The nearer edge of the screen always just fills the width |
| `thwaite_decomp_voxel_outline` | `20` (default), `off`, `10`-`40` | How much a voxel's edges darken, in percent |

Parity compares RAM, so the style changes only what you see. Two things don't carry over
from the console to voxels:
- the 8-sprites-a-line limit;
- a sprite behind the BG hiding a higher-numbered one in front, which on the console
  makes the cutscene's leaves cover the top of the trunk.

`thwaite_pictures` draws flat whatever the project sets, so its pictures can be compared.
To see voxels, give it `THW_OPTIONS=thwaite_decomp_render=voxel`. With
`thwaite_decomp_voxel_tilt=0,thwaite_decomp_voxel_fov=10,thwaite_decomp_voxel_outline=off`
the voxel picture is the flat one at 4x, apart from those two things. That is how
`voxel.c`'s layering, palettes, flips and splits were checked.

## How it follows the original

It sets breakpoints in the original (`dj_breakpoints`) and is driven by dj's events
(`docs/connectors.md#events`). It is parked at one of the original's wait loops; when the
original reaches the next wait's exit, the decomp runs its own step to there and compares
every RAM symbol of the original's with its own state, laid out the same way
(`src/compare.c`). In a cutscene that includes the text: the decoded line and the VRAM
packets waiting for the next vblank. On the way it takes what the original read from outside its RAM: the
controllers' raw bits (at `read_pads` and `read_mouse`) and the NMI counter where the logic
reads it.

## The tables

`src/data/` is generated from Thwaite's source and debug file, and committed. Regenerate it
when the pinned Thwaite commit (`../build-rom.sh`) changes, after building the ROM:

```bash
python3 tools/gen_assets.py ../build/thwaite-nes ../build/thwaite.nes
python3 tools/gen_map.py ../build/thwaite-nes ../build/thwaite.dbg
```

- `gen_assets.py`: the CHR from `tilesets/*.png` (checked against the ROM's, byte for
  byte), Mesen's default palette, the screens' texts (decoded from the build's
  `dtescripts.s`), and the cutscene scripts, still compressed, at their addresses in the
  ROM (the original keeps a pointer into them in RAM).
- `gen_map.py`: the original's RAM symbols, and the breakpoints: each wait loop's exit,
  each `lda nmis` the game logic reads, and where the controllers' raw bits are read.
  `breakpoints.tsv` is the same table for tools.

## The API

The API is dj's own headers, included from the repository's `include`:
`dj_connector.h` (the API version, coverage regions, the build id, the trace hook) and
`dj_compare.h` (events, breakpoints, writes, diffs). `src/dj_api.h` just includes both.
The decomp's trace is, after each step, a write event for every byte of the original's RAM
layout the step changed.
