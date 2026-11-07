# Concepts

decomp-dj keeps a **project**: a tree of plain-text input **segments** that drive a
libretro core from power-on (or from a saved starting point) into every situation a game
can reach. A project is what people share. Everything else (snapshots, coverage,
previews) is derived locally by playing it.

## Terms

These are the working terms. The docs and the GUI use them. The code and the CLI's help
still use older words in places ("replay" for segment, "collection" for project, "trunk"
for root segment), and segment files still start with `dj-replay 1`.

| Term | Meaning |
|---|---|
| **Project** | One game's folder: `config.toml`, the segments in `segments/`, blobs and `unreachable.txt`. It is what people share and keep in version control. Local data lives in its `.local/` folder. (Formerly "collection".) See [segments.md](segments.md#project-layout). |
| **Segment** | One `.segment` file: inputs by frame, metadata, hashes and a parent link. It is one entry in the GUI's sidebar tree. (Formerly "replay", and "node" in the GUI.) |
| **Root segment** | A segment with no parent. It starts from power-on, or from an SRAM or snapshot blob. (Formerly "trunk".) |
| **Child segment / children** | Segments that start where their parent ends. A segment with no children is a leaf. |
| **Snapshot** | A serialized emulator state, more or less a "save state". It may leave out memory that doesn't help the tools. |
| **Diff** | A patch from one snapshot to a later one, storing only what changed. |
| **Save** | Only the game's own saves: SRAM, memory cards and cartridge save chips. Never an emulator save state. |
| **Marker** | A named point in a segment: an `[EVENT]`, or another line the GUI shows in the marker lane (writes, resets, device changes). |
| **Macro** | Future: anything an author can script, for example RNG manipulation or pokes, kept locally and referred to by name. |
| **State hash** | A hash of the game's memory that says "where the game is" without shipping memory. See [playback.md](playback.md#state-hashes). |
| **Clean / dirty** | Whether a segment's recorded hashes still hold: clean when it has an end hash and starts where its clean parent ends, dirty otherwise. Dirty comes in two kinds: **unrecorded** (no end hash yet, as after an edit) and **stale** (hashes recorded, but it starts where its parent no longer ends). Worked out from the hashes, never stored. Playing a segment to its end records its hashes, so dirty mostly clears itself; **Clean all segments** (or `dj clean`) does it for the whole tree. See [playback.md](playback.md#clean-and-dirty). |
| **Changed ending (!)** | A segment whose recorded end hash, not saved yet, differs from the one in its saved file. Not a status: a flag for the author, who decides whether the change was intended. Saving clears it. |
| **Unsaved** | Anything in the editor not written to `segments/` yet: your edits, hashes recorded by playing, and agent drafts. Only saved segments count towards coverage. File > Save is the one way something becomes part of the project. |
| **Draft** | An unsaved segment an agent wrote, in `.local/drafts/`. The GUI shows it like any unsaved segment, marked "agent"; Save moves it into `segments/`. See [mcp.md](mcp.md#drafts). |
| **Local database** | A cache in `.local/cache.db` of facts worked out by playing: snapshots, previews, coverage and first hits, filed by content key. Any full pass fills it, whether watched in the GUI, cleaning or an agent's. It can always be deleted and rebuilt. See [database.md](database.md). |
| **Clean (the action)** | Play the segments that aren't clean or aren't in the local database yet, recording the hashes they reach and filling the database. See [database.md](database.md#in-the-gui). |
| **Content key** | A hash of a segment's setup, start state and inputs. Derived data in the local database is filed under it. |
| **Baseline** | The project's reference core (the original emulator) and the results it produces from the inputs. Also called the oracle. There is one per project, and it rarely changes. |
| **Comparison core** | Any other core played against the baseline with the same inputs, usually a decomp built as a libretro core, but possibly another emulator. Working name; "candidate" is another option. A project can list more than one. |
| **Coverage connector** | A small patch to a core that reports which ROM bytes ran. See [connectors.md](connectors.md). |
| **Coverage** | An instruction or block counts as covered when at least one clean, saved segment executed it. Dirty and unsaved segments don't count, though the GUI shows what they would add. |
| **Uncovered** | In the ROM, not executed by any clean, saved segment, and not marked unreachable. This is where the next segments should go. |
| **Parity** | A comparison core matched the baseline every time the baseline executed an instruction or block. If parity hasn't been checked, a covered block stays covered. |
| **No parity** | A comparison core has been shown to differ from the baseline at an instruction or block. |
| **Unreachable** | An instruction or block that is in the ROM but has been determined to be unreachable. Listed in `unreachable.txt`, in version control. See [coverage.md](coverage.md#unreachable-code). |

## Colors

The GUI, reports and any export use the same colors:

| Color | Meaning |
| --- | --- |
| Green | Covered, and a clean segment (the same green) |
| Yellow-green | New coverage not counted yet: run by an unsaved segment or the pass being played, and by no saved clean segment. It counts once saved |
| Dark, faded green | Run only by dirty segments (the last time each played). It counts once they are clean again |
| Gray | Never run |
| Purple | Parity |
| Red | No parity |
| Blue | Unreachable |
| Default text color | Uncovered |
| Orange | A dirty segment |
| Italic yellow title | An unsaved segment (edited here, or an agent's draft) |
