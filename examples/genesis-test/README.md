# Genesis test ROM

A tiny Mega Drive / Genesis program, built from source here, for trying the Genesis Plus GX
connector and the 68000 decoder end to end. It draws nothing but a background colour that
changes; each button runs different code:

| Code | Runs on |
| --- | --- |
| `start` (start-up: the VDP's registers, clearing RAM, copying `ram_mix` into RAM) | power-on |
| `main`, `wait_vblank`, `read_pad` (3- and 6-button pads), `set_colour`, `vblank` | every frame |
| `on_a`, a jump table of `bra.w` through `jmp table(pc,d0.w)` | A |
| `on_b`, a loop with `dbra` over a table after the function | B, every frame it's held |
| `on_c`, calls by `jsr` absolute, `jsr` PC-relative and `jsr (a1)`, and a word read `(d16,PC)` from after its `rts` | C |
| `on_x` | X (a 6-button pad) |
| `on_direction`, which calls `ram_mix` in work RAM | Left, Right |
| `on_start` | Start (no segment presses it: it stays uncovered) |

## Building

```sh
./build-rom.sh                            # build/genesis-test.bin and build/genesis-test.csv
../../connectors/genesis-plus-gx/build.sh # the core, into build/cores/ at the repository root
```

`build-rom.sh` needs only Python 3: [tools/asm68k.py](tools/asm68k.py) is a small two-pass
68000 assembler, with just the instructions [src/main.s](src/main.s) uses, so no Mega Drive
toolchain is needed. It writes the symbol map too, a csv with one line per `.func` block.

Then set the ROM in `.local/local.toml` (relative to this folder):

```toml
rom = "build/genesis-test.bin"
```

The core needs no setting: `config.toml` names it (`genesis_plus_gx_libretro`), and dj finds it in
`build/cores/`.

and play it: `dj -C examples/genesis-test check`, `dj -C examples/genesis-test summary`.

## The project

Made with `dj init` (which detects the Mega Drive header and picks the core), then the
segments with `dj new` and `dj clean`.

- **Segments:** boot and idle for 60 frames, then press A four times, hold B, press C,
  press X, and press Left then Right. The core gives port 1 a 6-button pad, since the header
  lists one (`J6`).
- **Buttons:** the segments name the RetroPad's buttons, as dj does: `Y` is the pad's A,
  `B` its B, `A` its C, `L` its X ([connectors/genesis-plus-gx](../../connectors/genesis-plus-gx/README.md#for-a-project)).
  `config.toml`'s `labels` show the pad's names in dj-gui.
- **Hashes:** `RAM`, the 68000's 64 KB.
- **Symbols:** `symbols/genesis-test.csv` (`name,ROM,offset,size,cpu_addr`; the CPU address
  is the ROM offset).
- **Coverage:** 152 of 167 instructions (91.0%), and `dj connector-check` finds every flag
  in agreement with the decoder. Not covered: `on_start` and the call to it (Start isn't
  pressed), `read_pad`'s 3-button path, the TMSS unlock in `start` (the core emulates a
  console without one), `exception` (nothing goes wrong), and `ram_mix`, which runs from
  RAM: its 7 instructions are flagged in the connector's `RAM` region, but coverage rolls up
  only `ROM`, so as a function it shows as never entered.

The ROM isn't committed (`config.toml` pins its sha256); the symbol map is. `asm68k.py`
gives the same bytes on every machine.
