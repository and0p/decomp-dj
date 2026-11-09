# Plan: decoders as loadable libraries

Status: exploring, 2026-11-09. Nothing built. The question: can instruction decoders be
libraries loaded at run time, like cores, so a user can bring a decoder for a CPU dj doesn't
know, and point at it from config? Background on why decoding is per CPU and lives outside
the connector: [CONNECTOR_PLAN.md](CONNECTOR_PLAN.md#where-decoding-could-live).

## What the decoders do today

Two decoders are compiled into the player (`player/src/analysis.rs`, `trace.rs`):

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

1. The `Decoder` trait and one generic sweep, 6502 and MIPS moved onto it; totals
   unchanged on all three projects.
2. The C interface and `[[decoders]]` loading, with the 6502 decoder also built as a test
   `cdylib` (`player/tests/testdecoder/`) so loading is tested against a known answer.
3. Decide native or WASM after a first outside decoder (65816 is the likely one).
