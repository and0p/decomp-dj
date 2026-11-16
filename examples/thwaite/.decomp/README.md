# Thwaite decomp

A C reimplementation of [Thwaite](https://github.com/pinobatch/thwaite-nes), built as a
libretro core that dj compares against the original ROM. Written from Thwaite's GPL-3.0
source, in its own structure; drawn with OpenGL 3.2 core; no sound. The plan, and what is
done, is in [.agents/THWAITE_DECOMP_PLAN.md](../../../.agents/THWAITE_DECOMP_PLAN.md).

**Status: milestone 1.** The legal screen and the title screen (joypad and mouse) are
written and in parity on every segment that reaches them. Everything after the title is
not written yet: the decomp answers "unjudged" there, with the reason, and copies the
original again when it is back on a screen it has.

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
player/target/release/dj parity examples/thwaite --compare decomp --all
```

Or open the project in dj-gui and pick **Compare > Comparison core > decomp**.

Two scratch tools in `player/examples/` help:

- `thwaite_pictures` plays a segment with the decomp beside the original, as dj-gui does,
  and saves both pictures side by side at the frames asked for.
- `thwaite_breaks` checks the breakpoint table on every segment.

```bash
cd player
cargo run --release --example thwaite_pictures -- ../examples/thwaite ../examples/thwaite/build/thwaite.nes ../build/cores/mesen_libretro d0c14220 9,119 /tmp/pics
```

Don't try the decomp with `dj play --core`: `dj play` saves the core it is given as the
project's own (in `.local/local.toml`), and the project's own core is the original's.

To check that parity catches a bug, build one with a bug planted (held keys count as new
presses) and give it its own entry in `examples/thwaite/.local/local.toml`:

```bash
cmake -S . -B build/broken -DTHW_BREAK_ON_PURPOSE=ON && cmake --build build/broken
```

```toml
[[compare]]
name = "decomp (broken on purpose)"
path = ".decomp/build/broken/thwaite_decomp_libretro"
content = false
```

## How it follows the original

It sets breakpoints in the original (`dj_breakpoints`) and is driven by dj's events
(`docs/connectors.md#events`). It is parked at one of the original's wait loops; when the
original reaches the next wait's exit, the decomp runs its own step to there and compares
every RAM symbol of the original's with its own state, laid out the same way
(`src/compare.c`). On the way it takes what the original read from outside its RAM: the
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
  byte), Mesen's default palette, and the screens' texts (decoded from the build's
  `dtescripts.s`).
- `gen_map.py`: the original's RAM symbols, and the breakpoints: each wait loop's exit,
  each `lda nmis` the game logic reads, and where the controllers' raw bits are read.
  `breakpoints.tsv` is the same table for tools.

## The API

`src/dj_api.h` copies the comparison-core API from `docs/connectors.md`. It goes when dj
ships a header of its own.
