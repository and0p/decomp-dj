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

So "the connectors are MIT like the rest of the repo" holds for the new files, not for the
patches as a whole. A tidy layout: each connector's new files under `connectors/<core>/src/`
with MIT headers, copied in by `build.sh`, and a small patch with only the hooks into
upstream files, marked as under the core's licence. A shared MIT header for the API
(`connectors/include/dj_connector.h`) would serve every connector. The connector READMEs
now say "this patch and the core built from it are GPL"; they would say the above instead.
Not legal advice; worth a second look before the release.

## Steps

1. **One Cargo workspace.** A root `Cargo.toml` over `player/` and `gui/`: one lock file,
   one `target/`, one version (`version.workspace`). `cargo build --release` builds both;
   docs and scripts use `target/release/dj`.
2. **CI** (`.github/workflows/ci.yml`, every push and pull request):
   - Linux (ubuntu-22.04, for an older glibc) and macOS (arm64): build, `cargo test`,
     `cargo clippy`, build the Mesen core and the Thwaite ROM (cached by their pinned
     commits), then `dj run examples/thwaite` to check all 12 hashes. Determinism becomes a
     check on every change.
   - Windows: the same jobs, `continue-on-error: true` until tested by hand. Rust with MSVC,
     Mesen and cc65 with MinGW under MSYS2 (`msys2/setup-msys2`), the OpenGL test skipped
     (no WGL context yet).
3. **Core search path** (above), and the Thwaite example pointing at `mesen_libretro` so it
   needs no `.local/` set-up once the core is built.
4. **Release workflow** (`.github/workflows/release.yml`, on a `v*` tag): build each target
   and zip a folder, then create a draft GitHub Release with the zips and a
   `SHA256SUMS` file. Published by hand.
   ```
   decomp-dj-0.1.0-macos-arm64/
     dj, dj-gui                 (dj.exe, dj-gui.exe on Windows)
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
