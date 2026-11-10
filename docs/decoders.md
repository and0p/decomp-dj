# Decoders

A decoder reads one instruction set. For each instruction it says how long it is and how
control leaves it: on to the next instruction, a jump or branch, a call, a return or a stop,
with the target when the bytes give one, its delay slots, the mode the next instruction runs
in, and any literal it reads. From those answers dj splits each function a symbol map names
into instructions and basic blocks, and finds successors and callers
([coverage.md](coverage.md#symbol-maps)). The work after decoding is the same for every
instruction set.

Built in:

| `cpu` | Instruction set | Systems |
| --- | --- | --- |
| `6502` | 6502 with the stable undocumented opcodes | NES |
| `mips3` | MIPS III, big-endian | N64 |
| `armv4t` | ARMv4T, ARM and Thumb, little-endian | GBA |

Any other instruction set can be added as a **decoder plugin**: a shared library (`.so`,
`.dylib`, `.dll`) that exports one function, `dj_decoders`. Plugins can be written in C or
anything that exports C functions; Rust has a helper crate. A libretro core with a coverage
connector can export the same function, for the CPUs it runs.

## Using a plugin

```toml
[[decoders]]                       # config.toml, or .local/local.toml for this machine
path = "decoders/dj_65816"         # the platform's suffix may be left out, as for cores
sha256 = "…"                       # optional pin
```

- **Paths:** absolute as written; relative from the project folder. A bare name (no folder)
  is looked for in the project folder, then in `decoders/` next to the `dj` executable.
- **Which decoder decodes a CPU:** a `[[decoders]]` plugin first, so a plugin can stand in
  for a built-in decoder; then the built-in one; then one the project's core exports.
- **Loading:** a `[[decoders]]` entry that's missing, fails to load, exports no decoders, is
  for another API version, doesn't match its `sha256`, or crashes or hangs while its tables
  are read stops the project from opening, with a message naming the entry. A core's
  decoders are read only when a CPU nothing else decodes is asked for.
- **Plugins never run inside dj.** dj reads a plugin's tables, and decodes with it, in a
  worker: a separate `dj` (or `dj-gui`) process. A plugin that crashes or hangs while
  decoding costs that report its functions, not dj: the report says so in a note
  ("the 65816 decoder (…) crashed (signal 11, SIGSEGV) decoding `ResetVector`") and counts
  the region in opcode bytes. A worker that takes longer than 10 s (plus 2 s per MB of code
  and 1 s per 1,000 functions) is stopped. The built-in decoders run in dj.
- **Layering:** `.local/local.toml`'s `[[decoders]]` replace config.toml's, unless every
  entry has a `name`, in which case they merge by name ([config.md](config.md#layering)).
- **Reports say which decoder ran:** `dj summary` and `dj report` print a note such as
  `ROM (armv4t) decoded by decoders/dj_arm.dylib sha256 0d6e592166e8` when a plugin decoded
  the code region, since totals depend on the decoder. Coverage results are cached per
  decoder, so changing one never reuses another's.

## Writing a plugin in C

Copy [decoders/include/dj_decoder.h](../decoders/include/dj_decoder.h) (MIT) and export
`dj_decoders`:

```c
#include "dj_decoder.h"

static void decode(const uint8_t* b, size_t n, uint64_t pc, uint32_t mode, struct dj_instruction* out) {
	if (n == 0) return;                     /* nothing to read: length stays 0 */
	switch (b[0]) {
	case 0x10:                              /* a branch: pc + 2 + rel */
		if (n < 2) return;
		out->length = 2;
		out->kind = DJ_JUMP;
		out->conditional = 1;
		out->has_target = 1;
		out->target = (pc + 2 + (int8_t) b[1]) & 0xFFFF;
		break;
	/* ... */
	default:
		out->length = 1;                    /* kind DJ_NEXT: on to the next one */
	}
}

static const struct dj_decoder mine = {
	DJ_DECODER_API, sizeof(struct dj_decoder), "toy8", /* max_length */ 3, /* fixed_length */ 0,
	{0}, /* alignment */ NULL, /* mode_from_flags */ NULL, decode,
};
static const struct dj_decoder* const all[] = { &mine };

const struct dj_decoder* const* dj_decoders(size_t* count) { *count = 1; return all; }
```

Build it as a shared library (`cc -shared -fPIC -O2 -o libmine.so mine.c`).
[player/tests/testdecoder/toy.c](../player/tests/testdecoder/toy.c) is a complete one, for a
made-up instruction set.

### The rules

- **Pure and thread-safe.** The same bytes, address and mode always give the same answer.
  No state between calls: dj may call from several threads at once, and caches results.
- **Quick to load.** dj loads the library in a new worker process for each report it
  decodes, so start-up code should do little.
- **Zeroed output.** dj zeroes `struct dj_instruction` before each call; set only what
  applies. `length` 0 means the bytes aren't an instruction (or don't fit in `available`).
- **What each field means:**

  | Field | Meaning |
  | --- | --- |
  | `kind` | `DJ_NEXT`, `DJ_JUMP` (a conditional one is a branch), `DJ_CALL` (control comes back after it and its delay slots), `DJ_RETURN`, `DJ_STOP` |
  | `conditional` | execution may also go on to the next instruction: a branch, a conditional return (`bxeq lr`, `RET NZ`) |
  | `target`, `has_target` | the CPU address a jump or call goes to, when the bytes say; leave unset for indirect ones (jump tables, `jr $t0`) |
  | `delay_slots` | instructions that run before control moves (MIPS: 1) |
  | `mode_after` | the mode the next instruction runs in; for instruction sets without modes, 0 |
  | `target_mode`, `has_target_mode` | the mode at `target`, when the jump changes it (ARM `bx pc` from Thumb) |
  | `data_ref`, `data_ref_length` | a literal the instruction reads at a CPU address its bytes give (ARM `ldr r0, [pc, #8]`): dj won't read those bytes as code |
  | `site` | how a call site through this instruction is listed in callers ("BL", "JSR"); a string that stays valid while the library is loaded |

- **Modes** are the decoder's own numbers. Where the CPU's connector sets mode bits on code
  that ran (docs/connectors.md, "What a connector reports"), `mode_from_flags` turns them into
  a mode, and dj starts a function in the mode its first instruction ran in.
- **Bytes** come in the ROM file's order, which the connector contract fixes per system
  (the N64's `.z64` is big-endian).
- **Answers are checked.** A `kind` dj doesn't know, or a `length` over `max_length`, is read
  as "not an instruction", counted, and reported in a note by `dj summary`.
- **Versions:** `api_version` is `DJ_DECODER_API` (1). Later versions add fields at the end of
  `struct dj_decoder`, and `size` says how much of it the plugin knows. dj refuses a decoder
  for another API version, naming the file.

## Writing a plugin in Rust

The [dj-decoder](../decoders/dj-decoder/) crate has the `Decoder` trait dj uses itself, and
`export_decoders!`, which exports Rust decoders through the C interface:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
dj-decoder = { path = "../decomp-dj/decoders/dj-decoder" }
```

```rust
use dj_decoder::{Decoded, Decoder, Kind};

#[derive(Default)]
struct Wdc65816;

impl Decoder for Wdc65816 {
    fn cpu(&self) -> &'static str { "65816" }
    fn max_len(&self) -> usize { 4 }
    fn decode(&self, bytes: &[u8], pc: u64, mode: u32) -> Decoded { /* ... */ Decoded::next(1) }
}

dj_decoder::export_decoders!(Wdc65816);
```

Decoders are unit structs that implement `Default`. A panic inside `decode` is caught and
reads as "not an instruction" rather than crossing into dj. [decoders/reference](../decoders/reference/)
exports the built-in decoders this way; dj's tests check that it decodes exactly as they do.

## Decoders in a core

A core can export `dj_decoders` itself, for the instruction sets it runs, so a new system
needs no separate file. dj reads it from the core library named in `.local/local.toml`
without starting the core. Keep the decoder's source apart from the core's patch, so that
another core for the same CPU can compile the same decoder.

## Workers

A worker is dj's own program started again with a hidden first argument (`__dj-worker
decode`); nothing extra is installed. Two environment variables are for testing:
`DJ_WORKER_EXE` starts another program as the worker (dj's tests point it at `dj`), and
`DJ_WORKER_TIMEOUT_SCALE` multiplies the time limits. Why and how: .agents/ISOLATION_PLAN.md.

## Checking a decoder

- **Against a core:** with a project that has a symbol map, `dj check-connector` compares
  every instruction the decoder reads with the connector's flags on code that ran ([cli.md](cli.md)).
- **Against another decoder or disassembler:** decode random bytes with both and compare
  lengths, kinds and targets. The built-in ARMv4T decoder was checked against LLVM's
  disassembler this way (.agents/DECODER_PLAN.md).
