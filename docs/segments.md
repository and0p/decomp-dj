# Segments and project files

This is the shared part of a project: the files people commit and trade. It is
normative for format version 1. Items marked **(reserved)** may appear in files, but
the tools don't act on them yet.

## Project layout

```
my-project/
  config.toml            pinned setup: ROM hash, core build, core options, hash rules
  segments/
    7f3c…e1.segment      one file per segment, named by its id
    0a9b…44.segment
  blobs/
    9a1c…7e.srm          SRAM and other blobs, named by content hash
  symbols/               optional: symbol maps for rolling coverage up into functions
  unreachable.txt        optional: ROM ranges no input can reach, left out of coverage
  .local/                local only, never committed
    local.toml           local settings
    cache.db             the local DB
    drafts/              segments written by agents, not yet saved (see mcp.md)
    traces/ snapshots/   trace files and core states written for agents
    mcp/                 running agent sessions and the GUI's shared view
```

Segments are stored **flat**, named by id. The tree lives in each file's
`parent` header, and tools present it as a tree. This keeps paths short, makes
re-parenting a one-line change, and never creates folders for empty leaves.

Nothing in a project is game data, except blobs a project author chooses
to include (SRAM and snapshots). Whether those are fine to distribute is the
author's call.

Settings are in `config.toml` and `.local/local.toml`, described in [config.md](config.md).

## Segment files

A segment is UTF-8 text with Unix or Windows line endings. It has a header,
a line containing only `---`, and a body. Lines starting with `#` are
comments everywhere. Blank lines are ignored. Tools that rewrite a file keep
every comment, but may move comments in the body to the top of the body.

```
dj-segment 1
id: 3b8f0f2c-1d44-4c1e-9e55-0d7a3c1b2e90
parent: none
title: Boot to title screen
frames: 240
start-frame: 0
sram: 9a1c…
start-hash: 2f4d…
end-hash: 71ee…
---
# frame  command
60  P1 JOYPAD START 1
64  P1 JOYPAD START 0
120 [EVENT] Title screen visible
```

### Header fields

| Field | Required | Meaning |
| --- | --- | --- |
| `dj-segment 1` | yes | First line: the segment format and its version. A dj refuses a newer segment format, naming it; files that start with `dj-replay 1` (the earlier name) are read as format 1 and get the new first line when saved, or all at once with `dj upgrade`. |
| `id` | yes | Lowercase UUID. Never changes once published. The file name is `<id>.segment`. |
| `parent` | yes | Id of the segment that must play immediately before this one, or `none` for a root segment. |
| `title` | no | One line for humans and agents. |
| `frames` | yes | Length in frames. The segment ends after this many frames even if the last input is earlier, so no trailing "wait" lines are needed. |
| `start-frame` | no | Frames since power-on when this segment starts (the sum of its ancestors' `frames`, or the frame recorded with a `snapshot` blob). Written by tools; used to catch mistakes. |
| `snapshot` | root only | Blob id of a snapshot to load before frame 0. (Formerly `save`; tools still read that name.) |
| `sram` | root only | Blob id of battery-backed save RAM to load before frame 0. Use it with or without `snapshot`; when both are given, `sram` is applied after the state loads (for cores whose states leave SRAM out). |
| `start-hash` | no | State hash at frame 0, before any input ([state hashes](playback.md#state-hashes)). |
| `end-hash` | no | State hash after the last frame. |
| `checkpoint` | no | Repeatable: `checkpoint: <frame> <hash>`. The state hash at the start of frame `<frame>`, before that frame's lines apply (so `checkpoint: 0` would equal `start-hash`). Narrows down where a run diverged. Editors check checkpoints while playing and keep the ones before an edit. |
| `tags` | no | Space-separated free tags (`boss`, `rng-forced`, …). The GUI sets them in Properties and filters the tree by them. |

A root segment with neither `snapshot` nor `sram` is a cold boot with no save data.

Unknown header keys are kept by tools and ignored by players, so the format
can grow without breaking older files.

### Frames

A **frame** is one call to the core's `retro_run()`. Body lines start with a
frame number **relative to the start of this segment**, from `0` to
`frames - 1`. Lines must be in non-decreasing frame order; lines on the same
frame apply top to bottom.

Everything on frame `n` is applied before `retro_run()` is called for frame
`n`. So an input set on frame `n` is what the core sees when it polls during
frame `n`.

Every segment starts with all inputs released, whatever its parent was holding
at its end. This keeps a segment's meaning the same whether a player reaches
its start by playing the parent or by loading a cached state.

### Input lines

```
<frame> P<port> <DEVICE> <control...> <value>
```

Input is stored the way libretro asks for it: a core calls
`input_state(port, device, index, id)` and gets back a 16-bit value. The line
names those four things and the value. Inputs are **sparse state changes**: a
value holds until a later line changes it. Pressing and releasing a button is
two lines, which is the "hold start, hold end" model.

| Device | Line | Value |
| --- | --- | --- |
| Joypad | `P1 JOYPAD A 1` | `1` pressed, `0` released. Buttons: `B Y SELECT START UP DOWN LEFT RIGHT A X L R L2 R2 L3 R3`. |
| Analog stick | `P1 ANALOG LEFT X -32768` | Signed 16-bit, `-32768..32767`. Sticks `LEFT RIGHT`, axes `X Y`. |
| Analog button | `P1 ANALOG BUTTON R2 32767` | `0..32767` pressure, using joypad button names. |
| Lightgun | `P2 LIGHTGUN SCREEN_X 1200` | Absolute screen position, `-32767..32767` maps edge to edge (libretro's convention). Also `SCREEN_Y`. |
| Lightgun buttons | `P2 LIGHTGUN TRIGGER 1` | `TRIGGER RELOAD AUX_A AUX_B AUX_C START SELECT DPAD_UP DPAD_DOWN DPAD_LEFT DPAD_RIGHT IS_OFFSCREEN`. |
| Mouse | `P2 MOUSE LEFT 1` | Buttons `LEFT RIGHT MIDDLE BUTTON_4 BUTTON_5`. |
| Mouse motion | `P2 MOUSE X 5` | **Impulse**: relative motion for this one frame only, back to `0` on the next frame. Also `Y`, `WHEELUP`, `WHEELDOWN`, `HORIZ_WHEELUP`, `HORIZ_WHEELDOWN`. |
| Pointer / touch | `P1 POINTER X 0` | Absolute, `-32767..32767`. `X Y PRESSED`, with optional finger index: `P1 POINTER 1 X 0`. |
| Keyboard | `P1 KEYBOARD RETURN 1` | Key names are libretro's `RETROK_` names without the prefix. |
| Anything else | `P1 RAW <device> <index> <id> <value>` | Numbers straight from libretro.h, for devices this table misses or core-specific subclasses. |

Ports are numbered from 1 in files (`P1` is libretro port 0).

For analog sticks, a segment may store exact values or only the 8 directions
plus neutral at full deflection (`±32767`). Both are valid. Exact values are
for games that read the precise angle.

### Controller devices

```
<frame> [DEVICE] P2 MOUSE
```

Sets the device plugged into a port (`retro_set_controller_port_device`).
The device names are those in the input table, or a number for a
core-specific subclass (`RETRO_DEVICE_SUBCLASS(base, id)` = `((id + 1) << 8) | base`;
Mesen's SNES mouse is `770`). Power-on devices come from `config.toml`,
and a `[DEVICE]` change stays in effect for the rest of the segment and for
all of its descendants.

Cores do not always read input from the port a device is plugged into. Mesen,
for example, reads the mouse from port 1 wherever the mouse is. Segments record
what the core actually polls, so such quirks show up in the file; a comment
line saying so helps the next reader.

### Memory writes

```
<frame> [WRITE] <region> <address> <byte> [<byte>...]
```

Writes bytes before the frame runs, starting at `<address>` and going up one
address per byte. Addresses and bytes are hex with or without `0x`. A
contiguous block is one line, however long:

```
0 [WRITE] RAM 0x0300 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10
```

Several writes on one frame apply top to bottom, so a later line wins where
two overlap. Regions:

| Region | libretro source |
| --- | --- |
| `RAM` | `retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM)` |
| `SRAM` | `RETRO_MEMORY_SAVE_RAM` |
| `VRAM` | `RETRO_MEMORY_VIDEO_RAM` |
| `RTC` | `RETRO_MEMORY_RTC` |
| `BUS` | the emulated address bus, through the core's memory maps (`RETRO_ENVIRONMENT_SET_MEMORY_MAPS`) |

A segment with any `[WRITE]` line is **poked**: it reaches a state real play
may not. Tools count its coverage separately and tag it `poked`, so a
project can always say how much coverage comes from play alone.

### Events

```
<frame> [EVENT] Mario lands on the flagpole
<frame> [EVENT @128,96] Missile hits the left silo
```

Annotations for humans and agents. The optional `@x,y` is a pixel position
in the core's video output for that frame. Events change nothing in the
emulation, so adding, editing or removing one never changes a hash. A frame
may carry any number of events. Editors show the event text on the timeline
and group lines that share a frame.

### Reserved commands

| Command | Meaning |
| --- | --- |
| `[RESET]` | Soft reset (`retro_reset`). |
| `[RNG] <name> <n>` | Advance a named RNG `n` times. Game-specific, defined by the project. |
| `[SCREENSHOT]` | Ask tools to keep this frame's video output. |

Players must reject unknown bracketed commands rather than silently skip them,
because skipping a state-changing command would make every hash after it wrong.

## Blobs

`blobs/` holds SRAM and snapshot files, each named by its content hash ([state hashes](playback.md#state-hashes))
and an extension for its kind: `<hash>.srm` for SRAM (libretro's own name for save
RAM files, so other emulators can open them) and `<hash>.state` for a core state.
The header field that names a blob (`sram` or `snapshot`) says which it is. Many segments
share one blob (for example a blank save).
A blob is the exact byte layout the pinned core uses: `retro_serialize`
output for `snapshot`, and the `RETRO_MEMORY_SAVE_RAM` contents for `sram`.

## Older projects

A dj reads every older project format and writes files in its own as it saves them;
`dj upgrade <dir>` rewrites every segment file at once (one commit, nothing about what
they play changing). Layouts from before format 1 are upgraded in place the first time a
tool opens them:

- The local folder used to be called `.emucov/`. It moves to `.local/`, and `.emucov/` becomes `.local/` in the project's `.gitignore`. Its old `coverage.sqlite` and `states/` are no longer used and can be deleted.
- `collection.toml` becomes `config.toml`, `replays/<id>.replay` becomes `segments/<id>.segment`, hashes lose their `blake3-` prefix and keep their first 32 hex digits, and blobs are renamed to `<hash>.srm` or `<hash>.state`.
