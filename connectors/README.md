# Coverage connectors

libretro has no standard way for a core to report which code it ran, so each
supported core gets a small patch (a "connector"). A connector exports these
functions through `RETRO_ENVIRONMENT_SET_PROC_ADDRESS_CALLBACK`, the libretro
mechanism for core-defined extensions:

```c
struct dj_region { const char *name; uint8_t *flags; size_t size; };

unsigned dj_api_version(void);                         /* 1 */
size_t   dj_region_count(void);
bool     dj_get_region(size_t index, struct dj_region *out);
void     dj_clear(void);                               /* zero all flags and the tick count */
void     dj_set_tick_exec(int region, int64_t offset); /* -1 disables */
uint64_t dj_get_tick_count(void);
```

- API version 2: each region also reports its instruction set (`cpu`), its bytes
  (`data`) and where it lies in the ROM file (`file_offset`). See
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
| [mesen](mesen/) | NES / Famicom | libretro/Mesen | v1 |
