# Plan: CI, packaging and the first release

Status: planning, agreed 2026-11-09. Nothing here is built yet. Package managers (Homebrew
and the rest) come later: [ROADMAP.md](ROADMAP.md#project-and-cli).

## Decisions

| Question | Decision |
| --- | --- |
| Executables | Keep `dj` (CLI) and `dj-gui` (GUI), two programs. One program can't be both a console and a windowed app on Windows. Maybe later: `dj gui [project]` starts the `dj-gui` next to it |
| Repository | `github.com/and0p/decomp-dj` (the old `emu_coverage` name redirects; local remotes can be updated with `git remote set-url origin git@github.com:and0p/decomp-dj.git`) |
| Targets | macOS arm64, Linux x86_64, Windows x86_64 |
| Signing | Unsigned. The install docs explain clearing macOS quarantine (`xattr -dr com.apple.quarantine <folder>`, or right-click > Open) |
| Cores | Not shipped. The release carries the connectors (patches and `build.sh`); users build their cores. CI builds Mesen only to test |
| Example ROM | Not shipped. The Thwaite README says how to build it (`build-rom.sh`) |
| Core paths | A bare name (`mesen_libretro`) is looked for in the project folder, then `<exe dir>/cores/`. Relative paths are from the project folder, absolute paths are used as they are, so a `local.toml` can always override |
| First version | `v0.1.0`, after projects record the format and tool version (ROADMAP, "Versioning projects") |
| Decoders | Explore loadable decoders before the release: [DECODER_PLAN.md](DECODER_PLAN.md) |

## Licensing of the connectors (to settle)

A connector is a patch against a GPL core (Mesen GPL-3.0, Mupen64Plus-Next GPL-2.0). Its
parts are different:

- **New files** (`Core/DjCoverage.cpp`/`.h`, `libretro/dj_connector.c`/`.h`) are our own
  code and can be MIT. MIT code can be combined into a GPL program.
- **Changes to upstream files** (`Core/CPU.cpp`, `libretro.cpp`, `pure_interp.c`,
  Makefiles) are modifications of GPL code, and the patch carries upstream lines as context.
  Once applied they are part of a GPL work, and the built core is GPL in any case.

So "the connectors are MIT" holds for the new files, not for the patches as a whole. (The
repository itself is GPL-3.0-or-later, with the decoder header, the decoder crates and the
headers in `connectors/include/` under MIT: README.md, "License".) A tidy layout: each connector's new files under `connectors/<core>/src/`
with MIT headers, copied in by `build.sh`, and a small patch with only the hooks into
upstream files, marked as under the core's licence. A shared MIT header for the API
(`connectors/include/dj_connector.h`) would serve every connector. The connector READMEs
now say "this patch and the core built from it are GPL"; they would say the above instead.
Not legal advice; worth a second look before the release.

**As it stands (looked at 2026-11-16):**

| Connector | Core's licence | New files in the patch | Changes to upstream files | New files say |
| --- | --- | --- | --- | --- |
| Mesen | GPL-3.0 | 356 lines (`DjCoverage.cpp`, `.h`) | 151 lines in 5 files | nothing |
| mGBA | MPL-2.0 | 284 lines (`dj_connector.c`, `.h`) | 88 lines in 3 files | MIT |
| Mupen64Plus-Next | GPL-2.0 | 412 lines (`dj_connector.c`, `.h`) | 373 lines in 6 files | nothing |

- **Each connector is still one patch.** Only the shared header exists
  (`connectors/include/dj_connector.h`, MIT).
- **Splitting a patch changes its build id** (`dj_build_id` hashes the patch file), so
  every core has to be rebuilt and every project repinned: Thwaite, Tetris, the GBA test
  and No Mercy. After a split the id should hash the hooks patch and the files under
  `src/` together.
- **A cheaper time to do it** is with the next change that needs a repin anyway (the
  roadmap queues Mesen's lag counter fix for one).
- **Decided 2026-11-17 (the developer): split later,** with that change. Licence headers
  on the new files wait for it too, since any edit to a patch changes its build id.

## Clean-up (started 2026-11-16)

| Item | Status |
| --- | --- |
| Terminology in the CLI help and messages | Done: "segment", "project" and "root segment" in every `dj` help page and error message that said "replay", "collection" or "trunk". The code's own names (`Replay`, `Collection`, `start_trunk`) are unchanged |
| `.gitignore` | Fixed: `scratchpad.md` and the comment after it had run into one line, so neither applied |
| `scratchpad.md` (root) | The developer's notes and a README draft. Tracked since the CI commit, because of the line above. Left alone |
| `player/examples/` spike tools | Done 2026-11-17 (the developer: delete): `mesen_spike`, `n64_spike`, `n64_roundtrip` and `tick_check` removed. The plans that name them say so. `ramtrace` stays: the Tetris example's README uses it |
| `examples/tetris/.decomp/_old/` | Done 2026-11-17 (the developer: delete): the earlier ROM-based port, 26 files with three `.tgz` archives, removed |
| The Tetris data tables | Done 2026-11-17 (the developer: generate at build): `decomp/src/data.c` is out of git and ignored. `tools/gen_data.py` reads the 29 tables from the user's ROM, after checking its sha256, using the names, addresses and lengths in `data.h`, which stays. `make ROM=...` the first time. Checked: the file it writes is byte for byte the one that was committed, and the 30 Tetris segments pass parity against the rebuilt decomp. The tables are still in git history |
| `.agents/` | Plans, notes and archives (`_notes.md`, `REFACTOR.md`, `RESULTS.md`, `yield-parity/archive/`). Not looked at: whether a public repository should carry them is the developer's call |

## Steps

1. **One Cargo workspace.** A root `Cargo.toml` over `player/` and `gui/`: one lock file,
   one `target/`, one version (`version.workspace`). `cargo build --release` builds both;
   docs and scripts use `target/release/dj`.
2. **CI** (`.github/workflows/ci.yml`, every push to main and pull request). **Written
   2026-11-10, not run yet** (it runs once pushed): build, test, clippy (reported, not failing)
   and package on Linux, macOS and Windows (Windows allowed to fail), the ZIPs kept as
   artifacts; and Thwaite replayed from power-on with the Mesen core on Linux and macOS (core
   and ROM cached by their pinning scripts). The GBA test project isn't replayed yet (it needs
   LLVM and an ELF linker on the runner). Before it was written, the player and GUI were
   type-checked for Windows here (`cargo check --target x86_64-pc-windows-gnu`, C
   dependencies stubbed): no errors. As planned:
   - Linux (ubuntu-22.04, for an older glibc) and macOS (arm64): build, `cargo test`,
     `cargo clippy`, build the Mesen core and the Thwaite ROM (cached by their pinned
     commits), then `dj run examples/thwaite` to check all 12 hashes. Determinism becomes a
     check on every change.
   - Windows: the same jobs, `continue-on-error: true` until tested by hand. Rust with MSVC,
     Mesen and cc65 with MinGW under MSYS2 (`msys2/setup-msys2`), the OpenGL test skipped
     (no WGL context yet).
3. **Core search path** (above), and the Thwaite example pointing at `mesen_libretro` so it
   needs no `.local/` set-up once the core is built.
4. **Release workflow** (`.github/workflows/release.yml`, on a `v*` tag). **Written
   2026-11-10, not run yet:** each system runs `scripts/package.sh --build` (also usable
   locally; tried on macOS: the unzipped folder runs, the app's signature survives), a last
   job writes `SHA256SUMS` and makes a draft release with the ZIPs; a hand-run builds the ZIPs
   as artifacts only, and a tag must match `player/Cargo.toml`'s version. The ZIP holds `dj`,
   `dj-gui` (`decomp-dj.app` on macOS), `decoders/` (the header and the reference plugin),
   `connectors/` (patches and scripts, no cores), `linux/` (desktop entry), `docs/`, the
   licence and README; no examples. As planned: build each target
   and zip a folder, then create a draft GitHub Release with the zips and a
   `SHA256SUMS` file. Published by hand.
   ```
   decomp-dj-0.1.0-macos-arm64/
     dj, dj-gui                 (dj.exe, dj-gui.exe on Windows, with their icons built in;
                                macOS: decomp-dj.app from gui/macos/bundle.sh instead of
                                a bare dj-gui; Linux: gui/linux/ with the desktop entry)
     connectors/                (patches, build scripts, READMEs)
     decoders/                  (dj_decoder.h, the dj-decoder crate, the reference plugin)
     examples/thwaite/          (config, segments, README with build-rom.sh)
     docs/
     README.md, LICENSE, CHANGELOG.md
   ```
5. **Docs.** install.md becomes "download, unzip, build a core, open the example", with
   building from source second and the macOS quarantine note; `CHANGELOG.md` starts.
6. **Windows by hand, last.** Thwaite hashes, the GUI, cores without OpenGL; then remove
   `continue-on-error`. Known gaps stay in the docs: no OpenGL context (WGL), and no job
   object for `dj mcp` workers.

## Open

- Whether CI should also check Tetris (it needs the ROM, so only on a machine that has it).
- Whether a release should carry a prebuilt `cc65`-free way to get the Thwaite ROM
  (decided no for now).
