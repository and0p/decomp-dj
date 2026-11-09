# mupen64plus connector

Patches the [libretro Mupen64Plus-Next core](https://github.com/libretro/mupen64plus-libretro-nx)
(Nintendo 64) so it reports which code ran, and so its save states line up with frames,
for decomp-dj. First version: not built or tried with dj yet
([.agents/CONNECTOR_PLAN.md](../../.agents/CONNECTOR_PLAN.md), N64).

- **Pinned upstream:** `15bada85906593376e8fd697f1b0b3af3a20f512` (2026-10-22).
- **Build:** `bash connectors/mupen64plus/build.sh` clones that commit into
  `build/mupen64plus-src`, applies `dj-mupen64plus.patch`, and builds
  `mupen64plus_next_libretro.{so,dylib,dll}` into `build/cores/`. It builds against the
  system's zlib and libpng (`SYSTEM_ZLIB=1 SYSTEM_LIBPNG=1`): the bundled copies take macOS
  for classic Mac OS. On macOS the Makefile builds no dynarec.
- **License:** Mupen64Plus-Next is GPL-2.0, so this patch and the core built from it are
  GPL-2.0 too. The decomp-dj player is a separate program that loads the core at run time,
  like any libretro frontend, and stays MIT.

## What the patch does

- `libretro/dj_connector.c` (new) holds the regions and the `dj_*` functions;
  `retro_set_environment` registers `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK` for them.
- The pure interpreter (`InterpretOpcode` in `pure_interp.c`) calls `dj_exec` for every
  instruction it executes, delay slots included, with the host pointer it fetched the word
  from; the connector turns that into an RDRAM offset and sets `0x01` on the first byte and
  `0x02` on the other three. Fetches outside RDRAM (the boot code) are ignored.
- **Save states line up with frames.** Unpatched, `retro_serialize` and
  `retro_unserialize` run the emulation until the next vertical interrupt after the job is
  done, so every save or load runs one extra frame. With dj driving the core,
  `gen_interrupt` hands control back (`dj_state_job_done`) right after the save or load
  has run and cleared its job; the next `retro_run` runs exactly one frame. (Not from
  `n64StateCallback`: the core calls it before clearing the job, and twice for a save, so
  a job set meanwhile would be lost.) If the job had to wait for a safe point and frames ran first, the
  call returns false (and logs it), so dj doesn't number frames wrong.
- **Options dj needs are forced** once dj has asked for `dj_api_version`, with a warning
  when they differ from the core options: GLideN64 video, the HLE RSP, no threaded renderer
  (angrylion keeps state outside the save state, so loads don't reproduce), and the pure
  interpreter (the only one that records coverage). `dj_forced_options` lists them, so the
  player records these values in `[core.options]` instead of what the options said.

Regions, in index order (connector API version 2):

| Region | `cpu` | `data` | `file_offset` | Flags |
| --- | --- | --- | --- | --- |
| `RDRAM` (8 MB, by physical address) | `mips3` | none: the core keeps RDRAM in host-endian 32-bit words | -1 | set |
| `ROM` (the cartridge) | none | the ROM in `.z64` (big-endian) order, from the core's ROM buffer | 0 | not set yet |

Code runs from RDRAM on the N64, so that's where coverage lands. Telling which ROM bytes
ran needs the provenance of RAM code (`dj_origin`, later). Ticks aren't counted yet
(`dj_set_tick_exec` is accepted and ignored).

The core can't save a state before its first frame (it sets up its devices then), so dj
runs one frame at power-on for it, and frame 0 of a root segment starts after that.

Tried 2026-11-09 with WWF No Mercy (USA) (Rev 1): about 4% slower (103 vs 107 fps, pure
interpreter with GLideN64); saves no longer shift the run; loads reproduce (after 0, 50, 100
and 200 frames, and twice in a row); a 300-frame segment's ending hash reproduces from an
empty database; RDRAM flags are word-aligned with `0x02` on the three bytes after each
opcode. Not checked yet: whether state hashes are identical with and without the patch
(unpatched saves run an extra frame, so it takes a run without snapshots).
