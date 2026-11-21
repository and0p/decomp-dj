# mupen64plus connector

Patches the [libretro Mupen64Plus-Next core](https://github.com/libretro/mupen64plus-libretro-nx)
(Nintendo 64) so it reports which code ran, and so its save states line up with frames,
for decomp-dj. First version: built and tried with WWF No Mercy (below); provenance of RAM
code is still to come
([.agents/_archive/CONNECTOR_PLAN.md](../../.agents/_archive/CONNECTOR_PLAN.md), N64).

- **Pinned upstream:** `15bada85906593376e8fd697f1b0b3af3a20f512` (2026-10-22).
- **Build:** `bash connectors/mupen64plus/build.sh` clones that commit into
  `build/mupen64plus-src`, applies `dj-mupen64plus.patch`, and builds
  `mupen64plus_next_libretro.{so,dylib,dll}` into `build/cores/`. It builds against the
  system's zlib and libpng (`SYSTEM_ZLIB=1 SYSTEM_LIBPNG=1`): the bundled copies take macOS
  for classic Mac OS. On macOS the Makefile builds no dynarec.
- **License:** Mupen64Plus-Next is GPL-2.0, so this patch and the core built from it are
  GPL-2.0 too. The decomp-dj player (GPL-3.0) is a separate program that loads the core at
  run time, like any libretro frontend; the two aren't combined into one work, so the
  licences don't need to match.

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
- **A save state carries the game's save storage.** The core keeps a game's saved data in
  one block, `saved_memory`: EEPROM, the four Controller Paks, SRAM and Flash RAM, 296,960
  bytes in all. libretro hands that block out as save RAM, which dj's `[hash] regions`
  calls `SRAM` whatever chip is behind it. The core's own state holds the storage devices'
  registers but not the block's bytes. So when dj loaded an earlier state, main memory went
  back in time and the save did not: it kept whatever the game had written since.
  - **What that broke.** A project that hashes `SRAM` got other state hashes after a jump
    than from a straight run, until the game next wrote its save, with nothing wrong in
    the game. And a game that reads its save after a jump would read data from its own
    future.
  - **The change.** `retro_serialize_size` reports the core's state plus the block.
    `retro_serialize` copies the block in after the core's state, and `retro_unserialize`
    copies it back once the load has run. The storage devices point into `saved_memory`,
    so nothing else needs telling.
  - **Why here and not in dj.** dj could keep the save beside each snapshot and write it
    back, as it does at power-on. But then every tool that loads a state would have to
    remember to, and a state file on its own would still be incomplete. In the connector,
    a state is whole wherever it goes.
  - **What it costs.** A state is 17,090,372 bytes, up from 16,793,412 (under 2%). The
    build id changes, so a project needs `dj repin`; state hashes from power-on are the
    same as before. A state made before the change is shorter: it loads, and leaves the
    save as it is, which is the old behaviour.
- **Options dj needs are forced** once dj has asked for `dj_api_version`, with a warning
  when they differ from the core options: GLideN64 video, the HLE RSP, no threaded renderer
  (angrylion keeps state outside the save state, so loads don't reproduce), and the pure
  interpreter (the only one that records coverage). `dj_forced_options` lists them, so the
  player records these values in `[core.options]` instead of what the options said.
- **A GLideN64 fix, so the picture shows on macOS.** `BufferedDrawer`
  (`opengl_BufferedDrawer.cpp`) enabled the rectangles' barycentric vertex attribute
  always but gave it a buffer only when coverage is supported, which this fork turns off.
  Apple's OpenGL draws nothing from a vertex array with an enabled attribute and no
  buffer, and raises no error, so every rectangle was dropped: the copy to the screen
  under framebuffer emulation (a black picture), and the game's own fill and texture
  rectangles. The patch enables the attribute only with coverage. It doesn't touch what
  dj hashes.

Regions, in index order (connector API version 3):

| Region | `cpu` | `data` | `file_offset` | Flags |
| --- | --- | --- | --- | --- |
| `RDRAM` (8 MB, by physical address) | `mips3` | none: the core keeps RDRAM in host-endian 32-bit words | -1 | set |
| `ROM` (the cartridge) | none | the ROM in `.z64` (big-endian) order, from the core's ROM buffer | 0 | not set yet |

Code runs from RDRAM on the N64, so that's where coverage lands. Telling which ROM bytes
ran needs the provenance of RAM code (`dj_origin`, later).

**Breakpoints** (`dj_set_breakpoints`) are in `RDRAM` only, on a word boundary. `dj_exec`
tests each instruction's place against them and calls dj at a hit, before the instruction
runs, on the game's coroutine (its stack is 8 MB). No call is made while a save or load
that dj asked for is in progress: the core runs instructions before a load, and those are
not the game's play. dj counts logic ticks at a breakpoint, so `[tick]` works on the N64
now. Other code can sit at the same address in another overlay and is counted too; pick an
address in code that is always loaded. `dj_read_registers` hands out the PC and r0 to r31.

**The trace hook** (`dj_trace_start`, `dj_trace_stop`, `dj_trace_read`) records every
instruction the pure interpreter runs and every byte the CPU reads or writes, through the
four aligned read and write functions in `r4300_core.c`. How the 6502-shaped event is
filled for a 32-bit CPU is in [docs/connectors.md](../../docs/connectors.md#traces). With
no trace running it costs one test per instruction and per memory access; No Mercy plays
at the same speed as before (41 frames a second here).

[stub/](stub/) has a comparison core that uses two breakpoints with WWF No Mercy: one at
the game's input routine and one at its logic tick. It is the test that breakpoints give
the same hits from power-on and from a loaded state.

The core can't save a state before its first frame (it sets up its devices then), so dj
runs one frame at power-on for it, and frame 0 of a root segment starts after that.

Tried 2026-11-09 with WWF No Mercy (USA) (Rev 1): about 4% slower (103 vs 107 fps, pure
interpreter with GLideN64); saves no longer shift the run; loads reproduce (after 0, 50, 100
and 200 frames, and twice in a row); a 300-frame segment's ending hash reproduces from an
empty database; RDRAM flags are word-aligned with `0x02` on the three bytes after each
opcode. Not checked yet: whether state hashes are identical with and without the patch
(unpatched saves run an extra frame, so it takes a run without snapshots).

Tried 2026-11-15, for the save storage, with No Mercy's "CPU vs CPU exhibition" segment
(2,136 frames, where the game writes its Flash RAM at about frame 665). The segment was
played from power-on, then again from a state loaded at its start, and the project's state
hash (`RAM` and `SRAM`) was compared after every frame:

| Core | Frames whose state hash differs | RDRAM alone |
| --- | --- | --- |
| Before the change | 665, frames 0 to 664 | none |
| After | none | none |
| After, loading a state cut to the old length | 665, as before the change | none |

That was with the GPU's copies to RDRAM off. With the project's own settings the recorded
end hash is still reached from power-on, so `dj repin` should pass.

**The GPU's copies to RDRAM are a separate problem, not fixed here.** With
`mupen64plus-EnableCopyColorToRDRAM = "Async"`, RDRAM itself differed on 1,138 frames
between the run from power-on and the run from a loaded state: the GPU's buffers are not in
a save state and the copy is asynchronous. For hashes that hold after a jump, turn
`EnableCopyColorToRDRAM` and `EnableCopyDepthToRDRAM` off.

**Two saves with no frame between them** are also still a problem: the second runs the
game on to its next interrupt before it saves, so it holds a state from inside the frame.
