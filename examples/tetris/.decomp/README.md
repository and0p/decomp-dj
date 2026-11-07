# Tetris C decompilation (planning)

A plan for decompiling Tetris (NES, USA) into C, checked against the original with
decomp-dj. Nothing here has been built yet: this folder is for review first.

| File | What it is |
| --- | --- |
| [PLAN.md](PLAN.md) | The plan: goal, reference, architecture, phases, how decomp-dj fits, risks |
| [QUESTIONS.md](QUESTIONS.md) | Decisions to make before work starts, each with a recommended default |
| [functions.csv](functions.csv) | Every routine in the reference disassembly, its area, size and how much of it the current segments run |
| [tools/dbg2csv.py](tools/dbg2csv.py) | Makes a decomp-dj `csv` symbol map from the reference build (used for the numbers in the plan) |

The reference is [CelestialAmber/TetrisNESDisasm](https://github.com/CelestialAmber/TetrisNESDisasm)
at commit `94581c0`. It is not copied here; Phase 0 of the plan fetches and builds it
into `examples/tetris/build/ref/`, which is gitignored like the rest of `build/`.
