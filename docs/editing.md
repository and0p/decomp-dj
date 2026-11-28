# Editing

How the tree and a segment's inputs change. The tree edits are normative: any tool that
makes the same edit must write the same files. The GUI is the only tool that makes them
today. `dj new [--parent ID] [--title TEXT] [--frames N]` adds a segment from the
command line and prints its path.

### A root from a snapshot

A root segment starts from power-on unless its header names a save state (`snapshot`) or
a save file (`sram`), both blobs ([segments.md](segments.md#blobs)). Two forms of `dj new`
make one, and both take `--title` and `--frames` as usual:

```
dj new --snapshot FILE [--sram FILE] [--start-frame N] [--blob-name NAME]
dj new --from SEGMENT --at FRAME [--blob-name NAME]
```

- **`--snapshot FILE`** starts from a save state of the project's core, such as
  `dj snapshot` writes. `--start-frame` says how many frames after power-on it was taken
  (0 if left out). It only numbers the frames; playback doesn't depend on it.
- **`--sram FILE`** starts with that save file, alone (a power-on with a save) or with
  `--snapshot`.
- **`--from SEGMENT --at FRAME`** plays SEGMENT to the start of that frame and starts the
  new root there. A save state holds the console's memory, not what was being done to it,
  so the new root's frame 0 gets a line for each button held and each stick off centre on
  that frame, and a `[DEVICE]` line for each port whose device isn't the one `config.toml`
  plugs in. The lines of SEGMENT from FRAME on, moved back by FRAME, then play to the same
  ending. The start frame is worked out, and without `--title` the root is named after
  SEGMENT and the frame.

In the GUI, Edit > **New root segment from here** does what `--from` does at the playhead
([segment editing](#segment-editing)).

Each copies the files into `blobs/`, loads them into the core,
and records the state hash the core is then in as the root's `start-hash`. A file the core
won't load is refused with the core's build named, and nothing is written: no segment, and
no blob that wasn't there already. All three need the core and the ROM (`--core`, `--rom`).
The copy is named after the file, or with `--from` after the segment and the frame;
`--blob-name` chooses another name, and it can be renamed later without touching a segment
([segments.md](segments.md#blobs)).

### A root from a save file

A game with a battery save can start a root from the save alone: the console powers on
with that save in the cartridge, and nothing of the game's own memory is copied. dj can
take the save out of a segment at any frame:

```
dj sram SEGMENT FRAME --out FILE        # the save RAM at the start of that frame
dj new --sram FILE --title "World 3"    # a root that powers on with it
```

In the GUI, Emulation > **Export save RAM…** writes the save as it is after the frame
shown, and Emulation > **New root segment from this save** makes the root in one step.

- **Wait for the game to finish saving.** A game that writes its save over several frames
  gives half a save if it is exported in the middle. Step a few frames past the end of
  the save before exporting.
- **A game with no battery save has no save RAM**, and both say so instead of writing an
  empty file.
- **The file is what the core calls its save RAM**: for most consoles the same `.srm`
  other emulators read. Mupen64Plus-Next keeps the EEPROM, SRAM, FlashRAM and controller
  paks in one file.

**Before a snapshot is made, dj says what one holds.** A snapshot is a copy of the
console's memory, which can include the game's code and data, so it shouldn't be published
unless you have the right to share the game. A save file, memory writes (`[WRITE]` lines)
or an in-game password get to most places without one. The warning is advice: dj never
refuses to make a snapshot, and what a project publishes is its author's call
([segments.md](segments.md#project-layout)). For a game that may be shared (homebrew, open
source, your own), `shareable = true` under `[rom]` in `config.toml` turns the warning off
for everyone who uses the project ([config.md](config.md#rom)); `dj init` doesn't ask.

- `dj new --snapshot` and `--from` print it as a note.
- The GUI asks first, with Create and Cancel, and **Don't show this again** keeps your
  answer in this project's `.local/local.toml` (`[gui] hide_snapshot_warning`).
- A project whose game may be shared turns it off for everyone with `shareable = true`
  under `[rom]` in `config.toml` ([config.md](config.md#rom)).

## Tree edits
**New child.** A new segment with `parent` set to the segment, a fresh id, no
hashes, and a length chosen by the author. It starts where the segment ends.

**Split at frame `f`** (`0 < f < frames`). One segment becomes two:

- The **second part** keeps the segment's id, title, tags, children and
  `end-hash`. Its body is the segment's lines from frame `f` on, renumbered from 0,
  and its `frames` is the old length minus `f`. Its `parent` becomes the first
  part. Checkpoints after `f` move to it, renumbered. It loses `snapshot` and
  `sram`, because only a root segment may name blobs.
- The **first part** gets a new id and takes the segment's old place under its
  parent. It keeps frames `0..f`, the earlier checkpoints, and any `snapshot` or
  `sram`. By default its title is the old title plus "(first f frames)".
- The state hash at the cut (at the start of frame `f`, after the first part
  has run) becomes the first part's `end-hash` and the second part's
  `start-hash`. Because the same frames still run with the same inputs, the
  second part and everything below it stay clean.
- Every segment starts with all inputs released ([frames](segments.md#frames)). So for each
  input held across the cut, the second part gets a line at frame 0 that sets
  it again.

Keeping the id on the second part means children and outside references to the
segment stay valid. Coverage is stored by content key ([database.md](database.md)), so both parts
get new keys and their coverage is recorded again the next time they are played.

**Branch at frame `f`.** A split at `f`, plus a new empty child of the first
part, so that a second path starts at the cut.

**Join with parent** (only for a segment that is its parent's only child), the
inverse of a split. The parent keeps its id, title and place, and grows by the
child's `frames`. The child's lines follow the parent's, shifted by the parent's old
length, with a release at the join for any input the parent still held (as a segment
start would do). The child's children become the parent's, the parent takes the
child's `end-hash` and later checkpoints, and the child's `start-hash` becomes a
checkpoint at the join frame. The same frames run with the same inputs, so every hash
below stays valid. Tools may add an `[EVENT]` at the join frame, by default named
after the child, to keep the moment the split marked.

**Delete.** Removes a segment and everything below it. **Delete, keep children** removes
only the segment: its children move up to its parent (or become root segments, starting
from power-on), and since they now start from another state they lose their hashes and
turn dirty, with everything below them.

**Move.** Gives a segment another parent, or makes it a root segment. It now starts where
the new parent ends, so its `start-hash`, checkpoints and `end-hash` go and it and
everything below turn dirty; its inputs stay as they are. A segment that gets a parent
drops its own save file or snapshot, which only a root segment has. A segment can't move
below itself.

**Properties.** Changing `title` changes no state. Changing `frames` drops the
`end-hash` and any checkpoints past the new end. A segment cannot be shortened
past its last body line. On a root segment, choosing or clearing its save (a snapshot, `snapshot`, or
a save file, `sram`) changes how it starts: the file is copied into `blobs/` under the name shown below it, and the segment drops its
`start-hash`, checkpoints and `end-hash`, so it and everything below turn dirty.

**Duplicate.** A copy of the segment beside it: same parent, same lines and hashes, a new
id and "(copy)" after the title. For a segment with children it asks "Duplicate N children?
(F frames)", counting everything below it, and offers **With children** (the whole branch
is copied under the copy, each with a new id) or **Just this segment**.

## What an edit does to hashes

Changing an input or a `[WRITE]` drops the segment's `end-hash` and every checkpoint after
the first edited frame, because those states are no longer known. The segment and
everything below it turn dirty, until it is played to its end and the new hashes are
recorded ([playback.md](playback.md#recording-hashes)). Editing only `[EVENT]`
lines changes no state, so it keeps every hash.

## In the GUI

Edits stay in memory until File > Save (Ctrl/Cmd+S). Edited segments show in yellow
italics in the tree until then. Open, Reload and Quit ask before they drop unsaved edits,
offering to save them first.

### Segment editing

The Edit menu (below Undo, Cut, Copy, Paste and Select all), and right-click in the tree, offer:

- **Play to here** (right-click only): plays through to the end of that segment, on from the playhead if the current segment is on the way, otherwise from the first segment below where the two paths meet.
- **New child at end**.
- **New root segment**: an empty 600-frame segment with no parent, starting from power-on; its Properties open so it can be named, sized and given a save file or snapshot, from the project's `blobs/` or from disk. Also on right-click in the empty space of the tree, and the **+ New root segment** button below it.
- **New root segment from here** (Edit menu): a segment with no parent that starts where the playhead is, in the state the frame shown left. It writes that state into `blobs/` as a snapshot and puts on the new root's frame 0 the buttons and sticks held there and any device plugged in since `config.toml` ([a root from a snapshot](#a-root-from-a-snapshot)). The new root is 600 frames long and named after the segment and the frame. It asks first, saying what a snapshot holds ([a root from a snapshot](#a-root-from-a-snapshot)).
- **Duplicate**: a copy beside the segment, asking first whether to copy what is below it too.
- **Branch at playhead**: splits the segment right after the playhead and starts an empty second path beside the rest.
- **Split at playhead**: just the split. The first part becomes "(first N frames)" in the old place.
- **Properties**: title and length, and for a root segment its **Save**: what it starts from. The list has power-on at the top, then the snapshots and the save files already in the project's `blobs/` under their own headings (name and size; hover for the segments that start from each), then **A snapshot from disk…** and **A save file from disk…**. With more than five files, typing narrows the list. Choosing one replaces the other kind: a snapshot already carries the game's save. (`dj new --snapshot FILE --sram FILE` still makes a root with both, and Properties shows both until one is chosen.) **Clear** goes back to power-on. **Name in blobs/** under it is the file's name: set it for a file from disk, or change it to rename the file that is there ([segments.md](segments.md#blobs)).
- **Save** and **Revert** (right-click only; also File > Save selected segment and Revert selected segment): write just this segment's file, or drop just its unsaved changes and put back its saved file, leaving other edits alone. Revert is one undo step. Save is off while the segment's parent has never been saved, and Revert for a segment that has never been saved (Delete removes it).
- **Join with parent** (only for a segment with no siblings): the confirmation has "Leave a marker", on by default, with an editable text that starts as the child's title. An event at the join frame keeps whatever moment made the author split there ("menu fully open", "victory pose finished").
- **Join with only child** (only for a segment with exactly one child): the same join, offered on the parent. It opens the same confirmation.
- **Join chain…** (when the only child has an only child of its own): joins every only child below the segment into it, down to the first segment with no children or several. The confirmation lists the segments and total frames, and "Leave markers" puts an event with each joined segment's title at its join frame. One undo takes it all back.
- **Move to…**: a list of every segment the selected one can go under, as paths (Root › A › B) with a filter, plus "No parent" to make it a root segment. See **Move** above.
- **Delete…**: removes the segment and everything below it, after a confirmation that names the segment, says how many segments go with it and gives the total frames. When it has children, **Delete, keep children** removes only the segment and moves its children up to its parent. The files are removed on Save. If the segment being shown was deleted, its parent is shown.

In the tree, Enter (or F2) renames the selected segment in place; Enter or clicking away
keeps the new name, Escape cancels. Segments can't be moved to another parent from the GUI
yet.

### Piano roll

The bottom of the window. Time runs left to right, with one row per button, grouped by
controller. Groups the segment uses are tinted and open, unused ones dimmed and closed;
click a group name to open or close it. A closed group still shows when any of its buttons
are held. Buttons and stick axes get rows; lightgun and pointer inputs show as ticks on
their controller's header. Rows are named as the core names its inputs, or as the project
does ([config.md](config.md#naming-the-controls)); hover a row for libretro's name.

- Wheel or trackpad over the timeline scrolls time; Ctrl/Cmd+wheel zooms at the pointer; wheel over the names scrolls rows; "fit" shows the whole segment.
- Click or drag on the ruler to seek.
- **Neighbours.** Left of frame 0 the roll shows the end of the parent segment, and past the last frame the start of the next one (the sticky segment, or the only child), faded and read-only, with their titles in the marker lane and their own frame numbers on the ruler. Clicking or dragging there does nothing. Only stepping (the arrow keys or the step buttons), playing on, or the start and end buttons cross the boundary, and that switches over fully: the other segment becomes the selected one, so it can be edited. The view stays put as you cross. No edit ever spans two segments.
- There is one editing mode (hover the ⓘ in the roll's toolbar for a summary):
    - Click an empty cell to add a press as long as the last press made or resized (4 frames to begin with). A drag from an empty cell selects an area; with **Edit > Drag to draw inputs** turned on (it starts off) it draws a press as long as the drag instead. Holding Ctrl/Cmd does whichever of the two the setting doesn't.
    - Drag a press to move it in time, and drag either edge to lengthen or shorten it (only while at most one press is selected; the grab zone is a thin strip on the edge). Plain drags never change rows.
    - Transpose (move presses to another button row of the same controller) only on purpose: hold Alt (Option on a Mac) while dragging, or press Alt+Up / Alt+Down with presses selected.
    - Click selects a press; Shift+click adds or removes it. To select an area, drag from empty space (Ctrl/Cmd+drag while drawing is on, or from a press), or Shift+drag to add to the selection (Shift+Alt+drag removes); a drag on a controller's header row or past the last frame selects too.
    - Right-click a press, or press Delete, to remove it. Escape clears the selection, and Cmd+A (Ctrl+A elsewhere, also Edit > Select all) selects every press of the segment, unless a text field has focus. Presses that end up overlapping merge into one. Holding a drag near either end of the roll scrolls it.

### Stick axes

A controller's sticks are rows below its buttons, one per axis (left X and Y, right X and
Y), shown when the core or the project names them or the segment uses them. An axis has a
value on every frame. 0 is the middle of the row, and a bar rises when the stick is pushed
right (X) or up (Y), with the number written on a run of frames wide enough to hold it.
The name column shows the value on the frame shown.

- **Units.** Where dj knows how the core turns the stick into what the game reads, the row
  is in the console's units: the N64's control stick on Mupen64Plus-Next, -80 to 80 with
  the core's default sensitivity, as a TAS movie writes it. Other sticks show libretro's
  values, -32767 to 32767. Either way up is positive, though libretro counts down as
  positive in the file.
- **Drag up or down** to set a value. On an empty stretch this makes a short input, as
  long as the last press or stick input made (4 frames to begin with), the way a click on
  an empty button cell does. On an input that is already there it changes that input's
  value. Near the middle it snaps to 0, and near the top or bottom to all the way.
- **Drag across** as well to choose the frames yourself: the value goes on the frames
  from where the drag began to where it ends, and what came after stays as it was.
- **Drag a boundary** to move where one value ends and the next begins. Two inputs that
  touch stay touching: one grows as the other shrinks, and nothing else in the row moves.
  The edge of an input next to an empty stretch works the same way, which makes the input
  longer or shorter. Neither input can be dragged away to nothing. Hold Alt to leave the
  frames the boundary passes over at 0, opening a gap.
- **Double-click** to type a value, for the input under the pointer or a new short one
  (Enter keeps it, Escape drops it). The box says which frames it will set.
- **Right-click** an input to put it back to 0.

In the file an axis is a line where its value changes (`120 P1 ANALOG LEFT X -20000`,
[segments.md](segments.md)), always in libretro's values. With console units, setting one
axis rewrites both of the stick's lines on those frames: the core works the game's two
numbers out from the stick's position as a whole (its deadzone is a circle), so keeping
the other number where it was takes a new value for its axis too.

**Stick pad.** View > Stick pad opens a small window with a square: drag the point in it
to set the stick's X and Y together on the frame shown, right and up positive, in the same
units as the rows. It snaps to the middle and to the edges. The X and Y fields set a
number exactly, and Centre lets the stick go on that frame. Pick the controller and the
stick at the top (a stick the project shows as buttons can't be picked). With **Hold while
stepping** on, stepping forward (or recording a frame) puts the stick in the same position
on every frame stepped onto, so a held direction is a matter of setting it once and
stepping, or holding the step key; playing or jumping to a frame doesn't carry it. A drag is one undo step, and so is a whole run of held frames.

While recording in real time (record on, playing, not frame by frame) the pad is a stick
being held: its position goes into each frame as it plays, alongside the keys held, and
play carries on. Without Hold the stick is only written while the point is dragged, and
the frames after you let go keep what they had; with Hold the position stays until you
move it again.

An axis whose directions the project names
([config.md](config.md#naming-the-controls)) is two button rows instead, edited like any
button: the N64's C buttons on Mupen64Plus-Next.

Recording writes a stick all the way in a direction that has a key
([gui.md](gui.md#recording)); part-way values are set here. Values on axis rows can't be
selected, moved or copied.

### Return to the edit

With View > **Return to the edit** on (the default), any edit pauses playback and shows the
earliest frame edited since you last played or moved the playhead, worked out again with
the change. Several edits in a row keep the playhead on the earliest of them; seeking,
stepping or clicking elsewhere first starts a new run, so the next edit returns to itself.
Turned off, an edit redraws the frame you were on instead.

Either way, the ruler shows a band over the frames whose hashes aren't known, in the colour
of the segment's tree dot (orange): from the first edited frame of a segment whose hashes
aren't recorded, over the whole of one that starts somewhere its parent no longer ends. It shrinks as playback runs through frames not played
since the edit, and goes when the segment is clean again, which reaching its end does when
it can ([playback.md](playback.md#recording-hashes)). Hover it for what is missing. While
the picture still shows a frame from before the edit and the new one is being worked out,
it is dimmed.

### Markers

The marker lane, above the button rows, shows events, RAM writes, `[DEVICE]` and
`[RESET]` lines, screenshots and checkpoints. Click "markers" in the label column to open
or close it, like a controller.

- Closed, it is one row: markers on one frame share a box, and markers too close together hide behind the most important one (events first), which shows "+N" for the rest.
- Open, every marker gets its own box with its whole label: events in the top rows, writes and everything else below. Labels run until the next marker in their row.
- Hover for all markers at that spot; click one to edit, delete or add lines on that frame. **E** adds an event at the playhead.

### Undo, cut, copy and paste

Edit > **Undo** / **Redo** (Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z or Ctrl+Y) cover every edit: note
edits, cut and paste, frame lines, renames, segment properties, using a new end hash, new
segments, splits, branches, joins and deletes. Hashes recorded because of an edit (reaching
a segment's end after changing it, say, and the segments below it that the local database
can then clean) join that edit's step, so one undo takes back the change and what it
caused. A clean is a step of its own. A segment that undo or redo brings back to exactly
what its saved file holds counts as saved again. The history holds 200 steps and is cleared
when a project is opened or reloaded. A segment removed by undo has its file deleted on the
next Save.

**Cut**, **Copy** and **Paste** (Ctrl/Cmd+X/C/V) work on the notes selected in the piano
roll. The copy is kept inside the app, and the notes also go on the system clipboard as
input lines with frames relative to the first note. Paste puts the leftmost copied note at
the play cursor and selects what it pasted.
