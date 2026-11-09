# Mesen connector

Patches the [libretro Mesen core](https://github.com/libretro/Mesen) (NES/Famicom)
so it reports which code ran, for decomp-dj.

- **Pinned upstream:** `0102910c39ad1a62bc3f784466f3f67ca9eae335` (2026-04-20).
- **Build:** `connectors/mesen/build.sh` clones that commit, applies
  `dj-mesen.patch`, and builds `mesen_libretro.{so,dylib,dll}` into `build/cores/`.
- **License:** Mesen is GPL-3.0, so this patch and the core built from it are
  GPL-3.0 too. The decomp-dj player is a separate program that loads the
  core at run time, like any libretro frontend, and stays MIT.

## Why Mesen

Mesen is one of the most accurate NES emulators and still fast enough to play
segments far faster than real time. Its mapper code already translates a CPU
address into "which byte of PRG ROM / work RAM / save RAM is mapped here"
(`BaseMapper::GetAbsoluteAddressAndType`), which is exactly the key coverage
needs on bank-switched cartridges. The libretro build leaves out Mesen's
debugger, so the patch adds a small hook instead of reusing it.

## What the patch does

- `CPU::MemoryRead` calls `DjCoverage::OnExec` for every opcode fetch and
  operand fetch, and marks the byte's offset in its region.
- `retro_set_environment` registers `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK`,
  which exposes the functions below.

- `retro_load_game` sizes the regions for the loaded game (`DjCoverage::OnLoad`), so
  they can be read right after loading.

Regions, in index order: `RAM` (2 KB internal), `PRG` (PRG ROM), `WRAM`, `SRAM`, all
with `cpu` `"6502"` (connector API version 2). `PRG` hands out the mapper's PRG ROM as
`data` and lies in the ROM file from byte 16 (after the iNES header; 528 with a trainer;
-1 for UNIF and FDS images). `WRAM` and `SRAM` hand out their buffers when the board has
them; `RAM` hands out none. Each byte holds flags: `0x01` executed as an opcode, `0x02`
executed as an operand. Built with this patch, the project's `[core] build` is
`mesen@0102910c+dj2`.

The patch adds one call to every instruction-fetch cycle. On the Thwaite
example it costs about 5% (482 vs 506 frames per second on one x86_64 core,
about 8 times real time), and every segment's state hashes are identical with
and without the patch. Mesen's accuracy is the main cost; a faster NES core
could get its own connector later.
