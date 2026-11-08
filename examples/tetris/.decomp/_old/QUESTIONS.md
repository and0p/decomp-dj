# Questions before work starts

Each has a recommended default, which [PLAN.md](PLAN.md) assumes. "Defaults" is a fine
answer to any of them.

### Q1. Matching or behavioural?

- **Behavioural (default).** C that behaves the same, checked by RAM hashes on every
  segment. Readable, and testable with what decomp-dj already does.
- Matching: C compiled to a byte-identical ROM. Not realistic for a game written in
  assembly; the disassembly already covers byte-for-byte rebuilding.

### Q2. Hybrid or all at once?

- **Hybrid (default).** A core with a 6502 interpreter that runs the ROM's code for every
  routine not yet in C. The game runs from day one, and each routine is checked alone.
  Costs an interpreter and the asm/C handover (PLAN 4.4).
- All at once: translate all 245 routines, then debug parity with the RAM diff. Less
  machinery, but nothing can be checked until nearly everything is written, and the first
  divergence can be anywhere.
- Inside Mesen instead: a connector hook that calls C routines from Mesen's own CPU. Exact
  hardware, but waiting for vblank inside a C routine would mean re-entering Mesen's frame
  loop, and the end state still needs its own platform layer.

### Q3. Where does the C run?

- **On the host, as a libretro core (default)**, so `dj` and `dj-gui` play it like Mesen.
  An SDL frontend can come later.
- Also, later and optionally: the same C built for the NES with llvm-mos, to run on Mesen
  or hardware. Interesting, but it changes timing and memory use, so it can't be the
  parity target.

### Q4. Stack page in parity?

- **Keep it (default).** C calls write the return addresses the original `JSR`s would, so
  all 2 KB match and the project's existing hashes are the test.
- Leave `$0100-$01FF` out: simpler C, but the hashes would need a mask or a second set of
  regions, which decomp-dj doesn't have (PLAN 9).

### Q5. Will the decomp be public?

This decides how careful the source must be:

- Either way, no ROM bytes in the repository: data tables, tiles and music come from the
  user's ROM at run time (default).
- The disassembly has no licence file. If the C will be published, should routine and
  variable names follow it (default: yes, names are what make it comparable), and may its
  comments be copied (default: no; write our own)?

### Q6. The symbol map for `examples/tetris`

The `ca65-dbg` reader finds nothing in this `.dbg` (PLAN 3). Options:

- **Generate a `csv` map into `build/ref/` with a script (default)** and point
  `[[symbols]]` at it, as Thwaite points at `build/thwaite.dbg`. Nothing from the
  disassembly is committed.
- Commit the generated `csv` under `examples/tetris/symbols/`, so a fresh clone has names
  without building the reference.
- Teach the `ca65-dbg` reader about this layout (a decomp-dj change).

### Q7. Where does the source live?

- **In `examples/tetris/.decomp/` (default)**, beside this plan, building into
  `examples/tetris/build/decomp/`.
- In its own repository, with `examples/tetris` as its test project.

### Q8. Parity tooling: in `.decomp/` or in decomp-dj?

- **Small tools in `.decomp/tools/` first (default)**: the two-core RAM diff and fixture
  capture. Move into decomp-dj (comparison cores, `dj parity`) once their shape has been
  proven on this game.
- Build the roadmap's comparison-core support now, with Tetris as its first user.

### Q9. Scope

- USA only (default). PAL and NWC later, if at all; the disassembly's `PAL` and `NWC`
  switches show where they differ.
- Two-player code: translate it for completeness but mark it untested, since it can't run
  (default), or leave it out.

### Q10. Style

Default: C99, one file per area, routine names from the disassembly in camelCase as they
are, `ram.h` names for every variable, a `BLOCK(addr)` at the top of each original basic
block and `CALL(site, fn)` for calls. Anything you want different (snake_case, C11,
clang-format settings)?

### Q11. Sound

Default: capture APU writes from the start, synthesize audio in Phase 8. Earlier if you
want to hear it while testing.

### Q12. Who does the work, and how do you review it?

Default: an agent does each phase, driving `dj` and `dj mcp`, and stops for your review
at the end of each phase, with parity results and the coverage map attached. Smaller steps
(per area, or per routine for the big ones) if you want to read along more closely.
