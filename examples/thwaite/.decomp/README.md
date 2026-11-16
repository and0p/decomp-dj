# Thwaite decomp

A C reimplementation of [Thwaite](https://github.com/pinobatch/thwaite-nes), built as a
libretro core that dj compares against the original ROM. Written from Thwaite's GPL-3.0
source, in its own structure; drawn with OpenGL 3.2 core; no sound. The plan, and what is
done, is in [.agents/THWAITE_DECOMP_PLAN.md](../../../.agents/THWAITE_DECOMP_PLAN.md).

**Status: milestone 0.** The core loads, draws both pattern tables with a test sprite, and
exports the comparison-core API as stubs, with the real breakpoint table. There is no game
logic yet.

## Build

Needs CMake and a C compiler; nothing else (the data tables are committed).

```bash
./build.sh
```

The core is `build/thwaite_decomp_libretro.dylib` (`.so` on Linux, `.dll` on Windows). To
see it draw, from the repository root:

```bash
player/target/release/dj play examples/thwaite d0c14220 --no-db --rom examples/thwaite/build/thwaite.nes --core examples/thwaite/.decomp/build/thwaite_decomp_libretro --screenshots /tmp/shots
```

The hashes won't match the original's, and without `--accept` nothing is written.

## The tables

`src/data/` is generated from Thwaite's source and debug file, and committed. Regenerate it
when the pinned Thwaite commit (`../build-rom.sh`) changes, after building the ROM:

```bash
python3 tools/gen_assets.py ../build/thwaite-nes ../build/thwaite.nes
python3 tools/gen_map.py ../build/thwaite-nes ../build/thwaite.dbg
```

- `gen_assets.py`: the CHR from `tilesets/*.png` (checked against the ROM's, byte for
  byte), and Mesen's default palette.
- `gen_map.py`: the original's RAM symbols, and the breakpoints: each wait loop's exit,
  each `lda nmis` the game logic reads, and the controller routines. `breakpoints.tsv` is
  the same table for tools; `player/examples/thwaite_breaks.rs` checks it on every segment.

## The API

`src/dj_api.h` copies the comparison-core API from the breakpoint design
(`.agents/yield-parity/PLAN.md`), which dj doesn't have in full yet. It goes when dj ships
its own header.
