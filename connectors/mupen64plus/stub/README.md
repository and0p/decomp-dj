# A stub comparison core for WWF No Mercy

`nomercy_stub.c` is a comparison core ([docs/connectors.md](../../../docs/connectors.md#comparison-cores))
with the game left out. It tests breakpoints inside the frame on the Mupen64Plus connector,
with a real game whose logic ticks do not line up with frames. It is for WWF No Mercy (USA,
Rev 1) only: the addresses are that ROM's.

What it does:

- **At the game's input routine.** A breakpoint where `pads_read` returns (`RDRAM:0x5BD0`,
  in code that is always loaded). The stub copies the pad buffer the game just filled
  (`g_pads`, 48 bytes at `0x8009EDA0`), as a port would take its inputs.
- **At the match's logic tick.** A breakpoint at `match_tick` (`RDRAM:0x114514`). The stub
  counts the tick and answers "match" when the pads it copied are the ones the game holds
  now. This is where it judges, through the helper in `dj_compare.h`.
- **Other code at the tick's address.** `match_tick` is in an overlay, and other code sits
  at its address at other times. The stub takes a hit as the tick only when the return
  address is the match's one call of it (`0x800D9E04`) and the instruction at the call
  site is still `jal match_tick`. Anything else is "pending".
- **Byte order.** The baseline's RAM is 32-bit words in the host's order, so the stub
  reads bytes through whole words.
- **What differed.** `dj_parity_diffs` gives the 48 bytes of each side, in region `RDRAM`
  at the console's address, so `dj parity explain` can name the instruction that wrote
  them.

Build (macOS; use `-shared` and `.so` on Linux):

```bash
cc -O2 -std=c11 -fPIC -fvisibility=hidden -I../../include -dynamiclib -o nomercy_stub_libretro.dylib nomercy_stub.c
```

Use, in a No Mercy project:

```toml
# config.toml
[[compare]]
name = "stub"
content = false

# .local/local.toml
[[compare]]
name = "stub"
path = "/path/to/nomercy_stub_libretro"
```

```bash
dj parity core --compare stub     # its breakpoints, checked against the connector
dj parity check --compare stub
```

Environment variables, for testing:

| Variable | Effect |
| --- | --- |
| `NM_STUB_LOG=<file>` | One line per event the stub acts on: jumps, pad reads, ticks, other code at the tick's address. Two runs that agree write the same file. |
| `NM_STUB_FAULT=<n>` | The stub spoils its copy of the pads at its nth tick, to see a failure. |
| `NM_STUB_DUMP=<file>` | Writes RDRAM at the first real tick, in the console's byte order. |
