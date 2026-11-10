# Plan: decoders as loadable libraries

Status: step 1 done 2026-11-09, and an ARMv4T decoder (below); steps 2 and 3 not started. The question: can instruction decoders be
libraries loaded at run time, like cores, so a user can bring a decoder for a CPU dj doesn't
know, and point at it from config? Background on why decoding is per CPU and lives outside
the connector: [CONNECTOR_PLAN.md](CONNECTOR_PLAN.md#where-decoding-could-live).

## What the decoders did before step 1

Two decoders were compiled into the player (`player/src/analysis.rs`, `trace.rs`); step 1 moved them into `decode.rs`:

| Piece | 6502 | MIPS (`mips3`) |
| --- | --- | --- |
| Instruction length | `op_len_6502` (table by opcode) | always 4 |
| How control leaves | `flow_6502`: next, branch, jump (or indirect), stop | `flow_mips`: next, call, branch, jump (or indirect), return, stop |
| Delay slots | none | 1 after every branch and jump |
| Calls for the callers list | `call_target_6502` (JSR, JMP abs) | from `flow_mips` (JAL, linking branches; J out of the function) |
| Sweep into instructions and blocks | `analyze_6502_with` | `analyze_mips_with` |
| Successors per block | `block_successors_6502` | inside `analyze_mips_with` |
| Executed bytes without operand flags | `executed_bytes_6502` | not needed (the connector sets operand flags) |
| Text for traces | `disasm_6502` (`trace.rs`) | none |
| Which CPUs exist | `has_decoder`: `matches!(cpu, "6502" \| "mips3")`, and `coverage.rs` picks by name |

Most of each `analyze_*` is the same algorithm written twice: a linear sweep from the
function's start that skips `[[data]]`, collects block leaders (the start, branch and jump
targets inside the function, and the instruction after a block ends), cuts blocks at the
leaders, then works out successors and call sites. Only the per-instruction answers differ.
So the interface can be per instruction; the sweep, blocks, successors, callers, the
conformance check (`dj check-connector`) and executed bytes stay in the player, written
once.

## Step 1: a decoder trait inside the player (no plugins yet)

**Done 2026-11-09.** `player/src/decode.rs`: the `Decoder` trait, `Mos6502` and `Mips3`,
`decoder_for(cpu)` (a table of built-ins; `analysis::has_decoder` asks it), and one
`analyze(dec, funcs, code, data, base)` sweep that replaced `analyze_6502_with`,
`analyze_mips_with`, `block_successors_6502` and `call_target_6502`. The report
(`coverage.rs`) picks the decoder from the code region's `cpu` and reads each function at
its `file` offset, else at its region offset when the region comes from the ROM file; the
no-map executed bytes (`analysis::executed_bytes`) take lengths from the region's decoder
instead of always the 6502.

What changed from the sketch below:

- **Kinds and a flag instead of `Flow`.** `Decoded { len, kind, conditional, target,
  delay_slots, mode_after, site }`, with `Kind` = Next, Jump, Call, Return, Stop. A branch is
  a conditional jump, so conditional returns and calls (SM83, Z80, ARM, PowerPC) need no new
  kinds. `site` is how a call site is listed ("JSR", "JAL"). Not added yet, for when a CPU
  needs them: `target_mode` (ARM `BX`/`BLX`), `data_ref` (PC-relative literal pools, ARM and
  SH-2), and `disasm` (traces stay `trace::disasm_6502` until traces carry wider addresses).
- **`fixed_len` and `alignment`** are on the trait (MIPS: 4 and 4). The sweep uses
  `alignment` to restart after data; `executed_bytes_by_length` uses `fixed_len`.
- **`mode`** is carried from each instruction's `mode_after` to the next; nothing reads the
  connector's mode bits into it yet (no CPU with modes has a decoder).

Behaviour, checked against the old code (a temporary test comparing both on 200k random
MIPS words, 64 KB of random 6502 bytes, and the Thwaite and Tetris ROMs with their maps;
then every `dj summary`, `report`, `gaps`, `frontier`, `check-connector`, `annotations` and
`function` output on both projects, diffed against a build of the previous commit):

- Identical totals, instructions, blocks and successors. Thwaite 3733 / 4656 instructions,
  835 / 1087 blocks; Tetris 3625 / 4657, 806 / 1026; `dj check-connector` clean on both.
- **Callers:** a 6502 `JMP` back to its own function's start (a loop) no longer lists the
  function as its own caller: 2 functions in Thwaite (`main`, `cut_handle_state_script`), 4 in Tetris. Jumps
  inside a function aren't call sites on any CPU now, as MIPS already did.
- **MIPS `break` and `eret`** end their block at once (they have no delay slot); before,
  the next instruction was taken into the block. Not checked on No Mercy: that scratch
  project is gone.
- A jump in another's delay slot (undefined on MIPS, seen only in random data) leaves the
  first one in charge, as before.

Speed (the benchmark in [Performance](#performance), same random input): MIPS 107 to
64 ns per instruction (1M instructions in 67 ms); 6502 about the same (60 ns, under 1 ms for
the ROM; the call through the trait shows at this size). The decoded functions are cached
(`coverage::decode_cached`, keyed by a hash of the decoder, the functions, the data ranges
and the ROM), so the GUI's report after an edit doesn't decode again.

The sketch as first written:

Worth doing whatever is decided about plugins: it removes the duplicated sweep and makes
a third CPU a table rather than a third copy.

```rust
pub enum Flow {
    Next,
    Branch(u64),          // conditional, to a CPU address
    Jump(Option<u64>),    // unconditional; None = indirect (jump tables)
    Call(Option<u64>),    // returns to after the instruction (and its delay slots)
    Return,
    Stop,                 // BRK, KIL, BREAK: execution doesn't go on
}

pub struct Decoded {
    pub len: u8,          // 0: not an instruction (the sweep stops the block here)
    pub flow: Flow,
    pub delay_slots: u8,  // instructions that run before control moves (MIPS: 1)
    pub mode_after: u32,  // the mode the next instruction runs in (65816 REP/SEP, ARM BX)
}

pub trait Decoder {
    fn cpu(&self) -> &str;                  // "6502", "mips3"
    fn max_len(&self) -> usize;             // bytes the player must offer (6502: 3, MIPS: 4)
    fn decode(&self, bytes: &[u8], pc: u64, mode: u32) -> Decoded;
    fn disasm(&self, bytes: &[u8], pc: u64, mode: u32) -> Option<String> { None }
}
```

- `bytes` are in the order they lie in the ROM file (or the region's `data`), which the
  connector contract fixes per system (N64: `.z64`, big-endian). The decoder is told
  nothing about byte order beyond its CPU.
- `pc` is the CPU address, so relative branches and MIPS `J` regions can be resolved. When
  the symbol map gives no CPU address, the player passes the region offset, as now.
- `mode` comes from the connector's mode bits (`0x04`-`0x20`) on the opcode byte where code
  ran, and from `mode_after` of the previous instruction where it didn't. Its meaning is the
  decoder's business; the player only carries it. The 6502 and MIPS ignore it.
- `Calls` replaces `call_target_6502`; a jump out of the function is also listed as a call
  site, as now.
- Registry: built-in decoders in a table by `cpu`; `has_decoder` and `coverage.rs` look up
  the table instead of matching names.

Check: Tetris, Thwaite and the No Mercy scratch project give the same totals, blocks,
successors and callers before and after; `dj check-connector` unchanged.

## ARMv4T (GBA)

**Decoder done 2026-11-09; not used yet**, because no connector reports `armv4t`.
`player/src/arm.rs`: `Armv4t`, ARM and Thumb, little-endian; modes `arm::ARM` (0) and
`arm::THUMB` (1).

Interface additions it needed, now on `Decoder`/`Decoded` (the 6502 and MIPS leave them at
their defaults; their output is byte-identical to before, checked as in step 1):

- `entry(addr) -> (addr, mode)`: a symbol's bit 0 means Thumb, as in ELF. The sweep starts
  each function in that mode at the address with the bit cleared.
- `alignment(mode)`: 4 in ARM, 2 in Thumb.
- `target_mode`: `bx pc` from Thumb jumps to the next word in ARM mode; the sweep switches
  mode when it reaches that target.
- `data_ref`: PC-relative literal loads (`LDR`, `LDRB`, `LDRH`, `LDRSB`, `LDRSH` from
  `[pc, #n]`; Thumb `ldr rN, [pc, #n]`). A literal further on in the function is skipped
  like `[[data]]`, so pools aren't decoded and don't count as instructions. One already
  passed (a backward reference) stays as read.

What it reads: `B`/`BL` and Thumb's two-halfword `BL`; conditional execution on every ARM
instruction (a conditional return falls through); `BX` (`bx lr` returns, `bx pc`, other
registers indirect); data processing that writes the PC (`mov pc, lr` and `subs pc, lr, #n`
return, the rest, such as `add pc, pc, r0, lsl #2`, jump indirectly); `LDR pc` (`ldr pc,
[sp], #4` returns); `LDM`/`POP` with the PC (from `sp` returns); Thumb hi-register
`MOV`/`ADD` to the PC; `SWI` as a call. Unpredictable forms (byte or halfword loads into the
PC, `LDRT pc`, cond `0xF`, `BLX` from ARMv5) read as plain instructions.

**Checked against LLVM 23** (`llvm-mc` assembling `.inst` words into an ELF,
`llvm-objdump --triple=armv4t`/`thumbv4t` disassembling it, the text classified by a
script): 300,000 random ARM words and 300,000 Thumb halfwords, biased toward branches, PC
writes, PC-relative loads and `BL` pairs. 524,410 instructions LLVM decodes; ours agree on
length, kind, condition, target and literal on all of them. Left out: encodings LLVM
rejects, and ~900 halfword pairs LLVM reads as `BL` by Thumb-2's rules (a second half of
`11110`), which on ARMv4T are two `BL` first halves; ours follows ARMv4T. The script isn't
kept in the repo.

Known limits:

- `mov lr, pc; bx rN` (an ARM-mode call through a register) reads as an indirect jump,
  so the block ends there and the code after it isn't a successor. A decoder sees one
  instruction; recognising the pair would need the sweep to look back.
- A function's mode comes only from its symbol's bit 0. Symbol maps without it (csv, `.map`
  files) start in ARM mode; the connector's mode bit on code that ran (see below) could
  correct that.
- `executed_bytes_by_length` (no-map gaps on a core without operand flags) decodes in ARM
  mode. A GBA connector should set operand flags, which avoids it.

**GBA end to end: done 2026-11-09** (mGBA connector, [connectors/mgba](../connectors/mgba/README.md);
test project [examples/gba-test](../examples/gba-test/)):

1. Connector for libretro's mGBA: regions `ROM`, `IWRAM`, `EWRAM`, `BIOS`; flags per
   instruction and the mode bits `0x04` Thumb / `0x08` ARM (now in docs/connectors.md).
2. The player takes a function's starting mode from the mode bits where its first
   instruction ran (`Decoder::mode_from_flags`, in `coverage.rs` before decoding), over the
   symbol's. `Function` gained `mode`; the `entry()` method sketched above went, since the
   ELF loader now clears Thumb's bit 0 from the address and sets `mode`.
3. ELF: Thumb bit 0 read as above; `$d` mapping symbols read as data ranges
   (`analysis::load_elf_data`), merged with `[[data]]` in the report. Symbol maps' bytes are
   now read where the connector places the region, over `rom_header`, which for an ELF is the
   address of the ROM file's first byte.
4. A test ROM built from source with LLVM (no GBA toolchain): `dj check-connector` clean on
   ARM and Thumb, 113 / 140 instructions, hashes the same with and without the patch.
   Still to try: a real game and its pret decomp build (locally; needs the user's ROM).

Follow-ups: IWRAM/EWRAM rollup (functions linked to run from RAM; the ELF knows both
addresses, so it could place them in `IWRAM` with a `file` offset, as the N64 does);
`dj init` choosing GBA hash regions; the connector's speed (37% slower).

## Step 2: loading decoders from libraries

### The C interface

```c
#define DJ_DECODER_API 1

enum dj_flow { DJ_NEXT, DJ_BRANCH, DJ_JUMP, DJ_CALL, DJ_RETURN, DJ_STOP };

struct dj_insn {
    uint8_t  len;          /* 0: not an instruction */
    uint8_t  flow;         /* enum dj_flow */
    uint8_t  delay_slots;
    uint8_t  has_target;   /* 0 for DJ_NEXT, DJ_RETURN, DJ_STOP, and indirect jumps or calls */
    uint32_t mode_after;
    uint64_t target;       /* CPU address, when has_target */
};

uint32_t    dj_decoder_api_version(void);                 /* 1 */
const char *dj_decoder_cpus(void);                        /* "65816\nspc700\n": one library, several CPUs */
size_t      dj_decoder_max_len(const char *cpu);
bool        dj_decode(const char *cpu, const uint8_t *bytes, size_t avail,
                      uint64_t pc, uint32_t mode, struct dj_insn *out);
/* optional */
size_t      dj_disasm(const char *cpu, const uint8_t *bytes, size_t avail,
                      uint64_t pc, uint32_t mode, char *buf, size_t cap);
```

Pure functions, no state, no allocation across the boundary, so any language that can
export C symbols works (C, C++, Rust `cdylib`, Zig). The player wraps a library in the same
`Decoder` trait as the built-ins.

### Finding and choosing them

```toml
[[decoders]]                     # config.toml, or .local/local.toml for this machine
path = "decoders/dj_65816"       # suffix added per OS, like core paths
sha256 = "…"                     # optional pin, as for cores
```

- Paths follow the core path rules ([RELEASE_PLAN.md](RELEASE_PLAN.md#decisions)): a bare
  name is looked for in the project folder, then `<exe dir>/decoders/`; relative is from the
  project; absolute as written.
- A decoder from config takes over a built-in for the same `cpu`, so a user can try a
  fixed 6502 decoder without rebuilding dj. `dj init` and `dj summary` say which decoder
  each region's CPU uses (built-in, or the library's path and sha256).
- Coverage totals depend on the decoder, so the report records which one it used (and the
  committed report's `meta.toml`, once that exists, says so too).

### Native libraries or WebAssembly

| | Native (`.so`/`.dylib`/`.dll`, `libloading`) | WebAssembly (one `.wasm`, e.g. wasmtime) |
| --- | --- | --- |
| Builds a user ships | one per OS and architecture | one file for every platform |
| New dependencies | none (`libloading` is already used for cores) | a WASM runtime, several MB and slower builds |
| Safety | a bad decoder can crash dj | sandboxed; a bad decoder gets an error |
| macOS quarantine | an unsigned downloaded library is blocked, as cores are | not an issue |
| Speed | fastest | slower per call, but decoding is a few thousand calls per function list |
| Fits with | cores and connectors, which are native already | nothing else in dj today |

Lean: native first, since it matches cores and adds nothing; the C interface above maps
one to one onto a WASM module's exports if portability turns out to matter.

## Performance

Measured 2026-11-09 with a throwaway benchmark (release build, Apple silicon), on random
bytes, which branch more often than real code, so the decode numbers are an upper bound:

| Work | How often | Time |
| --- | --- | --- |
| `analyze_6502_with`, 32 KB, 245 functions (15k instructions) | per coverage report | 0.8 ms, 52 ns per instruction |
| `analyze_mips_with`, 4 MB, 1,136 functions (1M instructions) | per coverage report | 112 ms, 107 ns per instruction |
| `dj summary` on Tetris, whole command | per command | 30 ms |
| First-hit scan of the coverage flags (`player.rs`, `Recorder`), 40 MB (N64) | every frame while recording | 28 ms |

**The universal interface costs little.** A call through a trait object is a few ns per
instruction, through a C library about 5-10 ns. Most of the 107 ns is the sweep's
bookkeeping, shared by every CPU: a `BTreeSet` of leaders, a scan of every `[[data]]`
range per instruction, and `flow_at` in `analyze_mips_with`, a linear search per block that
makes the successor pass quadratic in function size. Step 1's single sweep fixes these once:
leaders in a sorted `Vec` or bitset, data ranges sorted and walked alongside the sweep, flows
indexed by instruction.

**Cache the decoded functions.** Decoding depends on the ROM, the symbol maps,
annotations.toml and the decoder (its build or sha256), not on segments, yet the GUI
rebuilds the report, decoding included, after every edit. Keep the `AnalyzedFunction`s
until one of those inputs changes (in memory in the GUI and `dj mcp`, maybe in the local
database keyed by their hashes).

**Where CPU differences do matter: fixed-width instruction sets.** MIPS, PowerPC, ARM mode
and the SH-2 start instructions only at aligned 2- or 4-byte positions. A decoder declares
it, and the player picks faster paths:

```rust
fn fixed_len(&self) -> Option<u8> { None }   // Some(4) for MIPS: every instruction this long
fn alignment(&self) -> u8 { 1 }              // instructions start only at multiples of this
```

```c
/* optional in DJ_DECODER_API 1 */
uint8_t dj_decoder_fixed_len(const char *cpu);   /* 0: variable */
uint8_t dj_decoder_alignment(const char *cpu);
```

- Instruction boundaries are known without decoding lengths, so the sweep only looks for
  control flow.
- Scans of the flags (first hits, unions, executed bytes) need only look at aligned
  positions: 4x less for MIPS.
- Coverage can be held as one bit per instruction slot instead of a byte per byte (up to
  32x smaller unions for N64-sized regions, in memory and in the local database). Mode bits
  are the exception; a fixed-width CPU with modes (ARM/Thumb) keeps the bytes.

Variable-length CPUs with modes (65816, x86, ARM/Thumb mixed) can't take these shortcuts,
and lose nothing to the shared interface either: their sweep goes instruction by
instruction anyway.

**Plugins: an optional batch call.** Where a call crosses into a library, and more so into
WebAssembly (tens of ns per call), decoding a whole function per call removes the per-call
cost. Built-in decoders don't need it.

```c
/* optional: decode up to `max` instructions from `bytes`, in order, stopping at `avail` */
size_t dj_decode_range(const char *cpu, const uint8_t *bytes, size_t avail, uint64_t pc,
                       uint32_t mode, struct dj_insn *out, size_t max);
```

**Not the decoder: the per-frame flag scan.** The real hot path. See the N64 playback
speed entry in [ROADMAP.md](ROADMAP.md#comparison-and-parity) ("N64 follow-ups"); parity
reads flags per frame the same way, so it gains from the same fixes.

## Open questions

- **Who writes decoders?** Probably whoever writes the connector for a new system. If
  that's the main case, a decoder could also be exported by the connector's core (the same
  `dj_decode` names, looked up through the proc address callback) as well as by a
  standalone library. Per CPU stays the rule; the core just carries one.
- **Mode across unrun code.** `mode_after` follows REP/SEP and BX in a straight line, but
  not across a branch whose target is reached with another mode. Good enough for counts
  (code that ran uses the flags' mode); worth saying in the report that unrun 65816 code is
  approximate.
- **Executed bytes without operand flags** use instruction lengths today (6502 only). With
  the trait this works for any decoder, but needs the mode where the flags carry none.
- **Trace events** have a 16-bit address and 8-bit registers ([CONNECTOR_PLAN.md](CONNECTOR_PLAN.md)
  "traces v2"); `dj_disasm` only helps once traces carry wider addresses.
- **Versioning.** `DJ_DECODER_API` bumps with breaking changes; the player refuses other
  versions with a message, as for connectors.
- **Testing a decoder.** A `dj check-decoder <lib> --cpu X --rom FILE` that sweeps known
  functions and compares with a project's flags (the conformance check, pointed at a
  library) would let authors test without a full project.

## Suggested order

1. **Done 2026-11-09.** The `Decoder` trait and one generic sweep, 6502 and MIPS moved
   onto it, with the sweep fixes, `fixed_len` and `alignment`, and caching
   ([Step 1](#step-1-a-decoder-trait-inside-the-player-no-plugins-yet)).
2. The C interface and `[[decoders]]` loading, with the 6502 decoder also built as a test
   `cdylib` (`player/tests/testdecoder/`) so loading is tested against a known answer.
3. Decide native or WASM after a first outside decoder (65816 is the likely one).
