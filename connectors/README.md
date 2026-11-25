# Coverage connectors

libretro has no standard way for a core to report which code it ran, so each
supported core gets a small patch (a "connector"). A connector exports these
functions through `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK`, the libretro
mechanism for core-defined extensions:

```c
struct dj_region {
    const char    *name;        /* "PRG", "RDRAM", ... */
    uint8_t       *flags;       /* one flag byte per region byte */
    size_t         size;
    const char    *cpu;         /* instruction set that runs here, or NULL */
    const uint8_t *data;        /* the region's bytes, or NULL */
    int64_t        file_offset; /* where offset 0 lies in the ROM file, or -1 */
};

struct dj_break { int32_t region; int64_t offset; };   /* a breakpoint */
typedef int (*dj_break_fn)(void *user, int region, int64_t offset);

uint32_t dj_api_version(void);                         /* 3 */
size_t   dj_region_count(void);
bool     dj_get_region(size_t index, struct dj_region *out);
void     dj_clear(void);                               /* zero all flags */
void     dj_set_breakpoints(const struct dj_break *list, size_t n, dj_break_fn fn, void *user);
```

- `dj_set_breakpoints` makes the core call the frontend, and wait, each time an
  instruction at one of those places is about to run. dj counts logic ticks this way.
- The full API, with the optional functions (`dj_read_registers`, `dj_forced_options`, the trace hooks, and
  `dj_handle_event` and the rest for comparison cores, whose header is
  [include/dj_compare.h](include/dj_compare.h)), is in
  [docs/connectors.md](../docs/connectors.md#the-api).
- `flags` has one byte per byte of the region: `0x01` executed as an opcode
  (first byte of an instruction), `0x02` executed as an operand, `0x04`-`0x20`
  mode bits defined per instruction set. `0x40` and `0x80` are reserved.
- Offsets are positions **in the region** (for example PRG ROM), never CPU
  addresses, so bank-switched code is counted correctly.
- A core whose recompiler only sees whole blocks may set `0x01` on each block's
  first byte and leave the rest to the player's disassembly. Its README must say so.
- The player works without a connector. It still plays segments and checks hashes,
  but records no coverage.

| Connector | System | Upstream | Status |
| --- | --- | --- | --- |
| [mesen](mesen/) | NES / Famicom | libretro/Mesen | API 3 |
| [mupen64plus](mupen64plus/) | Nintendo 64 | libretro/mupen64plus-libretro-nx | API 3, first version: RDRAM coverage and breakpoints; no trace hook yet |
| [mgba](mgba/) | Game Boy Advance | libretro/mgba | API 3: ROM, IWRAM, EWRAM and BIOS, ARM/Thumb mode bits, breakpoints; no trace hook |
| [mesen-s](mesen-s/) | Super Nintendo | libretro/Mesen-S | API 3: the main 65816's ROM (LoROM, HiROM and the rest), WRAM and SRAM, register-width mode bits, breakpoints, traces; no coprocessors |
