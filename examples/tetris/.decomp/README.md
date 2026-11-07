# Tetris C decompilation

A behavioural port of Tetris (NES, USA) to C, translated from the annotated disassembly
[CelestialAmber/TetrisNESDisasm](https://github.com/CelestialAmber/TetrisNESDisasm) at
commit `94581c0`, and a libretro core that runs it so decomp-dj can play it like any other
core.

| Path | What it is |
| --- | --- |
| [port/](port/) | The game in C: every routine of the original, named as in the disassembly. Platform-independent; builds to `libtetris.a` |
| [libretro/](libretro/) | One frontend for the port: a libretro core with a small NES hardware model (PPU, MMC1, controllers). Builds to `tetris_libretro.so` / `.dylib` |
| [tools/gen_headers.py](tools/gen_headers.py) | Regenerates `port/src/ram_map.h` and `port/src/rom_labels.h` from a build of the disassembly |
| [tools/ramdiff/](tools/ramdiff/) | Plays segments on two cores and reports the first frames where their RAM differs |
| [tools/dbg2csv.py](tools/dbg2csv.py) | Makes a decomp-dj `csv` symbol map from the reference build |
| [PLAN.md](PLAN.md), [QUESTIONS.md](QUESTIONS.md), [functions.csv](functions.csv) | The plan this follows |

## The port

The port keeps the game's 2 KB of RAM byte for byte as the NES has it (`tetris_ram`), and
every variable lives there at its original address under its disassembly name
(`ram_map.h`). C locals stand in only for the 6502's registers. Scratch bytes the original
leaves behind (`tmp1`, `generalCounter`...) are written the same way, and where the
original hands a result to its caller in a register or a flag, the C function returns it
(the main loop's frame wait depends on the A register that each game mode handler leaves,
for example). Each routine has a comment saying what it does and any quirk it keeps.

The source contains no ROM data. Tables, nametables, sprites and music are read from the
player's own ROM by address (`rom_labels.h`), which the frontend passes in with
`tetris_set_prg()`.

Hardware is reached only through [`port/include/tetris/hw.h`](port/include/tetris/hw.h): register
reads and writes by address, `hw_idle()` when the game waits for the next frame, and
`hw_cycles()` to report long loops (clearing nametables with NMIs off takes more than a
frame on the NES). [`port.h`](port/include/tetris/port.h) is the other side: `tetris_reset()`
runs the game and never returns, and `tetris_nmi()` is the vertical blank interrupt.

| File | Routines |
| --- | --- |
| `system.c` | reset and boot, NMI, main loop, jump tables, PPU copy programs, RNG, controllers, MMC1, frame waits |
| `menus.c` | legal and title screens, game type and level menus, demo input |
| `play.c` | gameModeState and playState handlers: moving, rotating, dropping, locking, line clears, scoring, piece choice, sprites |
| `render.c` | the NMI's drawing during play |
| `highscore.c` | the high score table and name entry |
| `ending.c` | the A-type rocket and B-type endings |
| `audio.c` | sound effects and the music player |

From the repository root:

```sh
make -C examples/tetris/.decomp/port       # .decomp/port/build/libtetris.a
```

## The libretro core

```sh
make -C examples/tetris/.decomp/libretro   # examples/tetris/build/decomp/tetris_libretro.so (.dylib on macOS)
dj play examples/tetris <segment> --core examples/tetris/build/decomp/tetris_libretro.so
```

It accepts only the Tetris (USA) ROM (checked by the PRG's CRC32). Choices made in this
frontend, not in the port:

- The game's main loop runs on its own thread, which takes turns with the frontend's: a
  frame starts the vertical blank (and the NMI, if enabled), then the game runs until it
  waits for the next frame or `hw_cycles()` fills one.
- The picture is drawn from the PPU's state once per frame, which suits a game with no
  mid-frame effects. The palette is computed from the NTSC signal rather than taken from a
  palette file.
- Sound is not emulated: the core outputs silence. The port drives the APU registers as the
  original does, so a frontend with an APU would play it.
- Save states hold the input of every frame since power-on, and loading one replays them
  (a thread's stack can't be saved). This is exact because the port is deterministic, but
  states grow by two bytes a frame and loading takes time in proportion. Reset is a power
  cycle.

## Parity

Not checked yet. `tools/ramdiff` compares the port's RAM with Mesen's frame by frame:

```sh
cargo build --release --manifest-path examples/tetris/.decomp/tools/ramdiff/Cargo.toml
examples/tetris/.decomp/tools/ramdiff/target/release/ramdiff examples/tetris examples/tetris/build/tetris_usa.nes \
    build/cores/mesen_libretro.so examples/tetris/build/decomp/tetris_libretro.so \
    --names examples/tetris/.decomp/port/src/ram_map.h
```

Expect differences in the stack page, which the port does not reproduce (`ramdiff` ignores
`$0100-$01FF`), and possibly in where frame boundaries fall during the long NMI-off loops,
whose cycle counts in `hw_cycles()` are estimates.
