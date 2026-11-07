# The GUI

`dj-gui` is a desktop viewer and editor for projects, built with egui. It loads one
libretro core, shows the segment tree, plays and seeks with live video, and edits button
inputs on a piano roll. It is a prototype.

```sh
gui/target/release/dj-gui examples/thwaite       # or File > Open project…
```

The ROM and core come from the project's `.local/local.toml` (written by the CLI's
`--rom` and `--core`), or set them with Emulation > Set ROM… and Set core…. Building is in
[install.md](install.md).

## The window

| Area | What it holds | Details |
| --- | --- | --- |
| Left sidebar | Tabs for **Segments**, **Markers** and **Coverage**, with the coverage map below them | Below, and [coverage.md](coverage.md#in-the-gui) |
| Top | The emulator picture, with the transport bar below it | [Transport](#transport) |
| Bottom | The piano roll and marker lane | [editing.md](editing.md#piano-roll) |
| Bottom edge | The status bar: messages on the left; the unsaved count, the core and the MCP state (off, or agent sessions running, following, sharing) on the right. Long seeks, loads and cleans show a progress bar there. | |

## Segments tab

The segment tree. Dots show clean (green) and dirty (orange); anything under a dirty
segment is dirty too ([playback.md](playback.md#clean-and-dirty)). A **!** after the dot
means the segment's ending changed since it was last saved: playing or cleaning recorded a
different end hash. Save keeps it; undo takes it back. The header counts clean and dirty
segments and changed endings.
Unsaved segments (edited here, or an agent's draft) are yellow italics until saved. A draft an agent session still has open is light blue: it is the agent's, so it is read-only here, Save leaves it a draft, and cleaning and playing don't record into it. Right-click > **Take over** asks the session to let go (it closes the session at its next call, keeping the draft), and the draft becomes yours to edit and save.
Segment files changed by other tools (an agent saving a draft, `dj clean`, an editor) are
picked up within a second; one
with unsaved edits here keeps them, with a warning, since saving would overwrite the file. A blue **db** after a segment's length
means its current content is in the local database, and "+N" is how many instructions
only that segment runs.

With MCP > Enable MCP on, drafts written by an AI agent ([mcp.md](mcp.md#drafts)) appear
as unsaved segments, marked `agent` (and the session that has them open). Like any unsaved
segment they don't count towards coverage until saved, though the Coverage tab shows what
they add. File > Save moves them into `segments/` with any edits you made, except drafts
still open in an agent session; Delete removes one (its file goes when you save). Right-click
a draft that is open in a session to follow it.

- Click a segment to go to it; double-click to play it. Right-click for the [Segment menu](editing.md#segment-menu) and **Clean this and below**.
- The filter box at the top matches titles, id prefixes and tags, and shows each match with the path down to it (the path is dimmed).
- Indentation only steps in where a segment has more than one child. An only child stays in its parent's column, joined to it by a short line, so a long chain stays one column however deep it goes. The third icon (a stepped tree) switches to indenting every level, which reads better for small trees. The two icons before it collapse or expand every segment; Collapse all keeps the focused segment open.
- **Sticky segments**: the tree remembers the deepest segment you've been in. Going back up its chain highlights the chain above the selection, and the segments below it that lead back to the deepest one are highlighted faintly with faded titles. Next and the end button follow those even where a segment has several children. Selecting a segment off that chain starts a new one.
- Right-click > **Focus here** shows only that segment and what is below it. A breadcrumb bar above the filter shows the path to it (long paths fold the middle into a "… N" menu); click any crumb to focus there, or **All** for the whole tree. The filter searches inside the focused part only.

## Markers tab

Every event in the project alphabetically, then RAM writes, resets, device changes and RNG
lines, each with its frame and segment (and the parent when another segment has the same
title). The filter matches marker text and segment titles. Clicking a row goes to that
segment and frame, so a marker can do the job of a tiny segment that only exists to name a
moment. Checkpoints and screenshots aren't listed, since they have no names.

## Transport

Below the picture, left to right:

- **⏮** (start) goes to frame 0; pressed again at frame 0, it goes back to the last frame of the parent segment (off at the start of a root segment). **◀●** goes to the previous marker in this segment.
- Step back, play/pause (Space), step forward. The left and right arrow keys step too; Home and End go to the first and last frame. Stepping back from frame 0 goes to the parent's last frame, and stepping forward from the last frame into the segment the piano roll shows next ([neighbours](editing.md#piano-roll)).
- **●▶** goes to the next marker in this segment. Both marker buttons are off when there is no marker that way.
- **⏭** (end) goes to the last frame; pressed again at the end, it goes on into a child segment, carrying straight on from the current state: the sticky segment, or the only child, or, with several, it opens the list. The ▾ beside it lists every child at any time (faint when there is only one). At the end of a segment with no children it is off.
- The **end-of-segment dropdown** picks what playback does at the end of a segment. **Stop** stops. **Loop** plays the segment again from frame 0. **Next** (the default) carries on into the sticky segment or the only child, saved or not, unless an agent session has it open. **List** carries on down the segment list in tree order (whether folded or filtered out) to the end of the list, or to the end of the focused branch, so playing a root segment in List mode plays the whole tree. Each option has a tooltip.
- Speed: a dropdown of 0.25×, 0.5×, 1×, 2×, 4×, 8× and MAX, with − and + beside it.
- Reaching a segment's last frame [records its hashes](playback.md#recording-hashes) as an unsaved change. A pass that runs from frame 0 to the end without a jump or an edit also fills the local database, as cleaning does: snapshots, previews, coverage and first hits. A pass broken by a jump or an edit keeps the snapshots it took. The label beside the transport says the segment's status.

## Menus

| Menu | Holds |
| --- | --- |
| File | Open project…, Save (Ctrl/Cmd+S): writes every unsaved segment and moves drafts into `segments/`; when that would leave segments dirty it offers **Clean, then save** or **Save anyway**. Clean, then save saves at once only if the clean changed no ending; otherwise it stops with the changed endings marked **!** and offers **Save**, **Save anyway** or **Review first**, so nothing unreviewed is saved without asking. Reload from disk (discards unsaved edits), Quit |
| Edit | Undo, Redo, Cut, Copy, Paste at play cursor ([editing.md](editing.md#undo-cut-copy-and-paste)) |
| MCP | Enable MCP (off by default), Show agent drafts, follow an agent session, Stop following, Share my view with agents ([mcp.md](mcp.md#the-gui)) |
| Emulation | Play/pause, step, go to start or end, Use the local database, Snapshot memory, Set ROM…, Set core… ([database.md](database.md#in-the-gui)) |
| Segment | Tree edits and Add event at playhead (E) ([editing.md](editing.md#segment-menu)); at the end, Clean all segments and Replay everything ([database.md](database.md#in-the-gui)) |
| Input | Keyboard mapping (coming later) |
| View | Zoom in, Zoom out, Follow playhead, Return to the edit ([editing.md](editing.md#return-to-the-edit)), Show every joypad button, Second core side by side (coming later) |

## Limits

- Only button inputs get piano roll rows (no analog, lightgun or pointer rows yet).
- You can't play the game from the keyboard yet.
- Only one core runs at a time. Two cores side by side will need a second process, because a libretro core has global state.
- There is no warning before unsaved edits are lost on Open, Reload or Quit.
- MAX speed runs on the UI thread.
- Tested on Linux and macOS; Windows has not been tried.
