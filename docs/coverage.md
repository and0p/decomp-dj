# Coverage

Which parts of the ROM the project's segments run, rolled up into functions and basic
blocks when a symbol map is given. Coverage is not part of the shared files: each machine
works it out by playing the project.

## How it is recorded
Coverage is not part of the shared files. Players record it locally while
segments run, through a per-core **coverage connector** (see
[connectors.md](connectors.md)). The connector reports executed locations as
**offsets into the ROM image**, never CPU addresses, because bank switching
maps different ROM bytes to the same CPU address.

For each segment, the player stores which ROM bytes ran as an instruction's
first byte (opcode) and which ran as operands, during that segment's own frames
only. Ancestors' frames are not counted again.

It keeps this in the local database (`.local/cache.db`, see [database.md](database.md))
keyed by the segment's **content key**, so the totals always describe the
current files: a report counts only clean, saved segments whose current content has
been played, and says how many were dirty, unsaved (drafts, on the command line) or not
played yet. Each segment also
records the frame at which each ROM byte first ran in it, which answers "which
segment reaches this code, and when".

## Unreachable code

`unreachable.txt` at the project root lists ROM ranges that no input can
reach (dead code, debug paths). It is version controlled. One range per line,
`0x8000-0x80FF` or a single `0x8123`, optionally prefixed by a region
(`PRG:0x8000-0x80FF`), with `#` comments. Instructions and blocks inside them
are left out of the totals.

## Symbol maps

Coverage rolls up into basic blocks and functions when a symbol map is given. Bring your
own: dj doesn't find functions itself (no static analysis beyond splitting the functions
you name into instructions and blocks). Without a map, coverage counts the opcode bytes
that ran, and the queries that need functions (`function`, `frontier`, `gaps` by function)
say how to add one. These formats are read:

- `ca65-dbg`: the `.dbg` file from the cc65 linker (`ld65 --dbgfile`). Each
  `.proc` scope becomes a function, and so does each top-level label in a
  segment whose name contains `CODE`, up to the next label. `rom_header`
  (default 16, the iNES header) is subtracted from file offsets to get PRG
  offsets.
- `csv`: one function per line, `name,region,offset,size[,cpu_addr]` (numbers in hex).
- `splat`: splat's `symbol_addrs.txt`, the symbol list N64, PS1 and GameCube decomps
  edit as they name functions: `name = 0x80001234; // type:func rom:0x1234 size:0x40`.
  Every entry with a `rom:` offset counts, unless its `type:` is something other
  than `func` or it says `ignore:true`. The address becomes the CPU address.
- `mesen-mlb`: a Mesen label file, as Mesen's debugger writes it while you name
  things: `P:1F3A:name[:comment]`, or `NesPrgRom:1F3A:name` from Mesen 2. Offsets
  are already relative to PRG. Only single-address PRG ROM labels count; ranges
  (`P:0200-020F:table`) are data.
- `ld-map`: a GNU ld map file (`-Map=`) from building the decomp. Symbols in input
  sections named `.text*` are functions. The ROM offset is the symbol's address moved
  from its output section's address to the section's `load address` (the `AT()` that
  decomp linker scripts use for ROM placement).
- `elf`: the ELF the decomp builds, 32- or 64-bit. Its `STT_FUNC` symbols are functions,
  with their sizes. The ROM offset is the address moved from the program segment's
  virtual address to its physical (load) address.

Formats that give no size (`mesen-mlb`, `ld-map`, and `splat` entries without
`size:`) run each function up to the next symbol. That can include data between
functions, which then counts as uncovered code, so totals from label-only formats run
lower than from formats with real sizes. `unreachable.txt` can carve such data out.

Where each map's offsets land in the ROM is set per entry with `rom_header`, `region` and `cpu_base` ([config.md](config.md#symbols-optional-repeatable)).

A project can list several symbol maps. Where two describe the same bytes, the first one
listed wins, so a linker map can come first and a hand-edited `symbol_addrs.txt` can fill
in what isn't matched yet. `dj init --symbols FILE` picks the format from the file's
name. Only `PRG` functions are rolled up for now, with the 6502 sweep below; other
regions and CPUs come with their connectors.

For 6502 code the player splits each function into instructions and basic
blocks by a linear sweep (a block ends at a branch, jump, return or `BRK`, and
starts at a branch or jump target). A block counts as covered when its first
instruction ran. Data placed inside a function's range can confuse the sweep.

## Gap lists

Coverage can be exported as a gap list (functions and blocks no segment
reaches) to show collaborators where help is needed.

## From the command line

```sh
dj report <dir> [--json FILE] [--gaps FILE.csv]   # coverage of the current, clean segments
dj summary  <dir>                                 # totals, and the segments that add most
dj xref     <dir> ADDR                            # which segments first run it, at which frame
dj gaps     <dir> [--sort size|nearness] [--only never|partial]
dj function <dir> ADDR                            # blocks, callers, segments that enter it
dj frontier <dir> [ADDR]                          # covered branches into uncovered code
dj snapshot <dir> SEGMENT FRAME --out FILE        # the core state there, to start a trace from
```

- `report` reads the local database, not the files: it counts only clean segments whose current content has been played into it, and lists how many were dirty or not played yet. Drafts are left out, with what they would add once saved; so is what dirty segments ran the last time each played ("+N if cleaned"). Run `dj clean` first ([database.md](database.md)).
- `--gaps` writes the gap list as CSV.
- `xref` (also `where`) takes an address (`PRG:0x1234`, `0x1234` for a PRG offset, `$C123` for a CPU address, or a symbol, optionally `name+0x10`) and lists the segments where that code first runs, best first, with the frame and the nearest snapshot. `snapshot` then writes the core state at that frame, to start a trace from.
- `gaps --sort nearness` puts first the functions whose uncovered code a covered branch skips: the cheapest to reach. `frontier` lists those branches with the segments and frames that run them; given an uncovered address, it lists the ways into its block. Both need a symbol map; without one, on a NES ROM, `gaps` lists the uncovered PRG byte ranges instead, largest first, and `function` and `frontier` say how to add a map ([mcp.md](mcp.md#without-a-symbol-map)).
- Every query takes `--json`, `--limit` and `--cursor`. An agent gets the same queries, and can trace and play too, through [`dj mcp`](mcp.md).

## In the GUI

**Coverage map.** Always at the bottom of the left sidebar, whatever the tab; drag its top
edge to resize it. There is one square per function in ROM order (about 1,024 equal slices
of PRG without a symbol map), so a square never moves as coverage grows. Tabs over it choose
what it shows: **Combined** (at launch: coverage, with parity's purple, orange and red over
it once a comparison core is loaded), **Coverage**, **Parity** ([gui.md](gui.md#parity)) and
**Hidden** (only the tabs). The colour key under the map lists only the colours some square
shows. The function list's squares use the same colours. The coverage colours, in order of
precedence where several segments run the same code:

- **Green:** run by a clean, saved segment. Counted. A square goes from grey to green as more of the function is covered.
- **Yellow-green:** new, not counted yet: run by an unsaved segment (your edits, hashes recorded but not saved, or an agent's draft) or by the pass being played, and by no saved clean segment. It counts once saved and clean. Unsaved code a saved segment already runs is just green.
- **Dark, faded green:** run only by dirty segments, from the last time each played (found in the database, without playing anything). Not counted; it comes back when they are clean again.
- **Grey:** never run. **Blue:** all of it is unreachable or data in [annotations.toml](config.md#annotationstoml); the key calls both "unreachable".

The totals show the yellow-green and faded parts apart ("+N unsaved", "+N if cleaned").
Clicking a square opens that function in the Functions tab.

**Live coverage.** While a segment plays, every square whose code runs glows and fades over
half a second, so code that runs every frame stays lit. Code that runs while playing but
that no counted segment runs is new: yellow-green on the map and the bars, with a white
outline on squares that gain some, and counted as "+N new in this segment, counted once
saved". The count covers what the selected segment runs that no counted segment does: in
the play now, and in its last full pass if it is unsaved, so it stays the same once the
pass reaches the end and is recorded in the database ([database.md](database.md#in-the-gui)).
The live part starts again when you select or edit a segment, undo or redo a change to it,
and when playback continues into a child. It counts towards the project once the segment
is clean and saved.

**Functions tab.** The project's coverage from the local database, the same numbers as
`dj report`, and a function list that sorts by ROM order, most uncovered, least covered or
name. Click a row (or a square on the map) to see the function's uncovered blocks and the
segments where it runs, each with the first frame it runs there; click one to go to that
segment and frame. The line reads "Runs in N segments; nearest the top: <segment>, from
frame F", naming the best one (clean before dirty, then the shallowest segment, then the
earliest frame, the order `xref` uses); "All N" opens every one in a scrolling list, with a
filter by segment title when there are more than 20. The
report reads the segments in the editor, saved or not, so deleting an unsaved segment takes
its coverage with it. It is rebuilt after any edit, undo, save, finished pass or clean, which takes milliseconds at NES
sizes. Coverage needs the local database to be on (Project > Use the local database).

With no symbol map, the totals count opcode bytes and the function list is replaced by a
short panel: the formats above, where to get each, an example `[[symbols]]` entry, and
**Add symbol map…**. That picks a file, takes the format from its name (`.dbg`, `.mlb`,
`.map`, `.elf`, `.csv`, `symbol_addrs.txt`; anything else is refused, so add the entry by
hand), copies it into `symbols/` if it's outside the project, appends a `[[symbols]]`
entry to config.toml (the rest of the file stays as written) and rebuilds the coverage.

**Tree badges.** In the Segments tab, "+N" after a segment is how many instructions only
that segment runs.

## Limits

- Only one core has a connector (Mesen, NES).
- Block and function rollup is 6502-only, and only for `PRG`. Other CPUs get raw executed-byte counts until their disassembler is added.
