# SNES test ROM

A tiny Super Nintendo program, built from source here, for trying the Mesen-S connector and
the 65816 decoder end to end. It draws nothing but the backdrop, whose colour changes; each
button runs different code:

| Code | Where | Runs on |
| --- | --- | --- |
| `reset` (start-up: native mode, a block move, the main loop) | ROM, bank `$00` | power-on |
| `frame`, `show` | ROM, bank `$00`, 8-bit A, 16-bit X | every frame |
| `nmi` | ROM, bank `$00`, 16-bit A | every frame |
| `on_a`, and `fade` it calls | ROM, bank `$01` (`JSL`), 16-bit A with a 16-bit immediate | A |
| `on_b`, with a jump table (`JMP (abs,X)`) and a write to save RAM | ROM, bank `$00` | B |
| `on_start` | ROM | Start (no segment presses it: it stays uncovered) |
| `ram_routine` | WRAM (`$7E2000`), copied there at start-up with `MVN` | Left, Right |

## Building

```sh
./build-rom.sh                     # build/snes-test.sfc and build/snes-test.dbg
../../connectors/mesen-s/build.sh  # the core, into build/cores/ at the repository root
```

`build-rom.sh` builds cc65 (ca65 2.19) into `build/` the first time, and needs git, make, a
C compiler and Python 3 (for the header's checksum). It also builds the same program as
HiROM (`build/snes-test-hirom.sfc`, with its own `.dbg`), for checking the connector's
HiROM mapping; the project plays the LoROM one.

Then play it, from this folder:

```sh
dj check --rom build/snes-test.sfc
dj check --from-power-on --strict
dj connector-check
dj summary --segments
```

(`--rom` is saved in `.local/local.toml`, so later commands don't need it. The core needs no
flag: `config.toml` names it, `mesen-s_libretro`, and dj finds it in `build/cores/`.)

## The project

- **Segments:** boot and idle for 60 frames, then hold A, tap B four times (each entry of
  the jump table), press Right.
- **Hashes:** `RAM` (the 128 KB of WRAM) and `SRAM` (2 KB, which B writes).
- **Ticks:** `[tick] exec = "ROM:0x36"`, the main loop's `JSR frame`: one a frame.
- **Symbols:** `build/snes-test.dbg`, from ld65, with `region = "ROM"`.
- **Coverage:** 105 of 114 instructions (92.1%). Not covered: `on_start` (Start isn't
  pressed), the Start branch in `frame`, the unused-interrupt `RTI`, and `ram_routine`,
  which runs from WRAM: it is flagged there (5 opcode bytes), but coverage rolls up only
  `ROM`, so it shows as never entered.

The ROM isn't committed (`config.toml` pins its sha256); `build-rom.sh` gives the same
bytes with the same ca65. Another version of cc65 could give another ROM: rebuild, update
`[rom] sha256`, and re-record with `dj clean --accept`.
