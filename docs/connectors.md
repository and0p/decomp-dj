# Coverage connectors

A coverage connector is a small patch to a libretro core that reports which ROM bytes ran.
Without one, a core still plays segments and checks hashes, but records no coverage and
counts no [ticks](playback.md#logic-ticks-draft). There are connectors for Mesen (NES),
[connectors/mesen/](../connectors/mesen/README.md), mGBA (GBA),
[connectors/mgba/](../connectors/mgba/README.md), Mesen-S (SNES),
[connectors/mesen-s/](../connectors/mesen-s/README.md), and, in a first version, Mupen64Plus-Next
(N64), [connectors/mupen64plus/](../connectors/mupen64plus/README.md).

## What a connector reports

- Executed locations as **offsets into ROM regions**, never CPU addresses, because bank switching maps different ROM bytes to the same CPU address.
- One flag byte per byte of each region (below). Flags only ever get set, until the player clears them.
- For each region: its name, the instruction set that runs there, its bytes, and where it lies in the ROM file. The connector is the source of these facts; projects don't configure them ([.agents/_archive/CONNECTOR_PLAN.md](../.agents/_archive/CONNECTOR_PLAN.md) has the reasoning).
- **Region names are a contract per system**, so that two cores for one system can share a project's coverage, annotations and symbol maps. NES: `PRG` (program ROM), `RAM`, `WRAM`, `SRAM`. N64: `RDRAM` (main RAM, by physical address, where code runs) and `ROM` (the cartridge, `.z64` order). GBA: `ROM` (the cartridge, from byte 0 of the file, its address mirrors folded together), `IWRAM`, `EWRAM` and `BIOS`. SNES: `ROM` (the cartridge, after any 512-byte copier header, however the board maps it), `WRAM` and `SRAM`. Coverage rolls up into the first region from the ROM file whose instruction set has a decoder (`PRG` on the NES), else into the first region a CPU runs (`RDRAM` on the N64).

| Bits | Meaning |
| --- | --- |
| `0x01` | ran as the first byte of an instruction |
| `0x02` | ran as an operand byte. A core that only sees whole blocks may set `0x01` on each block's first byte and nothing else; the player then works operand bytes out from instruction lengths |
| `0x04`-`0x20` | mode bits, on opcode bytes, defined per instruction set (none for the 6502) |
| `0x40`, `0x80` | reserved |

Mode bits are generic: what each means comes from the region's `cpu`. For the 65816, for
example, they would record the register widths an opcode ran with, so an opcode run in both
keeps both: `0x04` 8-bit accumulator, `0x08` 16-bit accumulator, `0x10` 8-bit index
registers, `0x20` 16-bit index registers.

| `cpu` | Mode bits |
| --- | --- |
| `6502`, `mips3` | none |
| `65816` | as above: `0x04` 8-bit accumulator, `0x08` 16-bit, `0x10` 8-bit index registers, `0x20` 16-bit; emulation mode is 8-bit |
| `armv4t` | `0x04` ran in Thumb state, `0x08` in ARM state. The player starts a function in the mode its first instruction ran in. Thumb's `BL` pair is one instruction: its second halfword carries operand flags |

## The API

The core registers `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK` and returns these
functions by name. The player looks them up after `retro_init`, and treats a core without
them as having no connector. It reads the regions after `retro_load_game` (a core for
several systems reports different ones per game). A connector of another API version is
ignored with a warning: rebuild it.

```c
uint32_t dj_api_version(void);                       // 3
size_t   dj_region_count(void);
bool     dj_get_region(size_t index, struct dj_region *out);
void     dj_clear(void);                             // zero every flag
void     dj_set_breakpoints(const struct dj_break *list, size_t n, dj_break_fn fn, void *user);

struct dj_region {
    const char    *name;        // "PRG", "RAM", ...
    uint8_t       *flags;       // one flag byte per region byte, owned by the core
    size_t         size;
    const char    *cpu;         // instruction set that runs here ("6502"), or NULL
    const uint8_t *data;        // the region's bytes in the CPU's byte order, or NULL
    int64_t        file_offset; // where offset 0 lies in the ROM file, or -1
};

struct dj_break {               // a breakpoint
    int32_t region;             // index as in dj_get_region
    int64_t offset;             // into that region
};

// Called when the instruction at a breakpoint is about to run. Non-zero switches that
// breakpoint off until the list is set again.
typedef int (*dj_break_fn)(void *user, int region, int64_t offset);
```

**Breakpoints.** `dj_set_breakpoints` replaces the list (`n` 0, or no function, turns them
off). When the CPU is about to run an instruction at one, the connector calls `fn` and
waits: the core is inside `retro_run`, and carries on when `fn` returns. The frontend may
read the core's memory and registers meanwhile, and may write memory; it must not run,
save, load or reset the core.

- **This is how dj counts [logic ticks](playback.md#logic-ticks-draft):** the project's
  `[tick]` address is a breakpoint, and dj counts the calls. `dj_clear` has nothing to do
  with it.
- **A connector keeps its breakpoints apart from the coverage flags** (a bit per byte of
  its own), since the player stores the flags.
- **Places a connector can't break at are left out,** not refused: outside a region, or,
  on the N64, off a word boundary or outside `RDRAM`.
- **No call while a state is saved or loaded.** Mupen64Plus-Next runs instructions before
  it loads a state; its connector holds those back, and dj ignores any that arrive.
- **A breakpoint in a loop** would call thousands of times a frame. Past 1,024 calls in
  one frame dj returns non-zero, which switches it off, warns once, and sets the list
  again when the next frame starts.

Measured on the pinned cores: a call that returns at once costs a few nanoseconds, and
playing with a breakpoint armed is within about 1% of playing without
([.agents/yield-parity/PLAN.md](../.agents/yield-parity/PLAN.md), "Spike results").

Optional:

```c
const char *dj_forced_options(void);  // "key=value\n" lines: core options the connector needs
const char *dj_build_id(void);        // which build this is, for [core] build
size_t      dj_read_registers(void *out, size_t cap);  // the CPU's registers; returns the bytes written
```

`dj_read_registers` fills `out` with the registers of the CPU that runs the code regions,
and returns 0 if `cap` is too small or it can't say. The layout is fixed per instruction
set (a first version: nothing in dj reads them yet):

| `cpu` | Layout |
| --- | --- |
| `6502` | 7 bytes: A, X, Y, SP, P, then PC with its low byte first |
| `mips3` | 33 values of 64 bits in the host's byte order: PC, then r0 to r31 |
| `65816` | 16 bytes: A, X, Y, S, D and PC as 16-bit values, low byte first, then the program bank, the data bank, P, and E (1 in emulation mode) |
| `armv4t` | 17 values of 32 bits in the host's byte order: r0 to r15, then CPSR. r15 is as the CPU holds it, two instructions past the one about to run |

`dj_build_id` names the build exactly and the same way on every platform: the upstream
commit and the connector, as `<core>@<commit, 8 hex digits>+dj.<sha256 of the patch and then
the files under src/ in path order, with \r removed, 8 hex digits>` (`mesen@0102910c+dj.34266c1a`). Each connector's
`build.sh` writes it into a header the connector includes. dj records it in a project's
`[core] build` and warns when a core reports another ([config.md](config.md#core)); a core
without it is named by its `library_name` and `library_version`, which often don't change
when it's rebuilt or patched.

A connector that forces core options while dj drives the core (an interpreter instead of a
dynarec, a video plugin whose state saves) lists them here. The player takes them as its
own values before the game loads, so the options a project records are the ones that run.

`data` lets the player read the bytes the core runs, at the offsets the flags use; a core
that keeps ROM byte-swapped in memory hands out a copy in the CPU's order. `file_offset`
places symbol maps whose offsets count from the start of the ROM file (ca65 `.dbg`, splat,
ld maps): Mesen reports 16 for `PRG`, after the iNES header (528 with a trainer), and -1 for
the RAM regions. `dj init` prints what the connector reports and checks `data` against the
ROM file. The player records these facts per setup in the local database.

The player calls `dj_clear` when a segment starts and reads the flags as it plays, so
each segment records only its own frames. The GUI also reads and clears them on every
screen refresh for live coverage.

## Traces

A connector may also export a trace hook, which the [MCP server](mcp.md#traces) uses to
record every instruction and memory access over a span of frames. The player looks these up
too, and treats a core without them as unable to trace.

```c
bool   dj_trace_start(size_t capacity);   // start recording, keeping at most capacity events
void   dj_trace_stop(void);
size_t dj_trace_read(const struct dj_trace_event **events, uint64_t *dropped);
                                          // events since the last read; clears them

struct dj_trace_event {      // 40 bytes
    uint64_t cycle;
    int64_t  offset;         // offset in the region, -1 if none
    uint16_t addr;           // CPU address
    uint8_t  kind;           // 1 execute, 2 read, 3 write
    int8_t   region;         // index as in dj_get_region, -1 if none
    uint8_t  value;          // the byte read or written, or the opcode
    uint8_t  a, x, y, sp, p; // registers before an executed instruction
    uint8_t  operand_count;
    uint8_t  operands[3];
    uint16_t addr_high;      // the CPU address's high 16 bits, on a CPU with 32-bit addresses
    uint8_t  reserved[2];    // 0
};
```

Events past the capacity are counted in `dropped` rather than stored. A read or write of
several bytes is one event per byte.

The Mesen, Mesen-S and Mupen64Plus connectors have the hook. On the 65816 (Mesen-S) the
address is `addr_high << 16 | addr`, 24 bits, and the registers are the low bytes of A, X,
Y and S, and P. The struct was made for the 6502; on
the N64 it is filled like this:

- **The address** is `addr_high << 16 | addr`: the CPU's (virtual) address, such as
  `0x80005BD0`. `offset` is the physical address, in region `RDRAM`. An access outside
  RDRAM (a hardware register) has region -1.
- **An execute** carries the whole instruction: `value` is its first byte and `operands`
  the other three, in the CPU's (big-endian) order.
- **The registers are 0.** `dj_read_registers` gives them at a breakpoint.
- **A read is of the whole word,** four events, whatever the instruction took from it: the
  CPU's read function reads a word and the instruction picks its bytes afterwards. A write
  is of exactly the bytes written.
- **`cycle`** counts instructions since the trace started.
- **Only what the CPU does is traced.** A DMA from the cartridge and the GPU's copies into
  RAM are not in it.
- **About 830,000 events a frame** in a No Mercy match, 33 MB. The buffer grows as it
  fills, up to the capacity.

## Comparison cores

A [comparison core](gui.md#comparison-cores) (a decomp built as a libretro core, say) runs
beside the baseline on the same input lines. There are two kinds
([config.md](config.md#compare-optional-repeatable), `sync`):

- **A core that takes events** (the default). It exports `dj_handle_event`, found by name
  like the rest: through the core's proc address callback, then its library exports. The
  rest of this section is about these.
  - `include/dj_compare.h` declares what such a core exports, with the
    [helper](#the-helper).
  - A core that also reports its own coverage or traces exports the connector API above
    as well. `include/dj_connector.h` declares that: the version, the regions,
    breakpoints, registers and the trace hook.
- **The same emulator as the baseline** (`sync = "state"`), to compare builds or options.
  It needs no functions: dj loads the baseline's save state into it after a jump, runs it
  one frame after the baseline, and compares the two cores' system RAM byte for byte.
  `[WRITE]` lines go straight into its memory.

A comparison core runs in dj's own process, in the GUI and under `dj parity`
([gui.md](gui.md#comparison-cores)), and the handle it is given points at the baseline core
itself. Reading through it copies nothing. **A comparison core must only read the
baseline.**

**The header.** [`include/dj_compare.h`](../include/dj_compare.h)
declares everything below, and has a helper for the part most cores share
([below](#the-helper)). Copy it into your core.

**Inputs.** Held buttons aren't part of a save state: after a jump, and before each frame,
dj sets the comparison core's ports and inputs as the segment has them, as it does for the
baseline. The core reads them through its libretro input callbacks, during any event.

The comparison core's coverage functions are ignored for now.

### Events

A core that takes events is a kind of debugger client: it says where in the original it
wants breakpoints, dj calls it each time the baseline gets to one, with the baseline
stopped and readable, and the core runs itself, compares, and answers with a verdict. dj
does not sync it and does not call its `retro_run`.

```c
struct dj_baseline {                 // read the baseline during a call
    uint32_t version;                // 2
    void    *ctx;
    // A memory region by the name [WRITE] and [hash] use: "RAM", "SRAM", "VRAM", "RTC".
    // NULL if the baseline has none. Read only.
    const uint8_t *(*memory)(void *ctx, const char *name, size_t *size);
    // The CPU's registers, as dj_read_registers lays them out. Returns the bytes written.
    size_t (*registers)(void *ctx, void *out, size_t cap);
};

enum dj_event_kind  { DJ_EVENT_JUMP = 1, DJ_EVENT_CONSOLE_RESET, DJ_EVENT_BREAK,
                      DJ_EVENT_FRAME_END };
enum dj_jump_reason { DJ_JUMP_POWER_ON = 1, DJ_JUMP_STATE, DJ_JUMP_COPY_AGAIN };
enum dj_verdict     { DJ_PENDING = 0, DJ_MATCH, DJ_MISMATCH, DJ_UNJUDGED, DJ_HALT };

#define DJ_PREROLL 1u       // in `flags`: this frame is pre-roll (below)

struct dj_event {
    uint32_t size;          // sizeof(struct dj_event), so fields can be added later
    uint32_t kind;          // enum dj_event_kind
    uint32_t flags;
    uint32_t breakpoint;    // DJ_EVENT_BREAK: the core's own id
    uint32_t jump_reason;   // DJ_EVENT_JUMP: enum dj_jump_reason
    const char *segment;    // the segment's id
    uint64_t    frame;
    const struct dj_baseline *baseline;   // valid during the call
};

// Returns an enum dj_verdict. `reason` is text for unjudged, mismatch and halt.
int dj_handle_event(const struct dj_event *ev, char *reason, size_t reason_len);

struct dj_breakpoint {
    uint32_t    id;       // the core's own number, handed back in the event
    const char *region;   // the baseline connector's region: "PRG", "RDRAM"
    uint64_t    offset;
    const char *name;     // for dj's messages
};

// Optional. Called once, after the game is loaded. Returns how many there are.
size_t dj_breakpoints(const struct dj_breakpoint **out);

struct dj_write {
    uint32_t size, flags;
    const char *segment;
    uint64_t    frame;
    const struct dj_baseline *baseline;
    const char    *region;      // what was written: "RAM", "SRAM"
    uint64_t       offset;
    const uint8_t *bytes;
    size_t         len;
    const char    *at_region;   // the instruction it was made at, or NULL if the line had none
    uint64_t       at_offset;
};

// Optional. A [WRITE] line was applied to the baseline. Returns an enum dj_verdict.
int dj_handle_write(const struct dj_write *w, char *reason, size_t reason_len);
```

**When each is sent.** The frame an event carries is the one about to run, running, or
just run.

| Event | When | The baseline is |
| --- | --- | --- |
| jump | Before the first other event after the baseline's state was replaced (a seek, a snapshot, a power-on); and with the reason "copy again" after a halt, after a write the core can't follow, and for **Sync now** | Between frames |
| console reset | A `[RESET]` line was applied to the baseline | Between frames |
| break | The baseline is about to run the instruction at one of the core's breakpoints | Inside `retro_run` |
| frame end | The baseline's `retro_run` returned | Between frames |
| a write (`dj_handle_write`) | A `[WRITE]` line was applied: before the frame, or at the line's instruction | Between frames, or inside `retro_run` |

Between frames they come in the order jump, console reset, write. A segment playing
straight on into its child sends nothing: no state was replaced.

**Verdicts.** dj takes the baseline's coverage flags once a frame and holds them until a
verdict says what they are worth.

| Verdict | Meaning | What dj does |
| --- | --- | --- |
| pending (0) | Nothing to say, and nothing unvouched for | Nothing |
| match | What ran since my last verdict has parity | Credits the frames it holds |
| mismatch | It does not | The first of a run: keeps a failure and stops playback at the frame's end. Later ones, with no match between: shown, and playback goes on |
| unjudged | I can't vouch for what ran since my last verdict (a lag frame, an overlay not decompiled, or I have only just copied the baseline) | Drops the frames it holds, and this frame if the event is inside it |
| halt | I can't go on | Keeps a failure, stops playback, and sends nothing until a jump. Playing on sends one, "copy again" |

- **The core is not synced by dj.** After a jump it answers pending until it reaches an
  event where it can copy what it needs through the handle. A core that judges at frame
  ends can copy on the jump itself, and the frame that follows is checked. One that judges
  at a breakpoint waits for it.
- **The event at which it copies answers "unjudged":** what ran since the jump was compared
  with nothing.
- **After a mismatch the core is not touched.** It may carry on from its own state, or
  copy the baseline again; a mismatch may put itself right.
- **Coverage is credited by whole frames.** A verdict inside a frame speaks for the frames
  before it; the frame it is in goes with the next one. So a core that judges inside the
  frame gets its credit one verdict late, and the last frame or two of a segment, after
  its last verdict, are credited to nothing.
- **A write.** A core parked where the write lands applies the bytes and answers pending:
  one at a frame boundary for a line with no instruction, one with a breakpoint at the
  line's instruction otherwise (the write comes before that break). Any other copies the
  baseline again. A core without `dj_handle_write` is sent a jump, "copy again", in its
  place.
- **A console reset** is the console's soft reset, which the game has logic for. A core
  parked at a frame boundary runs its own reset path and stays in parity; one parked
  inside a frame can't, and copies the baseline again.
- **The picture.** The core calls the video callback whenever it has finished one, during
  any event; dj shows the latest when the baseline's frame ends. `GET_AUDIO_VIDEO_ENABLE`
  says whether one is wanted, so a core with costly graphics can skip drawing.
- **What differed** for a mismatch or a halt comes from `dj_parity_diffs`, as [below](#parity).

#### Pre-roll

A core that copies the baseline at a breakpoint can't judge the frames between a jump and
that breakpoint. Under `dj parity` every segment starts with a jump, so its first frame
or two would never be checked. `preroll = N` in the core's `[[compare]]` entry
([config.md](config.md#compare-optional-repeatable)) has dj start N frames earlier:

- **Where.** The jump event is sent N frames before the place asked for, in the segments
  above when the place is a segment's start, and never before a root segment's start. dj
  loads the nearest snapshot at or before that frame and plays to it first, with nothing
  sent, so the jump is at the same frame on every machine.
- **What the core sees.** Every event of those N frames has `DJ_PREROLL` in `flags`. The
  core does what it always does: copies when it can. It may skip its compare.
- **What dj does with the verdicts.** Nothing, except for a halt: no failure is kept for a
  mismatch, no coverage is credited, and the core's verdicts start afresh at the place
  asked for. A halt is a failure at the frame it happened in, and the place asked for is
  not checked.
- **When.** `dj parity`, at each segment's start; a failure played again (`dj parity rerun`,
  `explain`), which marks the same frames; and a seek in dj-gui while Check parity is on,
  inside the segment shown. Not when playback carries straight on from a parent into its
  child, and not for "copy again": nothing was loaded.
- **A failure records it:** where its run started (the jump) and how many frames from
  there were pre-roll.

The test core in `crates/dj-core/tests/testcore/` takes events in three ways (at frame ends, at a
breakpoint inside the frame, and copying at every event), and `crates/dj-core/tests/events.rs`
drives it through each case.

### The helper

After a jump a core copies what it needs from the baseline and answers "unjudged" for that
event; from then on it runs itself up to each event it judges at, and compares. That is the
same few lines in every core that judges at one place, so `dj_compare.h` has them, as
`dj_helper_event`. It is not part of the API: a core that needs more writes its own
`dj_handle_event`.

```c
#include "dj_compare.h"

static int copy(void *user, const struct dj_event *ev, char *reason, size_t len);    // 0: copied
static int run(void *user, const struct dj_event *ev, char *reason, size_t len);     // DJ_PENDING: go on and compare
static int compare(void *user, const struct dj_event *ev, char *reason, size_t len); // the verdict

static struct dj_helper helper = { .judge_at = 0, .copy = copy, .run = run, .compare = compare };

int dj_handle_event(const struct dj_event *ev, char *reason, size_t len) {
    return dj_helper_event(&helper, ev, reason, len);
}
```

- **`judge_at`** is 0 to judge at frame ends, or the id of the breakpoint to judge at.
- **Judging at frame ends,** the helper copies on the jump itself, so the first frame after
  it is checked. Judging at a breakpoint, it waits for the breakpoint.
- **`copy`** takes over from the baseline (`dj_baseline_memory(ev->baseline, "RAM", size)`
  gives a region). It is also offered a power-on jump, where a core may boot itself.
  Returning non-zero says "not here", and it is asked again at the next judging event.
- **A halt** from `run` or `compare` leaves the core stale: it copies again when dj sends
  the next jump. After a mismatch it is left as it is.
- **`live`** is whether the core holds the baseline's state. Set it to 0 where the core
  loses the baseline some other way, such as a write it can't follow.

The Tetris decomp (`examples/tetris/.decomp/libretro/tetris_libretro.c`) is built on it. It
judges at a breakpoint where the original starts waiting for the next frame, and has a
core option to judge at frame ends instead.

### On the N64

What a comparison core for the Mupen64Plus connector has to know. The No Mercy stub in
[connectors/mupen64plus/stub/](../connectors/mupen64plus/stub/README.md) does each of
these in a few lines.

- **RAM is 32-bit words in the host's byte order.** The byte at address A is at A ^ 3 on
  a little-endian machine. Read whole words and take bytes from them.
- **Other code can sit at a breakpoint's address.** Games load overlays over one another.
  The list of breakpoints is fixed, so check that a hit is the code you mean (the
  instruction there, or the caller's return address) and answer "pending" when it isn't.
  A breakpoint in code that is always loaded needs no check.
- **The registers** are the PC, then r0 to r31, 64 bits each in the host's order. r29 is
  the stack pointer and r31 the return address.
- **A logic tick often does not line up with a frame.** In a four-player match of No Mercy
  about half the ticks start in one frame and end in the next. Judge at the tick's
  breakpoint, not at frame ends.
- **Leave the framebuffers out of what you compare** when the GPU copies the picture into
  RAM (`mupen64plus-EnableCopyColorToRDRAM`): what it copies depends on what ran before.

### Parity

**What a match is** is the core's to decide, which matters for a port whose memory isn't
laid out like the original's: compare watched values, a RAM map, or the picture.

**What differed.** A reason string says one thing. A port that knows more can list every
difference it found, in the original game's terms, through one more optional function:

```c
struct dj_parity_diff {
    const char    *label;    // optional plain-text name: "Player 2 x coordinate"
    const char    *region;   // the original's region: "RAM", "SRAM" (NULL: no one place)
    uint64_t       address;  // offset in that region
    const char    *symbol;   // optional: a data symbol of the original's, shown beside the address
    const uint8_t *expected; // the original's bytes (may be NULL)
    const uint8_t *actual;   // the port's bytes (may be NULL)
    size_t         len;
};

// Called after the core answered mismatch or halt: fill up to `cap` differences and return
// how many there are in all. Pointers need to stay valid until the core's next verdict.
size_t dj_parity_diffs(struct dj_parity_diff *out, size_t cap);
```

dj keeps the first 100 with the failure and counts the rest. They show in dj-gui's failure
message, in `dj parity`'s output and in the MCP tools. A difference with a region and an
address can be traced: `dj parity explain` ([cli.md](cli.md#parity)) finds the original's
last instruction that wrote it, which is the function to fix. For a verdict given at a
breakpoint it looks no further than that breakpoint. A label alone (a value the
port keeps in no one place, like a checksum) is shown but can't be traced. The region names
are the baseline connector's (`"RAM"` for Mesen's internal RAM, `"RDRAM"` on the N64). A
`symbol` is shown beside the address and doesn't replace it; given alone, with no region,
it can't be traced, since dj doesn't resolve it to an address yet. A core without `dj_parity_diffs` gives none; the RAM compare
of a `sync = "state"` core lists one difference per run of differing bytes. The test core
in `crates/dj-core/tests/testcore/` has one, and so do the Tetris decomp (one per differing byte
of RAM) and the No Mercy stub.

**A failure names the event it answered.** dj keeps, with each failure, the `name` of the
breakpoint the core was called at, or "the frame end", and the tools show it beside the
frame. Give breakpoints names a person can find in the original's source. A mismatch or a halt stops playback at that frame
([gui.md](gui.md#comparison-cores)).

### Checking parity in CI

`dj parity check` checks the chosen segments against the comparison core and exits 1 when
one fails, so a port's CI can gate pull requests on it. [ci.md](ci.md) has the whole job:
getting the ROM from a secret, choosing segments by tag, seeding states to start from, the
JUnit report, and GitHub Actions and GitLab examples. dj doesn't build the port: the job
builds it and names it in a `[[compare]]` entry.

## Writing one for another core

1. Find where the core fetches an instruction byte, and where it can turn a CPU address into a ROM offset (Mesen's mappers already do: `BaseMapper::GetAbsoluteAddressAndType`).
2. Set the opcode flag on the first byte of each instruction and the operand flag on the rest. At the same place, test the breakpoints and call the frontend when one is hit.
3. Expose the functions above through the proc address callback.
4. Check that state hashes are identical with and without the patch, on every segment of a project, and measure the slowdown (about 5% for Mesen). With a symbol map, `dj connector-check` compares the flags with the decoder ([cli.md](cli.md)). A disagreement can also mean data inside a function that annotations.toml doesn't mark yet, such as inline arguments after a `JSR`.
   Save states need a look too. A core may have to run on a little to reach a point where it can save: mupen64plus-next runs to its next interrupt when it is not at a frame's end, so a save taken right after a load or another save moves its memory, and the frame's state hash with it. What plays afterwards is the same, but the same frame would hash two ways. dj therefore keeps the state it last saved or loaded and gives it back, without asking the core, for as long as nothing has run or been written.
5. Pin the upstream commit and ship a `build.sh`, your own files under `src/` and a patch that hooks them into the core under `connectors/<core>/`, like Mesen's.
6. If dj has no decoder for the CPU, write one ([decoders.md](decoders.md)): a plugin, or exported by the core itself (`dj_decoders`).

Rolling coverage up into functions and blocks also needs a decoder for the region's `cpu`.
The 6502, MIPS (`mips3`), ARMv4T (`armv4t`: ARM and Thumb, for the GBA) and the 65816 (`65816`, for the SNES) are built in ([coverage.md](coverage.md#symbol-maps)), and a decoder plugin can add any
other ([decoders.md](decoders.md)); without one, the report counts opcode bytes and says no
decoder exists yet.
