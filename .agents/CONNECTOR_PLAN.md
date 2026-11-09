# Plan: connectors and decoding beyond the NES

Status: planning notes and hypotheses, not shipped behaviour; the plan was approved to go
ahead on 2026-11-09 (see "Suggested order"). [connectors.md](../docs/connectors.md)
is the source of truth for the current API. Written 2026-11-09 from a look at how more
cores would fit next to Mesen (NES): bsnes-mercury (SNES), mupen64plus-libretro-nx (N64) and
Genesis Plus GX (Sega).

## The baseline

- **The Mesen connector** (`connectors/mesen/dj-mesen.patch`) exports `dj_api_version`,
  `dj_region_count`, `dj_get_region`, `dj_clear`, `dj_set_tick_exec`, `dj_get_tick_count`
  and the trace hooks (`dj_trace_start`, `dj_trace_stop`, `dj_trace_read`). It reports
  per-byte flags, `0x01` opcode and `0x02` operand, on `PRG`, `RAM`, `WRAM` and `SRAM`.
  It doesn't export `dj_sync`, `dj_write`, `dj_check_parity` or `dj_run`: those are for
  comparison cores (a decomp built as a core), and Mesen as a comparison core syncs by save
  state.
- **The player** rolls up only the region named `PRG`, decodes only 6502 (below), and maps
  CPU addresses to ROM offsets itself only for unbanked NES PRG (`FixedPrg`, 32 KB or
  less); otherwise CPU addresses come from the symbol map or `cpu_base`.

## Ground rules so far

- **Bring your own symbols.** dj doesn't find functions. Users supply symbol maps
  ([coverage.md](../docs/coverage.md#symbol-maps)); without one, coverage counts opcode bytes
  that ran and the function queries say how to add a map. The sweeps that used to guess
  routines are gone.
- **Decoding stays, but only inside named functions.** The player splits each function a
  symbol map names into instructions and blocks. That is the only analysis it does.

## What the decoder is today

- It lives in the player (`player/src/analysis.rs`), not in any connector, and is 6502-only:
  `op_len_6502` (lengths), `flow_6502` (next, branch, jump, stop), `analyze_6502_with`
  (instructions and blocks, skipping `[[data]]`), `call_target_6502` (JSR/JMP targets for the
  callers list), `executed_bytes_6502` (operand bytes of instructions that ran).
- The Mesen connector (`connectors/mesen/dj-mesen.patch`) only reports per-byte flags,
  `0x01` opcode and `0x02` operand, plus trace hooks. It decodes nothing for dj.

### What depends on the decoder

Worked through on Tetris. The project was a copy of the example's `config.toml`,
`segments/` (29) and `annotations.toml` into a new folder with an empty database, hydrated
with `dj hydrate` (20,792 frames), once without a symbol map and once with
`build/ref/tetris_symbols.csv` (245 functions). Byte counts come from the PRG coverage the
Mesen connector stored for those 29 segments (`0x01` opcode, `0x02` operand), OR-ed
together. Two runs on 2026-11-09: the first measured opcode bytes only; the second, from
another fresh database, measured operand bytes too, and its other numbers matched the first.

| Metric | With the decoder (what dj shows) | Without the decoder | Source |
| --- | --- | --- | --- |
| Instructions | 3610 / 4679 (77.2%) | no total: unrun code isn't counted | `dj summary` |
| Blocks | 792 / 1011 (78.3%) | none | `dj summary` |
| Functions entered | 207 / 241 (4 left out by annotations.toml) | 207 / 245 (annotations can't drop whole functions) | `dj summary`; computed |
| Opcode bytes run (`0x01`) | 3625 | 3625 | `dj summary` without a map; computed |
| Bytes run (`0x01` or `0x02`) | not shown | 7540 of 10,238 function bytes (73.6%) | computed |
| Functions fully covered by instruction | 163 | 149 of those read 100% of bytes; median 100% | computed |
| Same, counting opcode bytes only | | median 47% | computed |

Per function:

| Function | Bytes | Decoder: instructions run | Opcode bytes | Opcode or operand bytes |
| --- | --- | --- | --- | --- |
| `pollControllerButtons` | 151 | 39 / 74 | 39 | 82 (54%) |
| `highScoreEntryScreen` | 314 | 109 / 145 | 112 | 246 (78%) |
| `nmi` | 69 | 37 / 37 | 37 | 69 (100%) |
| `playState_receiveGarbage` | 80 | all | | 9 (11%) |

So with the operand bits, "bytes run" needs no decoder and a fully run function usually
reads 100%. The exceptions are functions with data or annotated unreachable code inside
(`playState_receiveGarbage`, and jump-table dispatchers such as `branchOnPlayStatePlayer1`, `gameMode_playAndEndingHighScore` and `branchOnGameMode` at 17-26%): the
decoder leaves those bytes out, a byte count can't.

| Feature | Needs the decoder | Why |
| --- | --- | --- |
| Instruction totals ("N of M") | yes | counting what hasn't run means decoding it |
| Blocks: counts, uncovered blocks, successors | yes | block ends are branches and jumps |
| `frontier`, gaps nearness ("try $A2B5") | yes | branch targets |
| Callers list | yes | `JSR`/`JMP` targets at known instruction starts |
| `[[data]]` inside a function | yes | it steers the split |
| `[[unreachable]]` counts, whole-function exclusion | yes | counted per instruction |
| Parity counts | yes | per instruction |
| GUI per-function counts, blocks bar, map colours | yes | built from the above |
| Functions entered | no | any opcode byte in range |
| Bytes run per function | no, with operand bits | `flags & 0x03` |
| `gaps` without a map | today yes (`executed_bytes_6502`) | no, with operand bits |
| `where_runs`, first hits, play tools, fuzzing, goals | no | raw offsets |

### Unused today

The player ignores the `0x02` operand flag and works out operand bytes from 6502 lengths,
although the database keeps it (3915 operand bytes in the Tetris run above).
Using the flag instead (`flags & 0x03`) would make "which bytes ran" exact for any CPU whose
connector sets both bits. Connectors built on a recompiler may set only `0x01` per block
([connectors README](../connectors/README.md)), so the length-based path stays as a
fallback.

## Case study: bsnes-mercury (65816)

Read from libretro/bsnes-mercury master (commit 79d7f9d). Higan fork; profiles accuracy,
balanced and performance in `sfc/Makefile`. The performance profile swaps in `sfc/alt/cpu`
and balanced swaps PPU and DSP, so a patch would pin the accuracy profile only.

**Hook points**

- `CPU::op_step()` (`sfc/cpu/cpu.cpp`) calls `debugger.op_exec(regs.pc.d)` and then fetches
  the opcode with `op_readpc()`.
- `op_readpc()` (`processor/r65816/memory.hpp`) is used only for fetches at PC: the first
  after `op_exec` is the opcode, the rest are operands. Data goes through `op_readdbr`,
  `op_readlong`, `op_readdp` and friends.
- A `debugger` struct (`op_exec`, `op_read`, `op_write`, `op_nmi`, `op_irq` in
  `sfc/cpu/cpu.hpp`) is called unconditionally but never assigned. Interrupt entry reads
  vectors outside `op_step`.
- The libretro wrapper (`target-libretro/libretro.cpp`) doesn't register
  `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK`; the patch would add it. Save states exist.
- `SA1` derives from `Processor::R65816`, a second 65816 sharing the same fetch helpers.
- Profiles: the patch would pin the accuracy profile only (above).

**CPU mode**

At `op_exec`, `regs.e`, `regs.p.m` and `regs.p.x` are known. The core runs a different
opcode table per mode (`op_table[5*256]`, `processor/r65816/table.cpp`). Immediate operand
length depends on M (accumulator) and X (index); E forces 8-bit.

**The built-in disassembler isn't reusable**

`R65816::disassemble_opcode()` (`processor/r65816/disassembler.cpp`) only `sprintf`s text.
It takes M and X from live registers, and `decode()` computes relative branches
(`OPTYPE_RELB`/`RELW`) from the live `regs.pc`, so it's right only for the instruction at the
current PC. No length or flow output, and `dreadb` skips MMIO.

**Address mapping**

- `Bus::map()` (`sfc/memory/memory.cpp`) fills `lookup[16M]` (reader id) and `target[16M]`
  (offset with mirror and mask applied), plus fast-page pointers (`fast_read`).
- Plain LoROM/HiROM cartridges map ROM in `sfc/cartridge/markup.cpp` with
  `fastptr = rom.data()`. So ROM offset = fast-page pointer minus `rom.data()`, or
  `target[addr]` when the reader is the ROM. About a dozen lines in a connector.
- `SA1::mmcrom_read()` (`sfc/chip/sa1/memory/memory.cpp`) picks ROM banks from run-time
  registers (`mmio.cb`/`db`/`eb`/`fb` and their mode bits) and can replace the vectors with
  registers. SPC7110 and S-DD1 are similar. CPU address to ROM offset depends on the moment,
  and only the core knows it.

**Other SNES specifics**

- Code copied to WRAM and run there is common: flags on a `WRAM` region show it, but it
  can't roll up into ROM functions.
- Coprocessors each have their own CPU and region: SA-1 (65816), SuperFX/GSU (own ISA, runs
  from ROM), SPC700 (audio RAM), DSP-n (internal program ROM). Start with the main CPU,
  then the SA-1.
- Trace events today have a 16-bit `addr` and 8-bit registers; the 65816 needs 24-bit
  addresses and 16-bit A/X/Y/S/D, plus DB and E.
- Symbol maps: ca65 `.dbg` already works; WLA-DX `.sym`, asar and bass would be new
  formats. A 512-byte copier header is covered by `rom_header`.
- Needs a `[cache]` preset from measured save-state sizes. Accuracy is the slowest profile;
  hydrate and fuzz speed not measured.

**Not checked yet** (raised while planning, not read in the source):

- Whether `retro_serialize` round-trips are deterministic. bsnes-era cores serialize at a
  synchronisation point (higan's `runtosave()`), which may run the emulation a little
  before saving; that could break "same state, same hash" and seeking.
- How WRAM is initialised at power-on (a fixed pattern or a seeded random fill) and whether
  it's the same every run, which hashing needs.
- Whether cheats apply inside `Bus::read` (it checks `cheat.enable()` there), and that
  nothing enables them during play.
- The licence (GPL-3 was mentioned) and what that means for shipping a patch and build
  script, as with Mesen.

## Case study: mupen64plus-libretro-nx (N64)

Read from libretro/mupen64plus-libretro-nx master (commit 15bada8). GPL-2 (`LICENSE`). The
core is mupen64plus-core plus RSP and RDP plugins, built into one libretro core.

**CPU cores** (core option `mupen64plus-cpucore`, `libretro/libretro_core_options.h`)

- `pure_interpreter`, `cached_interpreter` and, when built with `DYNAREC`,
  `dynamic_recompiler`, which is then the default (`cached_interpreter` otherwise). Mapped to
  `r4300_emumode` in `libretro/libretro.c`.
- Pure interpreter: `InterpretOpcode()` in `mupen64plus-core/src/device/r4300/pure_interp.c`
  fetches each instruction with `fast_mem_access(r4300, pc)`. Jumps (`DECLARE_JUMP`) run the
  delay slot by calling `InterpretOpcode` again with `delay_slot=1`, then set the PC. One
  hook there sees every executed instruction, delay slots included.
- Cached interpreter (`cached_interp.c`) decodes blocks into `ops` function pointers and runs
  them through `(*r4300_pc_struct(r4300))->ops()`; both interpreters already call
  `update_debugger(pc)` per instruction when `g_DebuggerActive` (the `DBG` build). A coverage
  hook fits in the same places.
- new_dynarec (`new_dynarec/new_dynarec.c`, x86/x64/arm/arm64 back ends) compiles blocks to
  native code. Per-instruction coverage would mean emitting instrumentation into compiled
  code; marking at compile time records what was compiled, not what ran. So a dj connector
  would pin an interpreter, and `[core.options]` (which dj records whole) pins it per project.
  Dynarec and interpreter timing also differ, so they are different setups anyway.
- `fast_mem_access()` (`r4300_core.c`) translates TLB-mapped addresses with
  `virtual_to_physical_address()` unless the address is in KSEG0/KSEG1 (`0x80000000`-
  `0xBFFFFFFF`), then masks to a physical address. A connector gets the physical address
  of each fetch for free.

**Where code runs from: the main difference from NES and SNES**

- Code runs from RDRAM, not from the cartridge. The game copies it with PI DMA:
  `dma_pi_write()` in `mupen64plus-core/src/device/rcp/pi/pi_controller.c` (cartridge to
  RDRAM, through `cart_rom_dma_write()` in `device/cart/cart_rom.c`, which knows
  `cart_addr`, `dram_addr` and `length`). Overlays are loaded and unloaded at run time and
  share virtual addresses.
- So "which ROM offset ran" is a provenance question, not a bus mapping. A connector can keep
  a provenance table, one ROM offset (or "unknown") per RDRAM word, set by cartridge DMA and
  cleared when the CPU, RSP or other DMA writes the word (stores already call
  `invalidate_r4300_cached_code()`, which marks the spots). On execution it flags the RDRAM
  byte, and also the ROM byte when provenance is known.
- That works for code DMA'd straight from ROM. It fails for code the CPU decompresses or
  relocates: compressed overlays (Yaz0/MIO0 in many retail ROMs, and decomps that match
  them) and relocated overlays, whose instruction words differ from the ROM. Taint-tracking
  every CPU copy would be too slow. That code can only be covered by RDRAM or virtual
  address, and attributed to a segment some other way.
- Decomp symbol maps address code by virtual address (VRAM): splat's `symbol_addrs.txt`
  gives `vram` plus `rom:` per symbol, linker maps and ELFs give VRAM with a load address.
  Overlays share VRAM, so a VRAM address alone doesn't say which overlay ran. Rolling up by
  VRAM needs "which segment occupied this RDRAM at the time": the connector's provenance
  (for DMA'd code) or a match of the RDRAM bytes against the decomp's built segments.

**The virtual-address problem, plainly**

N64 code lives in the ROM but runs from RDRAM, copied there by the game. There are three
addresses for the same instruction: its ROM offset, its RDRAM (physical) address, and its
virtual address (VRAM, what symbol maps use; usually `0x80000000` plus the physical address,
or TLB-mapped).

- **The main segment** is copied once at boot to a fixed place and stays there. Its VRAM is
  a stable name for its code, so coverage by VRAM and a VRAM symbol map work as they do for
  ROM offsets on the NES.
- **Overlays** are loaded on demand into VRAM ranges they share with other overlays. The same
  RDRAM address holds different code at different times, so flags keyed by RDRAM offset alone
  merge them: coverage of one overlay gets credited to whichever overlay the symbol map lists
  at that address.
- **Relocated overlays** (common in games with many small loadable modules, such as actor
  overlays) are linked at one VRAM and loaded at another, chosen at run time. Their code
  doesn't even sit at its symbol map's address.

So coverage tracking itself still works (the flags record what ran); what breaks is
attributing overlaid code to the right function. Until `dj_origin` or another way to tell
overlays apart exists, coverage of overlaid code is approximate (merged) and should be marked
so; the main segment is exact.

**MIPS decoding**

- Fixed 4-byte instructions: lengths and mode are not a problem. A connector sets `0x01` on
  the first byte of each word and `0x02` on the other three, as the API already says.
- Branch delay slots: a block ends after the slot, and "likely" branches run the slot only
  when taken. `J`/`JAL` targets are 26 bits within the current 256 MB VRAM region, so they
  resolve to VRAM, and to ROM only through the symbol map's segments. `JR $ra` returns;
  other `JR`s are jump tables (splat labels them `jtbl_`).
- A MIPS decoder in the player is small (about 150-200 lines). The hard part for N64 is
  addressing, not decoding.

**RSP**

- A second CPU (MIPS-like vector unit) running microcode from its 4 KB IMEM, loaded by SP DMA.
  Option `mupen64plus-rsp-plugin`: `hle` (default, no microcode runs), `parallel` or
  `cxd4` (LLE). Microcode is mostly Nintendo's libultra ucode, rarely what a decomp targets.
  If wanted later: an `IMEM` region with `cpu = "rsp"`, under LLE only.

**Libretro**

- `retro_get_memory_data`: `SYSTEM_RAM` is `g_dev.rdram.dram` (size `RDRAM_MAX_SIZE`,
  8 MB with the Expansion Pak), `SAVE_RAM` the save memory.
- No `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK`; the patch adds it.
- Video: `mupen64plus-rdp-plugin` is `gliden64` by default (OpenGL HW_RENDER, which dj
  supports), `parallel` (Vulkan, which dj doesn't support yet) or `angrylion` (software).
  `ThreadedRenderer` is an option for GLideN64. Hashes cover RAM, not video, but a threaded
  renderer can still affect timing.
- Save states are deferred: `retro_serialize()` sets a job (`savestates_set_job`) and
  `co_switch`es to the emulation thread until `savestates_save()` runs from the interrupt
  handler (`device/r4300/interrupt.c`) at a safe point. So a state is taken a little after
  the call, not at the frame boundary, and loads are deferred the same way. Same risk as
  bsnes's `runtosave()`.
- Timing options that change results: `CountPerOp` and `CountPerOpDenomPot`; dj records
  them with the rest of `[core.options]`.

**Not checked yet**

- Whether serialize then unserialize then run gives the same hashes as running on (the
  deferred save point), and whether dj's snapshot timing needs the connector to save at a
  fixed point instead.
- Whether the cached interpreter is deterministic across runs and machines, and its speed
  for hydrate and fuzz; whether angrylion should be the default for dj projects.
- Which RSP mode and video plugin keep RAM hashes stable.
- How much code real decomps (SM64, OoT, Paper Mario) run that was decompressed or
  relocated, i.e. how much provenance covers.
- What `DBG` builds add (`mupen64plus-core/src/debugger/`, breakpoints and memory hooks)
  and whether a connector should reuse them.

## Case study: Genesis Plus GX (Mega Drive, Master System, Game Gear, Mega CD)

Read from libretro/Genesis-Plus-GX master (commit f27655b). One core for several Sega
systems: Mega Drive/Genesis, Master System, Game Gear, SG-1000, Mega CD/Sega CD and Pico.

**CPUs and hooks**

- 68000: Musashi-derived (`core/m68k/m68kcpu.c`, `m68kcpu.h`). The Mega CD's second 68000
  is the same code built again (`s68kcpu.c` includes `m68kcpu.h` with `s68kconf.h`). Z80 in
  `core/z80/z80.c` (the Mega Drive's sound CPU, and the main CPU of the 8-bit systems).
- A hook already exists: `core/debug/cpuhook.h` defines `cpu_hook` with `HOOK_M68K_E`
  (execute), `HOOK_M68K_R`/`W`, VDP and register events, and `HOOK_Z80_E`/`R`/`W`.
  `m68k_run()` calls `cpu_hook(HOOK_M68K_E, 0, REG_PC, 0)` before each opcode fetch
  (`m68ki_read_imm_16()`), under `#ifdef HOOK_CPU`; `Makefile.libretro` sets `HOOK_CPU = 0`.
  I found no `cpu_hook` call in `z80.c` or in the sub-CPU's run loop, so the Z80 and the
  Mega CD's second 68000 would need hooks added.
- Extension words are fetched with `m68ki_read_imm_16`/`_32` at PC, so the bsnes split
  works: the fetch after the execute hook is the opcode word, later immediate fetches in
  that instruction are operands (`0x02`).
- No disassembler ships in this tree (Musashi's `m68kdasm.c` isn't here), so a decode call
  in the connector (option b) would again mean writing one.

**Address mapping**

- `m68k.memory_map[64 KB bank].base` points into `cart.rom` for ROM banks
  (`core/cart_hw/md_cart.c`), so ROM offset = `base + (addr & 0xffff) - cart.rom`. Plain
  cartridges up to 4 MB map linearly from `$000000`, so the CPU address is the ROM offset.
  Bank-switching boards (the SSF2 mapper and others in `md_cart.c`) only change `base`, so
  the same lookup stays right at run time. Simpler than the SNES; nothing like the N64.
- On a Mega Drive the Z80 runs code the 68000 uploads into Z80 RAM: provenance again, but
  it's sound driver code, rarely what a decomp targets.
- The Mega CD runs from PRG-RAM and Word RAM loaded from the disc: no ROM at all. Coverage
  would be by RAM region, with provenance from disc sectors if wanted (`dj_origin` with a
  disc region).

**ROM layout**

`core/loadrom.c` de-interleaves `.smd` dumps (`deinterleave_block`, 16 KB blocks after a
512-byte header), and on little-endian hosts (`LSB_FIRST`) keeps the ROM byte-swapped within
each 16-bit word. A connector computing offsets from `cart.rom` has to XOR them with 1 on
those hosts. This is the region-layout rule in practice: canonical is the headerless,
de-interleaved, big-endian ROM, and the player must normalise `.smd` files the same way.

**Libretro**

- `retro_get_memory_data`: `SYSTEM_RAM` is `work_ram` (68K RAM), `SAVE_RAM` the SRAM when
  on. The core also sets `RETRO_ENVIRONMENT_SET_MEMORY_MAPS`.
- No `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK`; the patch adds it.
- Save states are synchronous: `retro_serialize()` calls `state_save()` directly, as with
  Mesen. No threads in the libretro wrapper. Video is software.
- Licence: its own non-commercial licence (`LICENSE.txt`): redistributions may not be sold
  or used commercially, and modified versions must ship complete source. Not the GPL, and
  not GPL-compatible. A dj connector patch, and any core built from it, would be under it;
  dj itself stays a separate program, as with Mesen. Worth deciding before writing one.

**Decoding**

- 68000: lengths are 2 to 10 bytes, fixed by the opcode word and its addressing modes
  (extension words), with no run-time mode. Branches are PC-relative, `JSR`/`JMP` absolute
  (24-bit), `RTS`/`RTE`/`RTR` return, `JMP (An)` and `JMP (d,PC,Xn)` are tables. The decoder
  is mostly the effective-address table: perhaps 300 lines, or a crate if a suitable one
  exists (not checked).
- Z80: lengths 1 to 4 bytes from prefixes (`CB`, `DD`, `ED`, `FD`, `DDCB`), no mode.
- The 68000 also covers the Mega CD, Neo Geo, Amiga and Atari ST; the Z80 the Master
  System, Game Gear, SG-1000, MSX and others. Other cores for these systems (PicoDrive,
  BlastEm) would share the decoders and get the conformance check.

**Not checked yet**

- What enabling `HOOK_CPU` (or a direct call instead) costs; how fast hydrate runs.
- Whether a Rust 68000 disassembler crate gives lengths and flow kinds we could use.
- Whether a region can be run by two instruction sets (the Z80 can reach the 68000 bus
  through its bank window; executing from it is rare), which one `cpu` per region can't say.
- How the 8-bit systems' mappers (Sega, Codemasters) in `sms_cart.c` map ROM.
- Mega CD symbol maps: whether disassemblies name code by RAM address, and how a disc-based
  region would be keyed.

## The cores side by side

| | Mesen (NES) | bsnes-mercury (SNES) | mupen64plus-libretro-nx (N64) | Genesis Plus GX (Sega) |
| --- | --- | --- | --- | --- |
| CPU | 6502 | 65816 (plus SA-1, SPC700, GSU, DSPs) | MIPS R4300 (plus RSP) | 68000 and Z80 (plus a second 68000 on Mega CD; Z80 alone on 8-bit systems) |
| Connector today | yes (`connectors/mesen`) | none | none | none |
| Proc address callback | added by the patch | not registered | not registered | not registered |
| Execution hook | patch in the CPU core | `CPU::op_step` / `op_readpc` | `InterpretOpcode` / cached `ops()`; not the dynarec | `cpu_hook(HOOK_M68K_E)` in `m68k_run` (off by default); Z80 and sub-CPU need adding |
| Instruction length | 1-3 bytes by opcode | 1-4 bytes, depends on M/X/E | always 4 | 68000: 2-10 bytes, static; Z80: 1-4, static |
| Code runs from | ROM (PRG) | ROM, often WRAM too | RDRAM (copied from ROM by DMA) | ROM (Mega Drive); Z80 RAM; RAM from disc (Mega CD) |
| CPU address to ROM | mapper (`GetAbsoluteAddressAndType`) | `Bus` tables; SA-1 etc. remap at run time | provenance of DMA'd words; lost for decompressed code | `memory_map[bank].base - cart.rom`; linear for plain carts |
| Symbol maps address by | ROM offset / CPU address | ROM offset / 24-bit address | VRAM (overlays share it) | CPU address = ROM offset for plain carts |
| Save states | taken at the call (dj relies on it today) | not checked (`runtosave()` risk) | deferred to a safe point | synchronous (`state_save`) |
| Video | software | software | GLideN64 GL (default), ParaLLEl Vulkan, angrylion software | software |
| Profiles / modes to pin | none | accuracy profile | an interpreter `cpucore` | none found |
| Licence | GPL-3 | not checked | GPL-2 | non-commercial, not GPL-compatible |

## What a 65816 decoder needs that the 6502 one doesn't

- **The mode.** For code that ran, the connector can record it exactly. For code that
  hasn't, propagate it from covered predecessors (a covered branch's mode carries to its
  target), then through `REP`/`SEP`/`XCE` while sweeping a function; `PLP` makes it unknown.
  A function never entered has no starting mode: assume M=1 X=1 and show its counts as
  approximate.
- **Targets.** Relative branches (`Bcc`, `BRA`, `BRL`) resolve within the function as now.
  `JSR`/`JMP abs` use the function's program bank. `JSL`/`JML` are 24-bit and need the
  cartridge's mapping (LoROM, HiROM, ExHiROM, SA-1) to become ROM offsets.
  `JMP (abs,X)` tables stay unresolved, as on the 6502.
- **More flow kinds:** `RTL`, `JSL`, `JML`, `BRL`, `COP`, `WDM`, `STP`, `WAI`; block moves
  `MVN`/`MVP` are 3 bytes.
- Size: about 300-400 lines of Rust with mode propagation, against about 150 for the 6502.

## Where decoding could live

| Option | For | Against |
| --- | --- | --- |
| (a) A Rust decoder per CPU in the player | Once per CPU, shared by every core for that console (Mesen, FCEUmm; bsnes, Snes9x). Testable. Patches stay small. | Doesn't know the run-time mode or the mapping by itself. |
| (b) The connector exports a decode call | Knows the mapping. | A decoder per core, not per CPU: the same opcode tables re-implemented in each patch and rebased with it. bsnes's disassembler can't be reused (above). See "Per CPU or per core?" below. |
| (c) Richer flags only, no decoder | Exact for code that ran, a few lines in `op_readpc` plus mode bits at `op_exec`. | Nothing for code that hasn't run: no totals, blocks or callers. |
| (d) Hybrid of (a) and (c), plus a mapping call | Exact where it ran, approximate (and marked) where it hasn't, per-CPU code shared across cores. | Two pieces to keep in step. |

The CPU address to ROM offset mapping belongs in the connector: only the core knows the bus
layout and the chips' bank registers (SNES), or which DMA filled a word of RAM (N64).
`FixedPrg` won't stretch to `JSL` on the SNES or to overlays on the N64.

The N64 doesn't change the split: decoding MIPS is easy, and (d) still holds. What it adds
is that coverage may live in RAM, not ROM, so regions, symbol maps and the mapping call have
to allow addresses other than ROM offsets.

### Per CPU or per core?

Option (b) amounts to a decoder per core. The question is whether one decoder per CPU really
works across the cores of a system, or whether each core ends up needing its own anyway.
Per CPU looks safe, because the decoder depends on very little that a core decides:

| The decoder depends on | Same across cores? | Why |
| --- | --- | --- |
| Instruction bytes | yes | read from the ROM file the player loads (`need_prg` in `query.rs`), not from the core |
| Lengths, flow kinds, relative and absolute targets | yes | fixed by the CPU; cores differ in timing and quirks, which change what runs, not how bytes decode (the 6502's `JMP ($xxFF)` bug changes the target of an indirect jump, which the decoder never resolves) |
| Function bounds and names | yes | from the symbol map, per game |
| Which bytes ran, CPU mode (65816 M/X) | no | from the connector's flags |
| CPU address to ROM offset, provenance | no | from the connector (`dj_map_address`, `dj_origin`) |
| What a region offset means (header, byte order, interleave) | must be made the same | the connector contract, below |

What varies by core is exactly what (d) gives the connector. A decoder per core would
re-implement the same opcode table in each patch, in C or C++, each to rebase against its
upstream. "Per CPU" really means per instruction set, with variant tables:

| Instruction set | Systems | Variants the decoder needs |
| --- | --- | --- |
| 6502 | NES (2A03), Atari 2600 (6507), C64 (6510), PC Engine (HuC6280) | NES illegal opcodes (games use some); 2A03 has no decimal mode (behaviour, not length); HuC6280 adds opcodes (block transfers, `TAM`/`TMA`, ...) |
| 65816 | SNES, SA-1 | M/X/E mode from the connector |
| MIPS | N64 (MIPS III), PS1 (MIPS I) | instruction set level; delay slots in both |
| ARM | GBA (ARM7), DS (ARM9 and ARM7) | ARM/Thumb is a mode bit, like the 65816's M/X |

Where per CPU could break is the connector contract, not the decoding:

- **Region layout.** Two cores must mean the same thing by an offset. NES: `PRG` offset 0 is
  the first byte after the iNES header. SNES: dumps may carry a 512-byte copier header or be
  interleaved (some HiROM dumps). N64: ROMs come big-endian (`.z64`), byte-swapped (`.v64`)
  or word-swapped (`.n64`). Mega Drive: `.smd` dumps are interleaved behind a 512-byte header,
  and Genesis Plus GX keeps the ROM byte-swapped in memory on little-endian hosts, so its
  connector must XOR offsets with 1. connectors.md should pin a canonical layout per system (header
  stripped, de-interleaved, big-endian for the N64), and the player should normalise the
  file it decodes the same way.
- **Cores that set only `0x01`** (recompilers marking block starts) give the decoder less to
  check against; the decoding is the same.
- **CPU variants within a system** (the NES's 2A03 is a 6502 without decimal mode) change
  behaviour, rarely lengths. Where lengths differ, that's an option on the CPU's decoder.

A decoder per core would only be right if a core couldn't report operand bits or modes and
its internal decode tables were the only way to fill them in. bsnes's tables aren't usable
from outside anyway (`op_table` holds handlers, not lengths; its disassembler is text-only).
Mesen may be the exception: its debugger disassembles with lengths and is used by its own
UI, so a decode call could wrap it. Not checked, and it would still only help one core.

**Conformance check.** It would be needed with per-core decoders too: it validates the
offsets and flags a connector reports, not only the decoding, so moving decoders into
connectors wouldn't remove it. Instead of trusting this, check each new connector: after
hydrating a project, every byte the core flagged `0x01` must be an instruction start the
decoder agrees with, and the `0x02` bytes after it must match the decoded length (no `0x02`
on a byte the decoder calls an opcode, and the reverse). Run within symbol-map functions
(code by the user's say-so) or over all flagged bytes. A mismatch means the core lays out
offsets or sets flags differently. About 20 lines in the player (say `dj check-connector`).
Run it for every new connector or core build, and for a second core of a known system (FCEUmm
against Mesen on the same project). Mesen is expected to pass but it hasn't been run: in the
Tetris run, opcode and operand bits never overlapped on a byte (3625 opcode, 3915 operand),
which is consistent, not a check against decoded lengths.

## Leaning: (d)

1. **The connector reports what ran exactly:** opcode and operand bits, plus the CPU mode on
   each opcode byte. The player uses operand bits for executed bytes, gaps ranges and covered
   instruction counts on any CPU, with the length-based path as a fallback.
2. **The player keeps one small decoder per CPU** for lengths, flow and relative targets,
   used to split named functions into blocks and count what hasn't run. Its starting mode
   comes from the flags where code ran; anything else is approximate.
3. **The connector answers address questions** from the core's mapping: for long calls and
   jumps (callers, frontier) and for symbol maps that only give CPU addresses (what
   `cpu_base` handles today). On the N64 the question is provenance: which ROM offset an
   executed RAM word came from.
4. **Coverage can be reported in RAM regions too** (`RDRAM` on the N64, `WRAM` on the SNES),
   and symbol maps can name functions by virtual address in such a region. Rolling RAM
   coverage up into a decomp's segments needs provenance or a byte match.

## Convention over configuration

Agreed 2026-11-09. Connectors and cores are assumed stable; when one changes, a project
migration and a cache rebuild handle it, not configuration that bridges the gap. So the
connector provides everything about how the core works, by convention, and config keeps only
what a connector can't know. No per-system "profiles" for connecting or decoding.

**Stays in config**

| Item | Why the connector can't provide it | Kind |
| --- | --- | --- |
| ROM path, core path | about this machine | local.toml |
| ROM `sha256`, core `build` | which setup the hashes belong to; a connector can't vouch for itself | record, written by `dj init` |
| Core options in use | a choice, and a core's defaults can change between builds | record; defaults from the connector |
| Hash regions | define what a segment's hashes mean; changing them is a migration | record; default from the connector (system RAM) |
| Ports and devices | what the game needs | record; default from libretro's controller info |
| Symbol maps (files, format) | the user's decomp | intent |
| `[tick]`, `[[compare]]` entries | game- and user-specific | intent |
| Core options the game needs (region, expansion pak, ...) | about the game | intent, recorded with the rest of `[core.options]` |
| annotations.toml | knowledge about the game | intent |
| Machine preferences (database, memory budget, bindings, MCP) | about the user | local.toml |
| A debugging override of a connector fact | for working on a connector | local.toml only, with a warning whenever it's in effect |

**Moves to the connector (or to dj measuring)**

- Region names, sizes, bytes and instruction sets: the region list.
- ROM layout and headers: each region says where it lies in the ROM file, so symbol formats
  with file offsets convert by themselves; `rom_header` becomes a rare override.
- `[[symbols]] cpu_base` and `region`: resolved through `dj_map_address` and the regions;
  rare overrides.
- `[[compare]] sync`: inferred from whether `dj_sync` is exported; save states otherwise.
- Options that break determinism (the N64 dynarec): the patch forces what dj needs while the
  connector is active, instead of a config warning.
- `[cache]` presets: dj measures save-state size and change per frame on the first run and
  picks page size and snapshot interval; a local override stays for tuning.

**Caveats**

- "Convention" still means recording: whatever defines the hashes (build, options, hash
  regions, devices) is filled in by `dj init` from the connector and written to config.toml,
  so it shows in diffs and another machine can't silently differ. Hand edits are overrides.
- Conventions are per system, not per connector: a second core for the same system must
  report the same region names and layout, or stored coverage, annotations and symbol maps
  stop matching. That's a contract in connectors.md (the conformance check can help verify it).
- Third-party comparison cores (a decomp built as a core) must follow the same documented
  convention for the regions they report.
- Cores without a connector (plain libretro cores, coverage off) get their defaults from
  plain libretro (system RAM, controller info), with nothing system-specific.
- What a connector reports at run time is harder to see than a file: `dj init` and
  `dj status` should print it.
- A symbol map the connector can't relate to the ROM (another dump, a hand-made csv, N64
  overlays by VRAM) keeps a per-map override. Specified only when needed.

### Decisions so far

1. **Mesen gets rebuilt for API 2**, no compatibility layer for API 1: dj is pre-alpha with
   no outside contracts. Projects re-clean; segment files keep their RAM hashes.
2. **Region names are fixed for a project's lifetime** (pinned ROM, system and core build).
   The NES keeps `PRG`. Names are a per-system contract in connectors.md.
3. **A symbol map's region is inferred** where possible (from the format, and addresses
   through the connector's mapping), and specified only as an override. The NES doesn't need
   it.
4. **The connector declares as many (region, instruction set) entries as the core needs**;
   the `cpu` string picks the decoder. The GUI and reports default to the regions that have
   a symbol map, so an NES project looks as it does now.
5. **The decoder fallback stays** for regions with no operand flags (cores that set only
   `0x01`).
6. **Convention over configuration**, as above.
7. **The N64 (mupen64plus-libretro-nx) is the next connector**, after the Mesen refactor.
8. **Save-state round-trip determinism is the first N64 experiment**, since its saves are
   deferred to a safe point.
9. **Several regions in the GUI:** shown appended as one large block (map and totals). A
   dropdown to pick a region is in the roadmap (GUI section).
10. **RAM-resident code (WRAM and the like):** inferred when it can be inferred with
    certainty; otherwise the symbol map names the region.
11. **A region run by two instruction sets:** handled when the Mega Drive is added.
12. **Licences (Genesis Plus GX):** handled when those systems are added: use another core,
    or leave users to build their own connector.
13. **N64 provenance: on the back burner.** Start with coverage by VRAM in an `RDRAM`
    region, which works for code that isn't overlaid (the main segment, say); `dj_origin`
    only once overlays need telling apart. This is about the baseline core's coverage, not
    parity: comparison cores sync through `dj_sync` or save states and need no mapping. The
    option of the player matching RDRAM bytes against the decomp's built segments is
    dropped for now.
14. **An opcode run in both widths:** split the block there and count it once, marked
    ambiguous; all mode bits stored.
15. **Mode bits:** generic bits `0x04`-`0x20` defined per instruction set, with per-CPU
    named aliases in the header and a table in connectors.md.
16. **Conformance is a contract, not a required step.** Connectors shipped with dj are
    assumed compatible with the decoders shipped with them. The contract in connectors.md
    is what third-party connector authors must follow. The conformance check becomes an
    optional developer and debugging tool (`dj check-connector` or similar), for writing a
    new connector, after bumping a connector's upstream, or for third parties verifying
    theirs. Users don't run it.

### Draft API changes (not in connectors.md)

```c
/* Flag bits, one byte per region byte. 0x01 and 0x02 as today. */
#define DJ_FLAG_OPCODE   0x01  /* first byte of an instruction that ran */
#define DJ_FLAG_OPERAND  0x02  /* other bytes of an instruction that ran */
/* On opcode bytes only, CPU-specific. For the 65816, the width each time it ran, so an
   opcode run in both modes keeps both bits: */
#define DJ_FLAG_A8       0x04  /* ran with an 8-bit accumulator (M=1 or E=1) */
#define DJ_FLAG_A16      0x08  /* ran with a 16-bit accumulator */
#define DJ_FLAG_X8       0x10  /* ran with 8-bit index registers (X=1 or E=1) */
#define DJ_FLAG_X16      0x20  /* ran with 16-bit index registers */
/* 0x40-0x80 reserved. */

struct dj_region {
    const char    *name;        /* "PRG", "ROM", "RDRAM", "WRAM", ... */
    uint8_t       *flags;
    size_t         size;
    /* API 2: */
    const char    *cpu;         /* "6502", "65816", "mips", "m68000", ... or NULL for data */
    const uint8_t *data;        /* the region's bytes as the CPU sees them (natural byte
                                   order), for decoding; NULL for RAM the player shouldn't
                                   read this way */
    int64_t        file_offset; /* where offset 0 lies in the ROM file the project pins
                                   (16 for NES PRG after the iNES header), -1 if the region
                                   isn't from the ROM file */
};

/* API 2, optional: where a CPU address points right now. cpu as in dj_region ("sa1" for
   the SA-1). False when it isn't in a reported region (MMIO, open bus). For the N64 the
   address is virtual; it resolves to RDRAM (through the TLB if mapped). */
bool dj_map_address(const char *cpu, uint32_t address, int *region, int64_t *offset);

/* API 2, optional: where the bytes at a region offset came from, for regions code is copied
   into (RDRAM, WRAM). False when unknown (written by the CPU, decompressed, never loaded). */
bool dj_origin(int region, int64_t offset, int *from_region, int64_t *from_offset);
```

- Regions come from the connector per address space: `ROM`/`PRG` (cartridge), `RDRAM` or
  `WRAM` (RAM that code runs from), `IMEM` (RSP). A connector that knows provenance may
  also set flags on the ROM region when RAM code from a known ROM offset runs.
- Trace events v2 also need a delay-slot bit for MIPS, and 32-bit (or 64-bit) registers;
  probably a variable record with a CPU-specific register block rather than one struct.

| Bit | Name | Set on | Meaning |
| --- | --- | --- | --- |
| `0x01` | `DJ_FLAG_OPCODE` | first byte of an instruction | ran (today) |
| `0x02` | `DJ_FLAG_OPERAND` | other bytes of an instruction | ran (today) |
| `0x04` | `DJ_FLAG_A8` | 65816 opcode bytes | ran with an 8-bit accumulator |
| `0x08` | `DJ_FLAG_A16` | 65816 opcode bytes | ran with a 16-bit accumulator |
| `0x10` | `DJ_FLAG_X8` | 65816 opcode bytes | ran with 8-bit index registers |
| `0x20` | `DJ_FLAG_X16` | 65816 opcode bytes | ran with 16-bit index registers |
| `0x40`, `0x80` | | | reserved |

- `dj_api_version` returns 2; a version 2 trace event carries 24-bit addresses and 16-bit
  registers (A, X, Y, S, D), plus DB and the E flag.
- The player picks its decoder from the region's `cpu` and rolls up whatever region the
  connector and symbol map name, instead of the hard-coded `PRG`.
- Cores that set only `0x01` (a recompiler that sees whole blocks, as the connectors README
  allows) keep working: the player falls back to decoded lengths for executed bytes.
- `data` and `file_offset` let the player decode exactly the bytes the core runs, at the
  offsets the flags use, without parsing ROM files itself (no `.smd` de-interleaving or N64
  byte-order handling in dj); a core that keeps the ROM swapped in memory copies it once at
  load. `file_offset` relates file-offset symbol formats (ca65 `.dbg`, splat `rom:`, ld maps)
  to the region.
- Regions are read after `retro_load_game`, since multi-system cores report different ones
  per system.
- connectors.md's "needs a sweep for the core's CPU" would become "a decoder for the
  region's `cpu`".

### Open questions

Decided (see "Decisions so far"): API compatibility (no API 1 layer), region names (stable,
per-system contract), the instruction set per region (from the connector, inferred for the
NES), ROM layout (from the connector, `data` and `file_offset`), regions read after
`retro_load_game`, and the dynarec (forced off by the patch, not warned about).

Decided after weighing these (decisions 14 and 15):

**An opcode that runs in both widths.** On the 65816, `LDA #imm` is 2 bytes with an 8-bit
accumulator and 3 with a 16-bit one. A routine called both ways sets both width bits on
the opcode, and the byte after the operand gets `0x01` in one run and `0x02` in the other.

| Option | For | Against |
| --- | --- | --- |
| (1) Count each (offset, mode) as its own instruction, decoding both streams | exact; shows both paths | counts and blocks no longer add up per byte; more decoder and UI work |
| (2) Split the block there and count the instruction once, marked ambiguous | simple; totals stay per byte; the mark says why | under-counts one of the paths |
| (3) Pick one mode silently (first or last seen) | simplest | hides real behaviour; wrong blocks after it |

Decided: (2), with all four bits stored so (1) stays possible later, and the conformance
check reporting how often it happens.

**Mode bits: generic or named per CPU.**

| Option | For | Against |
| --- | --- | --- |
| Generic: bits `0x04`-`0x20` mean whatever the region's `cpu` defines | one flag layout; no API change per new CPU; the decoder already knows the `cpu` | the same bit means different things per CPU; tools must read them with the `cpu` at hand |
| Named per CPU, each in its own bits | self-describing | six free bits run out fast (the 65816 needs four, ARM/Thumb two more) |

Decided: generic bits in the API and the database, with per-CPU named aliases in the header
(`#define DJ_FLAG_65816_A8 DJ_FLAG_MODE0`) and a table in connectors.md. Which CPUs need
them: the 65816 (M and X) and ARM (ARM/Thumb). None for the 6502, 68000, Z80 or MIPS.

Still open:

Deferred (2026-11-09, "address later"):

- How the GUI and reports mark approximate counts (never-entered functions, overlaid N64
  code).
- N64 overlays: telling them apart (`dj_origin` or otherwise).

## Suggested order

1. **Done 2026-11-09.** The player uses the operand flag for executed bytes
   (`analysis::executed_bytes`: `flags & 0x03` when the region has any `0x02`, else 6502
   lengths, and the no-map `gaps` note says which), so no-map `gaps` also works on non-NES
   ROMs when the core sets operand flags. `dj check-connector` (docs/cli.md) is the optional
   developer check. Results with Mesen:
   - Thwaite: 3733 instructions checked, operand flags present, no disagreements.
   - Tetris: 3610 checked, operand flags present, 37 disagreeing bytes (15 opcode flags where
     the decoder starts no instruction, 22 unexpected operand flags, none missing). All are
     in functions calling `bulkCopyToPpu` ($AA98), which takes a 2-byte inline pointer after
     the `JSR`: the decoder reads the pointer as instructions, while Mesen's flags show the
     real path. So it's unannotated data, not a connector fault; `[[data]]` entries for those
     19 call sites would fix the decoder's counts there.
   - Tetris totals unchanged (3610 / 4679 instructions, 792 / 1011 blocks, 207 / 241
     functions); no-map `gaps` unchanged (108 ranges, largest 13,069 bytes).
   Covered-instruction counts with a symbol map already came from opcode flags on decoded
   instructions, so they didn't change; the operand flag only matters for executed bytes.
2. **Done 2026-11-09, except measured cache settings.** Connector API 2
   (docs/connectors.md): `dj_region` gains `cpu`, `data` and `file_offset`; mode bits
   `0x04`-`0x20` generic per `cpu`; regions read after `retro_load_game`; no API 1 support.
   - Mesen patch: all regions `cpu` "6502"; `PRG` hands out PRG ROM with `file_offset` 16
     (528 with a trainer, -1 for UNIF/FDS); `WRAM`/`SRAM` buffers when present, `RAM` none;
     regions sized at load (`DjCoverage::OnLoad`). Core rebuilt with `build.sh`: sha256
     `3e9a32e4916cf47a25e103dfc2ad109cbcc137c8ca2c8e9ddaa7ff8763772ae0` (was `283a709a…`);
     examples pinned to `mesen@0102910c+dj2`.
   - Player: reads the v2 struct; records the connector's region facts per setup in the
     local database (`setups.info` "regions"; read by `cache::setup_regions`); the report
     rolls up `analysis::code_region` (first region from the ROM file with a decoder, `PRG`
     when nothing is recorded) and picks the decoder from its `cpu` (an unknown one gives
     opcode-byte counts and a report note); symbol maps are placed by the region's
     `file_offset` (`rom_header` overrides); the GUI's live coverage, first hits and
     annotation notes use the report's region. `dj init` prints the regions and checks each
     region's `data` against the ROM file at `file_offset`. No new config keys: build,
     options, hash regions and ports were already recorded.
   - Migration: both examples re-cleaned with the new core; every ending hash reproduced
     (Tetris 29 segments, Thwaite 12, none changed). `dj check-connector`: no disagreements
     on either (Tetris 3625 instructions checked after its 19 `bulkCopyToPpu` inline
     pointers were annotated; Thwaite 3733).
   - Not done: measuring cache settings (presets stay); decoding from the connector's `data`
     during a session (queries run without a core, so they slice the ROM file at
     `file_offset`, which `dj init` checks is the same bytes); query output and CLI/MCP
     addresses still say `PRG:` (fine for the NES; generalise with the first non-NES
     connector).
3. The N64: first the save-state round-trip test (serialize, unserialize, run on, compare
   hashes, since saves are deferred to a safe point). Then a mupen64plus-libretro-nx
   connector on the cached interpreter (forced by the patch), an `RDRAM` region, symbol maps
   by VRAM (splat), the MIPS decoder, and the checks: hashes with and without the patch, and
   slowdown.
4. N64 attribution to ROM: `dj_origin` from PI DMA provenance, overlays, and how symbol maps
   key them.
5. Later: other cores and their decoders (FCEUmm as a cheap side check of one decoder per
   instruction set, 6502; Genesis Plus GX if its licence is acceptable, 68000 and Z80;
   bsnes-mercury, 65816 with the mode bits), traces v2,
   `dj_map_address`, WRAM rollup, the SA-1, SPC700 and RSP.
