# Tetris

A project for trying the GUI on a commercial game: Tetris (USA) menus for both
game types, and a lost game of each.

The ROM is not included. Put your own dump at `build/tetris_usa.nes`
(sha256 `ddb876c3…53082`, checked against `config.toml`). `build/` is
gitignored.

## The tree

- Boot to main menu (640 frames; the legal screen can't be skipped; events mark the title screen and the menu)
  - Cycle through game types; Cycle through music options
  - **Select A-Type** (events: game type chosen, A-type menu entered)
    - Cycle through levels
    - Start A-Type Game at Level 0 › Lose Game immediately and return to menu
  - **Select B-Type** (event: game type chosen)
    - Cycle through levels and heights (the level grid, A, then the height grid)
    - Start B-Type Game at Level 0, Height 0 › Lose Game immediately and return to menu
    - Start B-Type Game at Level 3, Height 3 › Lose Game immediately and return to menu

Each "Start …" segment ends as the game begins, so other ways of playing that game can go
beside "Lose Game". The lost games carry events for the last piece you can move, the piece
that ends the game, the start of the game-over curtain, and the return to the menu
(checkpoints mark where the old segments were joined). In the B-type games those frames come
from RAM: `$48` (play state) is 8 on the frame a piece spawns and 10 (0x0A) during the
curtain, and `$58` counts curtain rows.

In the B-type menu, A moves from the level grid to the height grid. B on the height grid
goes back to the game type screen, not to the level grid.

## Run it

```sh
cd examples/tetris
../../player/target/release/dj check --rom build/tetris_usa.nes --core ../../build/cores/mesen_libretro.dylib
../../gui/target/release/dj-gui .
```

There are no symbols, so coverage reports can't name functions or blocks.

## Why small input changes change the pieces

Traced with `player/examples/ramtrace.rs`, which plays a segment's chain from power-on and
prints RAM bytes per frame:

```sh
cd player && cargo run --release --example ramtrace -- ../examples/tetris <segment id> 17,18,19,BF 20
```

- `$17-$18` is Tetris's random number (a 16-bit shift register). It steps **once every
  frame** from power-on: on the legal screen, in the menus, and in play. Its value at any
  frame doesn't depend on menu input timing.
- A new piece is picked from the random number **at the frame the piece spawns**
  (`$19` is the piece that just spawned, `$BF` the next one). A piece spawns when the
  previous one locks, so its timing depends on every input that changed when pieces
  landed: soft drop, moves and rotations, and also the frame the game started. Gravity
  counts from the game start, so starting one frame later can change a lock frame many
  pieces later.
- Picking a piece sometimes steps the random number again (a re-roll when it would repeat
  the last piece). After that, the random number itself differs between the two runs.

An experiment: pressing START for a B-type game (level 3, height 2) one frame later
leaves the random number and the first several pieces identical. The pieces
first differ about 400 frames into the game, and the random number differs from about
530 frames in.

What this means for projects:

- Any input change before a spawn can change every later piece. That changes the RAM
  hash, so everything below turns dirty. This is correct, but it can surprise.
- A segment can pin the sequence with `[WRITE] RAM 0x17 <hi> <lo>` (or the spawn bytes)
  at a known frame. That makes its gameplay independent of upstream timing, at the cost
  of being a poke.
- Ideas for decomp-dj: per-project watched addresses (here the random number and the
  next piece) drawn as a lane in the piano roll, and after an edit, a marker at the first
  frame a watched value differs from the last run. A game-specific `[RNG]` command
  (reserved in .agents/SPEC.md) could set the random number by name.
