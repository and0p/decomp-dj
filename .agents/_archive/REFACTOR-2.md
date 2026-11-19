# Refactor, part 2: next steps

[REFACTOR.md](REFACTOR.md) is built. This plan is what comes next, from two plan reviews
(`/mnt/project-files/reviews/refactor-plan-review.md` and `-2.md`) and the decisions Andrew
made after them. It starts with bugs in what was built and ends with the larger
mechanism that makes most dirty states go away.

**Status: carried out** (all four phases), with these choices made on the way:

- Item 12: the GUI has one action, **Clean all segments**, which plays what isn't clean or
  isn't stored, plus **Replay everything** (every segment, stored or not); the right-click
  menu has **Clean this and below**. `dj clean` does the same (`--all` to replay
  everything); `dj hydrate` stays for scripts that only want the database filled. The noun
  is "local database", the verb "clean"; the **db** badge reads "Stored".
- Item 13: hashes recorded by reaching a segment's end, or read from the database, join the
  last undo step. A clean (Clean all segments, or the clean before a save) is an undo step
  of its own.
- Item 14: the two kinds of dirty are **unrecorded** and **stale**.
- Item 16: `cache gc` refuses while a GUI has the project open (a `.local/gui-<pid>.open`
  file the GUI touches every minute; older than 3 minutes counts as gone), unless `--force`.
- The ROADMAP's "MCP notices a save" was dropped, as planned; Agent-owned segments, Atomic
  segment writes and Undo takes back hashes shipped and left it.

## Decisions already made

- **One ! per segment, cleared by Save.** It lists the consequences of an upstream change;
  the cause is the most upstream one, so it needs no mark of its own.
- **Playback runs straight through changed endings.** A child always starts from where its
  parent really ends. An opt-in "Stop at a changed ending" is on the roadmap.
- **The word stays "drafts"**, and so do `.local/drafts/` and the `draft_*` tools. The GUI's
  "agent" tag is what the person needs.
- **Agent-owned segments:** while a session has a draft open it is the agent's: light blue
  in the tree, read-only, **Take over** to make it yours.
- **Saving reviews changed endings first.** Clean, then save never saves changes nobody looked at.
- **Undo takes back the hashes its edit caused.**
- **`--allow save` stays.**

## Phase 1: fix what was built (small, do first)

1. **Next and List carry on into any reachable child**, saved or not. Today `next_node`
   refuses when either side is unsaved, and recording hashes counts as unsaved, so after
   cleaning, playback stops at every boundary. Condition: the child exists and it isn't
   agent-owned.
2. **Unsaved before dirty in the coverage report.** `build_report_with` tests dirty first,
   so a new draft (no end hash, hence dirty) shows as faded "+N if cleaned" instead of
   yellow-green "+N unsaved". Test unsaved first; dirty-only is for saved segments.
3. **Clean, then save stops for review.** After cleaning:
   - nothing changed and nothing failed: save;
   - endings changed: "Cleaned: N endings changed (marked !). Save / Review first";
   - something couldn't be cleaned (a play error, Stop, or agent-owned): "N segments stay
     dirty: Save anyway / Cancel", asked once, not by reopening the first dialog.
4. **Agent-owned drafts aren't counted as unsaved.** Leave them out of the title's `*`,
   Save's count and the status bar, or show them apart ("2 agent").
5. **`dj clean` gates changed endings like the GUI.** It writes recorded and cleaned hashes,
   lists changed endings without writing them and exits non-zero; `--accept` writes them.
   `--accept` then means the same thing on `run`, `play` and `clean`.
6. **Roots whose start changed.** `try_record` no longer compares a root's recorded
   `start-hash` (a new blob, say), and **!** watches only the end hash. Let **!** cover a
   changed start too, or treat a root start mismatch as a changed ending.

## Phase 2: agent ownership

7. **Agent-owned in the GUI.** A draft open in a session: title in light blue (distinct
   from the **db** badge's blue and unreachable blue), no edits, not saved, not cleaned;
   Follow works; **Take over** closes the session with `keep` and makes it an ordinary
   unsaved segment.
8. **Close the save race on the GUI side.** Re-read the session files just before Save
   moves drafts, instead of trusting the once-a-second poll. The server doesn't switch
   ids under an agent (dropped from the roadmap).
9. **Atomic writes** (temporary file, then rename) for draft files in `dj mcp` and for the
   GUI's Save.

## Phase 3: the cache answers (the big one)

10. **The cache supplies hashes.** `contents` has `end_hash` per content key. When a
    segment's key (setup, start state, body) is stored, its hashes come from the row
    without playing. A fresh clone with a filled cache, a saved draft, or a child whose
    parent's new end it already ran from is clean at once, and Clean all only plays what
    the cache can't answer. Includes the "check on open" from REFACTOR.md: a stored pass
    whose end differs from the file flags **!** without playing.
11. **MCP sessions record (rule 2 on the agent side).** A session's straight pass from its
    start records its content (coverage, first hits, end state, snapshots) as GUI passes
    do, and keeps snapshots when broken. Drafts then reach the GUI with coverage known
    (yellow-green at once), and with 10 their save costs nothing.
12. **Merge Hydrate all and Clean all** once 10 is in: one action that fills the cache and
    cleans, with a "replay everything" option. One noun and verb for the cache in the docs
    and UI (local database / cache / hydrate / the db badge).

## Phase 4: undo and small things

13. **Undo takes back caused hashes.** `auto_record` and `end_verify` fold the segments they
    change into the top undo step, so undoing the edit restores the children's hashes
    with it. No new kind of step. Check the case where the top step is an unrelated edit:
    undoing it then drops hashes that a play can record again, which is acceptable.
14. **Name the two kinds of dirty once** in concepts.md (no hashes recorded; starts
    where its parent no longer ends), and use those words for `dj status`'s `?` and `~` and
    the GUI's hover text.
15. **database.md:** a segment cleaned by a broken pass is clean but uncounted until a full
    pass or hydrate; the **db** badge is the hint. (Phase 3 makes this rarer.)
16. **`cache gc` keeps the GUI's work.** It drops snapshots of content no saved segment
    has, including a live edit's. Skip gc while a GUI has the project open, or keep content
    played in the last day.

## Order

Phase 1 in one pass (items 1 to 6 are each small and independent), then 7 to 9, then 10,
11 and 12 in that order, then 13 to 16. Docs change with each item, as before.
