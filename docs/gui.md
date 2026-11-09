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
| Left sidebar | Tabs for **Segments**, **Markers** and **Functions**, with the coverage map below them and its own tabs: **Combined**, **Coverage**, **Parity** and **Hidden**. On the map, blue is unreachable code and data from [annotations.toml](config.md#annotationstoml); hovering a square shows each entry's `why`. The colour key sits under the map. With no symbol map, the Functions tab says how to add one and has **Add symbol map…** ([coverage.md](coverage.md#in-the-gui)). | Below, and [coverage.md](coverage.md#in-the-gui) |
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
segment they don't count towards coverage until saved, though the Functions tab shows what
they add. File > Save moves them into `segments/` with any edits you made, except drafts
still open in an agent session; Delete removes one (its file goes when you save). Right-click
a draft that is open in a session to follow it.

- Click a segment to go to it; double-click to play it. Right-click for the [segment editing menu](editing.md#segment-editing) and **Clean this and below**.
- The filter box at the top matches titles, id prefixes and tags, and shows each match with the path down to it (the path is dimmed). `#tag` matches only segments with exactly that tag. The **#** button beside it lists every tag with its count (click one to filter by it) and has **Show what's below the matches**, which also shows every segment below each match. Right-click a tagged segment for **Filter by tag**. Tags are set in a segment's Properties (space-separated, any segment, roots included).
- Indentation only steps in where a segment has more than one child. An only child stays in its parent's column, joined to it by a short line, so a long chain stays one column however deep it goes. The third icon (a stepped tree) switches to indenting every level, which reads better for small trees. The two icons before it collapse or expand every segment; Collapse all keeps the focused segment open.
- **Sticky segments**: the tree remembers the deepest segment you've been in. Going back up its chain highlights the chain above the selection, and the segments below it that lead back to the deepest one are highlighted faintly with faded titles. Next and the end button follow those even where a segment has several children. Selecting a segment off that chain starts a new one.
- **+ New root segment** below the tree, or right-click on its empty space, adds a segment with no parent ([editing.md](editing.md#segment-editing)).
- Right-click > **Focus here** shows only that segment and what is below it. A breadcrumb bar above the filter shows the path to it (long paths fold the middle into a "… N" menu); click any crumb to focus there, or **All** for the whole tree. The filter searches inside the focused part only.

## Markers tab

Every event in the project alphabetically, then RAM writes, resets, device changes and RNG
lines, each with its frame and segment (and the parent when another segment has the same
title). The filter matches marker text and segment titles. Clicking a row goes to that
segment and frame, so a marker can do the job of a tiny segment that only exists to name a
moment. Checkpoints and screenshots aren't listed, since they have no names. While the tree
is focused (**Focus here**), the tab lists only the markers in the focused segment and
below it, and says so above the list.

## Transport

Below the picture, left to right:

- **⏮** (start) goes to frame 0; pressed again at frame 0, it goes back to the last frame of the parent segment (off at the start of a root segment). **◀●** goes to the previous marker in this segment.
- Step back, play/pause (Space), step forward. The left and right arrow keys step too; Home and End go to the first and last frame. Stepping back from frame 0 goes to the parent's last frame, and stepping forward from the last frame into the segment the piano roll shows next ([neighbours](editing.md#piano-roll)).
- **●▶** goes to the next marker in this segment. Both marker buttons are off when there is no marker that way.
- **⏭** (end) goes to the last frame; pressed again at the end, it goes on into a child segment, carrying straight on from the current state: the sticky segment, or the only child, or, with several, it opens the list. The ▾ beside it lists every child at any time (faint when there is only one). At the end of a segment with no children it is off.
- The **end-of-segment dropdown** picks what playback does at the end of a segment. **Stop** stops. **Loop** plays the segment again from frame 0. **Next** (the default) carries on into the sticky segment or the only child, saved or not, unless an agent session has it open. **List** carries on down the segment list in tree order (whether folded or filtered out) to the end of the list, or to the end of the focused branch, so playing a root segment in List mode plays the whole tree. Each option has a tooltip. Below them, **Stop at a changed ending** (off by default) pauses at the end of a segment whose end hash just came out different from its saved file (**!**), before the segments below replay, so the edit can be undone; play again to go on. It stops only at the first one in a run, not at every segment below it.
- The **record button** (a red circle) turns [record mode](#recording) on and off; the ▾ beside it holds the recording options.
- Speed: a dropdown of 0.1×, 0.25×, 0.5×, 1×, 2×, 4×, 8× and MAX, with − and + beside it.
- Reaching a segment's last frame [records its hashes](playback.md#recording-hashes) as an unsaved change. A pass that runs from frame 0 to the end without a jump or an edit also fills the local database, as cleaning does: snapshots, previews, coverage and first hits. A pass broken by a jump or an edit keeps the snapshots it took. The label beside the transport says the segment's status.

## Recording

Record mode writes what you hold on the keyboard into the piano roll. It is a mode you switch on and off with the record button (or Escape to stop), separate from playing or paused: switching it on doesn't start playback. While it is on, the button is red with a white circle, the transport and the roll's ruler are tinted red and the picture has a red frame. It is off on an agent's read-only draft.

**Keys.** File > Keyboard mapping… (also in the ▾ menu) sets a key for each button of the controller being recorded, and the two recording keys, Commit (frame by frame only) and Rewind. Click a key, then press the new one. The defaults are WASD for the d-pad, J = B, K = A, U = Select, I = Start, Space = Commit and Backspace = Rewind; X, Y, L and R have no key until you give them one. The keys are saved per machine in `[keyboard]` in `.local/local.toml`. While recording, these keys go to the game: the GUI's own single-key shortcuts (arrows, Home, End, E, Enter, the roll's Delete) are off, and only Escape, Space as play/pause in real time, and Ctrl/Cmd shortcuts such as undo and save stay live.

**Options** (the ▾ beside the record button, saved in `[record]` in `.local/local.toml`):

| Option | Default | What it does |
| --- | --- | --- |
| Frame by frame | off | Off (real time): Space plays and pauses, and every frame played records what is held, at the selected speed. On: Space is Commit, below |
| Rewind clears inputs | off | Rewind also clears the input on each frame it leaves. Shift+Rewind does the opposite of this setting |
| Overwrite all inputs | on | Each recorded frame clears every controller's input on that frame, so only what you hold is left. Off: held buttons are only added and nothing is released, to record player 2 over player 1, say |
| Extend segment | off | Recording past the last frame makes the segment longer. Off: playback pauses at the end and record mode stays on |
| Controller | player 1 | The controller the keys drive |

**Frame by frame.** Commit writes the buttons held to the next frame and steps to it; held, it keeps committing at the playback speed after a short delay (0.1× is there for this). Rewind steps back one frame without changing anything, unless it clears (above); held, it keeps going back.

**Rewind in real time.** Rewind works the same way while recording in real time: it pauses, steps back one frame (or keeps going back at the playback speed while held), clearing as it goes if set, and when it is let go recording carries on forward from there if it was playing.

**Undo and hashes.** One recording pass is one undo step: from Record on to off in real time (a rewind starts a new step), or one press or hold of Commit or Rewind frame by frame. Recording is an edit like any other: the segment loses its end hash and later checkpoints, and turns dirty until it is played to its end. Playback isn't paused for it ([return to the edit](editing.md#return-to-the-edit) doesn't apply while recording).

## Comparison cores

A comparison core is another libretro core (a decomp built as a core, or the same emulator
in another build or with other options) that runs beside the baseline on the same input
lines. The project lists them as `[[compare]]` entries ([config.md](config.md#compare-optional-repeatable)).

The **Compare** menu holds these controls: Comparison core, Run, Sync now, Check parity, and
Re-run old failures (Stop re-running while it runs). They're only in the menu, not in the
transport.

- **Comparison core** (greyed out when the project has no entries) picks one, or None. A core
  loads when you pick it and unloads when you pick another or None; only one is loaded at a
  time, and none is when the app starts. If it fails to load, a dialog says why and only the
  baseline shows.
- **Run** pauses a loaded comparison core without unloading it. Turned back on, it syncs.
- The pictures sit side by side under their names, with the comparison core's state beside
  its name: **in step**, **paused**, **waiting to sync** or **out of sync** (with the reason
  over its picture). Drag the divider between them to resize; double-click it for an even
  split.
- Each frame the baseline runs, the comparison core runs the same frame with the same input
  lines, on saved, unsaved and dirty segments alike. After a jump the comparison core is
  synced to the baseline before the next frame shown ([connectors.md](connectors.md#comparison-cores)):
  when you're paused, the frame shown is re-run so both pictures show it. **Sync now** tries
  again after a failed sync (in the menu, also after a stop at a failure).
- **Check parity** (off when the app starts) checks after every frame both cores
  run that they agree: with the core's `dj_check_parity`, or by comparing system RAM byte
  for byte when it has none ([connectors.md](connectors.md#parity)). While it's on, every
  frame is drawn, so a failure shows its own frame.
- **Any failure stops playback**: a parity failure, a failed tick (`dj_run`), a failed or
  declined sync. Both cores stop on that frame, and the comparison picture stays faded with
  the reason over it, the state **stopped** and the frame the comparison core was last synced
  at (where its run started). Playing or stepping on syncs it again first, as after a jump;
  after a failed sync, playback carries on without the comparison until the next jump or
  **Sync now**.

### Parity

Each frame checked with **Check parity** is a test. While it's on, the coverage connector's
flags are read after every frame, so each frame's code is known on its own. The results are
kept in the local database, separately for each `[[compare]]` entry
([database.md](database.md#what-it-holds)):

- A frame that **passed** adds the code it ran to the passes of the comparison core's
  **build**: the sha256 of its library when it was loaded. A rebuilt library is a new
  build and starts with no passes.
- A frame that **failed** (a parity failure or a failed tick) is kept with how to reproduce
  it: the build, the segment and frame the comparison core was synced at, the failing
  segment and frame, the reason, and the code that frame ran. Failed syncs aren't kept.

The Functions tab's **Parity with "core"** section counts the instructions with parity,
failed and not checked, and lists the failures, newest first. Click one to show its frame.
**Re-run** syncs where it was synced and plays to its frame with Check parity on (through
the segments in between, if it was synced in an ancestor); **Clear** forgets it. **Re-run old
failures** re-runs every failure from an older build, one after another (**Stop** ends it),
and the status bar says how they went. The map's **Parity** tab colours the ROM map and the
function list by parity alone; **Combined** (the default) lays purple, orange and red over
the coverage colours:

| Colour | Meaning |
| --- | --- |
| Purple | Parity: frames that passed with this build ran it, and no failing frame did. Brighter as more of the code that ran has parity. |
| Red | A frame that failed with this build ran it. A later pass doesn't clear it: clear the failure once it's fixed, or Re-run it. |
| Orange | A frame that failed with an older build ran it. A Re-run that passes now clears it; one that fails turns it red. |
| Grey-green | Ran, but in no frame checked against this build. |
| Dark grey | Never run. |

Any frame that fails marks everything it ran, so code every frame runs (the NMI handler,
the main loop) turns red with any failure. The selected function shows its parity counts
and links to the first failing frame that ran it.

## Menus

| Menu | Holds |
| --- | --- |
| File | Open project…, Save (Ctrl/Cmd+S): writes every unsaved segment and moves drafts into `segments/`; when that would leave segments dirty it offers **Clean, then save** or **Save anyway**. Clean, then save saves at once only if the clean changed no ending; otherwise it stops with the changed endings marked **!** and offers **Save**, **Save anyway** or **Review first**, so nothing unreviewed is saved without asking. Save selected segment (writes only that segment's file), Revert selected segment (drops only its unsaved changes and puts back its file; undo brings them back), Reload all from disk (every segment, dropping all unsaved edits), Options… (not yet: GUI preferences), Keyboard mapping… ([below](#recording)), Quit. With unsaved edits, Open, Reload all and Quit (also the window's close button) ask first: **Save and …**, **Discard and …** or **Cancel**. Drafts nobody edited here don't count: they stay in `.local/drafts/`. |
| Edit | Undo, Redo, Cut, Copy, Paste at play cursor ([editing.md](editing.md#undo-cut-copy-and-paste)), Select all (Ctrl/Cmd+A: every press in the piano roll), Drag to draw inputs (off to begin with; [editing.md](editing.md#piano-roll)); then the segment edits and Add event at playhead (E) ([editing.md](editing.md#segment-editing)) |
| View | Zoom in, Zoom out, Follow playhead, Return to the edit ([editing.md](editing.md#return-to-the-edit)), Show every joypad button |
| Project | Clean all segments, Replay everything, Use the local database ([database.md](database.md#in-the-gui)), Set ROM…, Set core…, Project settings… (not yet) |
| Emulation | Play/pause, step, go to start or end, Snapshot memory |
| Compare | Comparison core, Run, Sync now, Check parity, Re-run old failures or Stop re-running ([above](#comparison-cores)) |
| MCP | Enable MCP (off by default), Show agent drafts, follow an agent session, Stop following, Share my view with agents ([mcp.md](mcp.md#the-gui)) |

## Limits

- Only button inputs get piano roll rows (no analog, lightgun or pointer rows yet).
- Recording uses the keyboard only (no gamepads yet), for one controller at a time, and the default keys don't cover X, Y, L or R.
- One comparison core at a time, and it has no sound. Same-file copies (a comparison core that is the baseline's own library) are tested on Linux only.
- MAX speed runs on the UI thread.
- Tested on Linux and macOS; Windows has not been tried.
