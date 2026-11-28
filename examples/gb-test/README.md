# Game Boy test ROM

A tiny Game Boy Color program (it runs on a Game Boy too), built from source here, for trying
the SameBoy connector and the SM83 decoder end to end. It draws nothing; each button runs
different code, in a different ROM bank:

| Code | Where | Runs on |
| --- | --- | --- |
| `start`, with the main loop (the project's logic tick, `ROM:0x178`) | bank 0 | power-on, then every frame |
| `wait_vblank`, `read_joypad` | bank 0 | every frame |
| `hram_tick`, copied to HRAM at start-up | HRAM | every frame |
| `press_a` → `on_a`, `a_every_fourth` | bank 0 → bank 1 | A (`a_every_fourth` on every fourth frame A is held) |
| `press_b` → `on_b`: WRAM bank 3, and a counter in battery RAM | bank 0 → bank 2 | B |
| `press_start` → `on_start` | bank 0 → bank 3 | Start (no segment presses it: it stays uncovered) |
| `on_direction`, through a jump table (`jp hl`) to `dir_right`, `dir_left` | bank 0 | Right, Left |

## Building

```sh
./build-rom.py                       # build/gb-test.gbc, and symbols/gb-test.csv
../../connectors/sameboy/build.sh    # the core, into build/cores/ at the repository root
```

`build-rom.py` needs only Python 3: it is a small SM83 assembler for the subset of RGBDS syntax
that [src/gb-test.asm](src/gb-test.asm) uses, and writes the cartridge header (MBC5 with 8 KB
of battery RAM, Color compatible). It always gives the same bytes, so `[rom] sha256` holds on
every machine. `--mapper mbc1|mbc3` and `--dmg` (with `--out`) make variants of the same
program, for trying other mappers and the original Game Boy by hand.

Then set the ROM in `.local/local.toml` (relative to this folder):

```toml
[rom]
path = "build/gb-test.gbc"
```

The core needs no setting: `config.toml` names it (`sameboy_libretro`), and dj finds it in
`build/cores/`.

and play it: `dj -C examples/gb-test check --from-power-on --strict`, `dj -C examples/gb-test summary`.

## The project

- **Segments:** boot and idle for 300 frames (SameBoy's Color boot animation takes about 185),
  then hold A, tap B twice, or press Right and then Left.
- **Hashes:** WRAM (every bank), HRAM through the memory maps, and the battery RAM (see
  [connectors/sameboy](../../connectors/sameboy/README.md#for-a-project)).
- **Tick:** `ROM:0x178`, the top of the main loop: one tick a frame once the program runs.
- **Symbols:** `symbols/gb-test.csv`, written by `build-rom.py`.
- **Coverage:** 107 of 118 instructions (90.7%), 11 of 15 functions. Not covered:
  `press_start` and `on_start` (Start isn't pressed), `dir_none` (never called without a
  direction) and `dir_both` (Right and Left at once). `hram_tick` runs from HRAM: coverage
  rolls up only ROM, so it isn't in the totals. The `nop` and `jp` at the entry point, before
  the header, run outside any function.

The ROM isn't committed (`config.toml` pins its sha256); the symbol map is.
