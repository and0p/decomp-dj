# Coverage connectors

A coverage connector is a small patch to a libretro core that reports which ROM bytes ran.
Without one, a core still plays segments and checks hashes, but records no coverage and
counts no [ticks](playback.md#logic-ticks-draft). There are connectors for Mesen (NES),
[connectors/mesen/](../connectors/mesen/README.md), mGBA (GBA),
[connectors/mgba/](../connectors/mgba/README.md), and, in a first version, Mupen64Plus-Next
(N64), [connectors/mupen64plus/](../connectors/mupen64plus/README.md).

## What a connector reports

- Executed locations as **offsets into ROM regions**, never CPU addresses, because bank switching maps different ROM bytes to the same CPU address.
- One flag byte per byte of each region (below). Flags only ever get set, until the player clears them.
- For each region: its name, the instruction set that runs there, its bytes, and where it lies in the ROM file. The connector is the source of these facts; projects don't configure them ([.agents/CONNECTOR_PLAN.md](../.agents/CONNECTOR_PLAN.md) has the reasoning).
- **Region names are a contract per system**, so that two cores for one system can share a project's coverage, annotations and symbol maps. NES: `PRG` (program ROM), `RAM`, `WRAM`, `SRAM`. N64: `RDRAM` (main RAM, by physical address, where code runs) and `ROM` (the cartridge, `.z64` order). GBA: `ROM` (the cartridge, from byte 0 of the file, its address mirrors folded together), `IWRAM`, `EWRAM` and `BIOS`. Coverage rolls up into the first region from the ROM file whose instruction set has a decoder (`PRG` on the NES), else into the first region a CPU runs (`RDRAM` on the N64).

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
| `armv4t` | 17 values of 32 bits in the host's byte order: r0 to r15, then CPSR. r15 is as the CPU holds it, two instructions past the one about to run |

`dj_build_id` names the build exactly and the same way on every platform: the upstream
commit and the connector patch, as `<core>@<commit, 8 hex digits>+dj.<the patch's sha256
with \r removed, 8 hex digits>` (`mesen@0102910c+dj.34266c1a`). Each connector's
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
    uint8_t  reserved[4];
};
```

Events past the capacity are counted in `dropped` rather than stored. Only the Mesen
connector has the hook so far.

## Comparison cores

A [comparison core](gui.md#comparison-cores) (a decomp built as a libretro core, say) runs
beside the baseline on the same input lines. These functions are optional, found by name
like the rest: through the core's proc address callback, then its library exports.

```c
struct dj_baseline {
    uint32_t version;                                // 1
    void    *ctx;
    void    *(*get_proc)(void *ctx, const char *name);
};

#define DJ_SYNC_BOOT 1

// Bring this core to the baseline's state at the start of power-on frame `frame`.
// 0: synced. 1: can't match it here (fill in `reason`). Anything else: an error.
int dj_sync(const struct dj_baseline *baseline, uint64_t frame, uint32_t flags,
            char *reason, size_t reason_len);

// Follow a [WRITE] line: `len` bytes at `address` in the baseline's region `region`.
// 0: done. 1: ignored on purpose. Anything else: an error (fill in `reason`).
int dj_write(const struct dj_baseline *baseline, const char *region, uint64_t address,
             const uint8_t *bytes, size_t len, char *reason, size_t reason_len);
```

`get_proc` returns any function of the baseline core by name: the libretro API
(`retro_get_memory_data`, `retro_get_memory_size`, `retro_serialize`, ...) and its own
connector functions. With them `dj_sync` reads whatever it needs from the baseline, in
whatever way suits the port: a RAM map, a few watched values, or the save state. The handle
is only good for the length of the call.

A comparison core runs in dj's own process, in the GUI and under `dj parity`
([gui.md](gui.md#comparison-cores)), and the handle points at the baseline core itself.
Reading through it copies nothing. **A comparison core must only read the baseline:** a
write through the handle changes the baseline, and its state hashes with it, and nothing
reports that.

**When the player syncs.** After every jump (a seek, a step back, choosing a segment,
switching the comparison on), just before the first frame it shows. While the playhead is
held (dragging the ruler) it waits for the button to be let go. Playing straight on,
including from a segment's end into its child, needs no sync. A failed sync is tried again
at the next jump, or with **Sync now**.

**`DJ_SYNC_BOOT`** marks a fresh power-on: frame 0 of a root segment that doesn't start from
a snapshot. The baseline has only just booted, so a port may prefer to boot itself (read
its own SRAM, skip a config menu) and line up from there, rather than copy the baseline.

**Inputs.** Held buttons aren't part of a save state: after a sync the player sets the
comparison core's ports and inputs as the segment has them at that frame, as it does for
the baseline.

**`[WRITE]` lines** go to `dj_write`. A core with `sync = "state"` takes them straight into
its own memory. A `dj_sync` core without `dj_write` can't follow one, so the player syncs it
again on the next frame shown.

The comparison core's coverage functions are ignored for now. A test core with these
functions is in `player/tests/testcore/`.

### Events

A comparison core may take **events** instead of `dj_sync`, `dj_write`, `dj_run` and
`dj_check_parity`. It is then a kind of debugger client: it says where in the original it
wants breakpoints, dj calls it each time the baseline gets to one, with the baseline
stopped and readable, and the core runs itself, compares, and answers with a verdict. dj
does not sync it or run its frames. A core that exports `dj_handle_event` is driven this
way; the functions above are still read for a core that doesn't.

```c
struct dj_baseline {                 // version 2: read the baseline during a call
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

#define DJ_PREROLL 1u       // in `flags`: this frame is pre-roll (not sent yet)

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

The test core in `player/tests/testcore/` built with `-DEVENTS` takes events, in three ways
(at frame ends, at a breakpoint inside the frame, and copying at every event), and
`player/tests/events.rs` drives it through each case.

### Parity

```c
// Called after each frame both cores ran from a synced state, if Check parity is on.
// 0: match. 1: mismatch (fill in `reason`). 2: skip, not judged here (fill in `reason`).
// Anything else: an error. `frame` is the frame just run, counted from power-on.
int dj_check_parity(const struct dj_baseline *baseline, uint64_t frame,
                    char *reason, size_t reason_len);

// Called instead of retro_run when the core has it: run one frame (call retro_run, or your
// own loop). 0: done. Anything else stops playback with `reason` shown.
int dj_run(char *reason, size_t reason_len);
```

`dj_check_parity` decides what a match is, which matters for a port whose memory isn't laid
out like the original's: compare watched values, a RAM map, or the picture. A core without
it is checked by comparing both cores' system RAM (`retro_get_memory_data`) byte for byte;
when the sizes differ, frames aren't judged.

**What differed.** A reason string says one thing. A port that knows more can list every
difference it found, in the original game's terms, through one more optional function:

```c
struct dj_parity_diff {
    const char    *label;    // optional plain-text name: "Player 2 x coordinate"
    const char    *region;   // the original's region: "RAM", "SRAM" (NULL: no one place)
    uint64_t       address;  // offset in that region
    const char    *symbol;   // optional: a data symbol of the original's, instead of an address
    const uint8_t *expected; // the original's bytes (may be NULL)
    const uint8_t *actual;   // the port's bytes (may be NULL)
    size_t         len;
};

// Called after dj_check_parity said mismatch: fill up to `cap` differences and return how
// many there are in all. Pointers need to stay valid until the next dj_check_parity call.
size_t dj_parity_diffs(struct dj_parity_diff *out, size_t cap);
```

dj keeps the first 100 with the failure and counts the rest. They show in dj-gui's failure
message, in `dj parity`'s output and in the MCP tools. A difference with a region and an
address can be traced: `dj parity --explain` ([cli.md](cli.md#parity)) finds the original's
last instruction that wrote it, which is the function to fix. A label alone (a value the
port keeps in no one place, like a checksum) is shown but can't be traced. The region names
are the baseline connector's (`"RAM"` for Mesen's internal RAM), and `symbol` isn't
resolved to an address yet. Without `dj_parity_diffs`, the RAM compare lists one difference
per run of differing bytes; a core with `dj_check_parity` and no `dj_parity_diffs` gives
none. The test core in `player/tests/testcore/` has one. `dj_run` lets a port stop with a message (a
crash it caught, an unimplemented function it hit) instead of running on. A mismatch, an
error or a failed `dj_run` stops playback at that frame ([gui.md](gui.md#comparison-cores)).

### Checking parity in CI

`dj parity` ([cli.md](cli.md#parity)) checks every clean segment against the comparison
core and exits 1 when a frame fails, so a decomp's CI can run it after building its core.
dj doesn't build the decomp: the job builds it, names it in a `[[compare]]` entry, and
runs `dj parity`. A sketch for GitHub Actions, for an NES decomp checked against Mesen:

```yaml
name: parity
on: [push, pull_request]

jobs:
  parity:
    runs-on: ubuntu-22.04
    env:
      DJ: decomp-dj-0.1.0-linux-x86_64
    steps:
      - uses: actions/checkout@v5

      # dj, from a release ZIP. Its connectors/ folder builds the baseline core.
      - name: Get dj and the baseline core
        run: |
          curl -sSLO https://github.com/and0p/decomp-dj/releases/download/v0.1.0/$DJ.zip
          unzip -q $DJ.zip
          bash $DJ/connectors/mesen/build.sh "$PWD/cores"

      # Your decomp, built as a libretro core with dj_sync (and dj_check_parity,
      # dj_parity_diffs if it has them).
      - name: Build the decomp's core
        run: make libretro && cp build/mygame_libretro.so cores/

      # The original ROM. Never commit one you may not share: a secret (base64), a
      # self-hosted runner, or a free game's ROM built from its source.
      - name: Get the ROM
        run: echo "${{ secrets.ROM_BASE64 }}" | base64 -d > game.nes

      # Paths for this machine. A [[compare]] entry in config.toml can be shared, with
      # its path given here (entries are merged by name).
      - name: Point dj at them
        run: |
          mkdir -p .local
          cat > .local/local.toml <<'TOML'
          rom = "game.nes"
          core = "cores/mesen_libretro"
          [[compare]]
          name = "decomp"
          path = "cores/mygame_libretro"
          TOML

      - name: Check parity
        run: $DJ/dj parity . --compare decomp

      # What failed and what differed, for reading in the run's artifacts.
      - name: Report
        if: failure()
        run: |
          $DJ/dj parity . --compare decomp --report --json > parity-report.json
          for id in $(jq -r '.failures[] | select(.current_build) | .id' parity-report.json); do
            $DJ/dj parity . --compare decomp --explain "$id" --json > "explain-$id.json" || true
          done
      - uses: actions/upload-artifact@v5
        if: failure()
        with:
          name: parity
          path: "*.json"
```

- **A fresh database each run** checks every segment. That's the point in CI: the build is
  new anyway, and results are kept per build (the library's sha256). Caching `.local/`
  between runs (`actions/cache`) saves replaying parents, nothing more.
- **Segments must be clean** (their end hashes recorded in the segment files): dirty ones
  are left out and listed. Clean them locally (`dj clean`) and commit.
- **Exit status:** 0 when every frame passed, 1 when one failed, 2 on an error (the job
  fails either way). A crash or hang in the decomp's core is a failure at that frame, not an
  error. `dj parity` plays in a child process that it watches: when the decomp's core takes
  the child down, `dj parity` reads which core it was, records the failure at that frame,
  keeps the frames that passed before it, and goes on to the segments below.
- **`--explain`** needs the trace hook in the baseline's connector, which Mesen's has; it
  replays each failure, so leave it out for long segments or many failures.
- Comparing two builds of the same emulator needs no decomp: `sync = "state"` in the
  `[[compare]]` entry ([config.md](config.md#compare-optional-repeatable)).

## Writing one for another core

1. Find where the core fetches an instruction byte, and where it can turn a CPU address into a ROM offset (Mesen's mappers already do: `BaseMapper::GetAbsoluteAddressAndType`).
2. Set the opcode flag on the first byte of each instruction and the operand flag on the rest. At the same place, test the breakpoints and call the frontend when one is hit.
3. Expose the functions above through the proc address callback.
4. Check that state hashes are identical with and without the patch, on every segment of a project, and measure the slowdown (about 5% for Mesen). With a symbol map, `dj check-connector` compares the flags with the decoder ([cli.md](cli.md)). A disagreement can also mean data inside a function that annotations.toml doesn't mark yet, such as inline arguments after a `JSR`.
5. Pin the upstream commit and ship a `build.sh` and the patch under `connectors/<core>/`, like Mesen's.
6. If dj has no decoder for the CPU, write one ([decoders.md](decoders.md)): a plugin, or exported by the core itself (`dj_decoders`).

Rolling coverage up into functions and blocks also needs a decoder for the region's `cpu`.
The 6502, MIPS (`mips3`) and ARMv4T (`armv4t`: ARM and Thumb, for the GBA) are built in ([coverage.md](coverage.md#symbol-maps)), and a decoder plugin can add any
other ([decoders.md](decoders.md)); without one, the report counts opcode bytes and says no
decoder exists yet.
