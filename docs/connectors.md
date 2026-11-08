# Coverage connectors

A coverage connector is a small patch to a libretro core that reports which ROM bytes ran.
Without one, a core still plays segments and checks hashes, but records no coverage and
counts no [ticks](playback.md#logic-ticks-draft). The only connector so far is for Mesen
(NES): [connectors/mesen/](../connectors/mesen/README.md).

## What a connector reports

- Executed locations as **offsets into ROM regions**, never CPU addresses, because bank switching maps different ROM bytes to the same CPU address.
- One flag byte per byte of each region: `0x01` executed as an opcode, `0x02` executed as an operand. Flags only ever get set, until the player clears them.
- Region names the player knows: `PRG` for program ROM is the one coverage rolls up. Mesen also reports `RAM`, `WRAM` and `SRAM`.

## The API

The core registers `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK` and returns these
functions by name. The player looks them up after `retro_init`, and treats a core without
them as having no connector.

```c
uint32_t dj_api_version(void);                       // 1
size_t   dj_region_count(void);
bool     dj_get_region(size_t index, struct dj_region *out);
void     dj_clear(void);                             // zero every flag and the tick count
void     dj_set_tick_exec(int region, int64_t offset); // count executions here; region -1 turns it off; resets the count
uint64_t dj_get_tick_count(void);

struct dj_region {
    const char *name;   // "PRG", "RAM", ...
    uint8_t    *flags;  // one flag byte per region byte, owned by the core
    size_t      size;
};
```

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
when the sizes differ, frames aren't judged. `dj_run` lets a port stop with a message (a
crash it caught, an unimplemented function it hit) instead of running on. A mismatch, an
error or a failed `dj_run` stops playback at that frame ([gui.md](gui.md#comparison-cores)).

## Writing one for another core

1. Find where the core fetches an instruction byte, and where it can turn a CPU address into a ROM offset (Mesen's mappers already do: `BaseMapper::GetAbsoluteAddressAndType`).
2. Set the opcode flag on the first byte of each instruction and the operand flag on the rest. Count a tick when the opcode fetch matches the tick location.
3. Expose the functions above through the proc address callback.
4. Check that state hashes are identical with and without the patch, on every segment of a project, and measure the slowdown (about 5% for Mesen).
5. Pin the upstream commit and ship a `build.sh` and the patch under `connectors/<core>/`, like Mesen's.

Rolling coverage up into functions and blocks also needs a sweep for the core's CPU. Only
the 6502 has one so far ([coverage.md](coverage.md#limits)).
