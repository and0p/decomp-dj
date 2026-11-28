# Install

Download a release, unzip it, and run. The Linux and macOS downloads include the emulator
cores, so there is nothing to build before a first project.

## Download and run

Each release has a ZIP for Linux (x86_64), macOS (Apple silicon) and Windows (x86_64), with
`SHA256SUMS` to check them against. Unzip it anywhere; nothing is installed.

```sh
unzip decomp-dj-0.1.0-macos-arm64.zip
cd decomp-dj-0.1.0-macos-arm64
xattr -dr com.apple.quarantine .     # macOS only, once (below)
./dj --version
```

Put the folder on your `PATH` to type `dj` from anywhere, or call it by its path. Keep the
folder together: `dj` finds its cores and decoder plugins in the folders beside it.

- **macOS:** the programs aren't signed with a Developer ID, so macOS stops them the first
  time. Run the `xattr` line above once in the unzipped folder, or right-click
  `decomp-dj.app` and choose Open.
- **Windows:** SmartScreen may warn about an unrecognised app: More info > Run anyway.
  `dj-gui.exe` opens no console window; what it would print there doesn't show (a debug
  build from source, `cargo run` without `--release`, keeps its console for that). The
  Windows ZIP has no cores yet: [build one](#building-a-core).
- **Linux:** a core that draws with OpenGL needs `libEGL.so.1`
  ([below](#cores-that-draw-with-opengl)). `linux/install-desktop-entry.sh` gives dj-gui
  its icon in launchers and on Wayland.

### What is in the folder

| In the folder | What it is |
| --- | --- |
| `dj` (`dj.exe`) | The command line ([cli.md](cli.md)) |
| `dj-gui` (`dj-gui.exe`); `decomp-dj.app` on macOS | The GUI ([gui.md](gui.md)) |
| `cores/` | Linux and macOS: the cores, ready to use ([below](#the-cores)) |
| `decoders/` | Decoder plugins: an example plugin ([decoders.md](decoders.md)). A bare `[[decoders]]` path is looked for here |
| `connectors/` | The patches, source files and scripts the cores were built with, to build one yourself |
| `include/` | The C headers for decoder plugins, connectors and comparison cores (MIT) |
| `linux/` | Linux: `install-desktop-entry.sh` and the icon |
| `docs/`, `LICENSE.txt` | These docs, and the licence (MIT) |

### The cores

dj plays a game in a libretro core, and records coverage only with a core that has a
[connector](connectors.md). The Linux and macOS ZIPs carry these six, built from `connectors/`:

| Console | Core | Named in a project as |
| --- | --- | --- |
| NES | Mesen | `mesen_libretro` |
| SNES | Mesen-S | `mesen-s_libretro` |
| Game Boy, Game Boy Color | SameBoy | `sameboy_libretro` |
| Game Boy Advance | mGBA | `mgba_libretro` |
| Nintendo 64 | Mupen64Plus-Next | `mupen64plus_next_libretro` |
| Mega Drive / Genesis | Genesis Plus GX | `genesis_plus_gx_libretro` |

- **You don't point dj at them.** `dj init` picks the one for your ROM's console, and a
  project's `config.toml` names its core without a folder (`path = "mesen_libretro"`),
  which dj looks for in the project and then in `cores/` beside itself
  ([config.md](config.md#core-paths)). So a project works on any machine with a release
  unzipped.
- **Each core is a separate program under its own licence,** not MIT: the text is in
  `cores/<core>.LICENSE.txt`. `cores/README.txt` names each core's exact build and the
  `*-source.tar.gz` file on the same release that holds its complete source.
- **Genesis Plus GX's licence is not a free software licence:** it may be passed on, but
  not sold or used in a commercial product (`cores/genesis_plus_gx_libretro.LICENSE.txt`).
  The other cores are GPL (Mesen, Mesen-S, Mupen64Plus-Next), MPL (mGBA) and Expat
  (SameBoy).
- **Any other libretro core** plays segments and checks hashes too, given by its file
  (`--core path/to/core`), but records no coverage until it has a connector.

## A first project

```sh
cd my-decomp
dj init
```

On a terminal, `dj init` asks for what it needs, one question at a time. Enter takes the answer in brackets:

```text
A new dj project. Press Enter to take the answer in [brackets].

ROM file: game.nes
Project folder [.dj]:
Project name [game]:
  game.nes is an NES ROM. dj suggests the Mesen core, which has a coverage connector.
Core [/opt/decomp-dj/cores/mesen_libretro.so]:
Symbol map, if you have one (.dbg, .mlb, .map, ELF or CSV) [none]:
Make a first segment to record into (60 frames from power-on)? [Y/n]:
```

It then loads the core with the ROM, shows the project it would make, and asks `Create it? [Y/n]`. Nothing is written before that yes; a no, or Ctrl-D at any question, makes nothing and exits with status 1.

- **The ROM** is the one thing with no default. A path can be typed, pasted or dropped into the terminal (quotes, `\ ` and `~/` are understood).
- **The folder** is `.dj` in the current folder unless you name another. dj finds a project in `.dj` from that folder and from any folder below it, as git finds a repository, so the project sits inside the decomp or port it is about. A project in `.dj` is named after the ROM file, one in a folder of its own after the folder.
- **The core** suggested is the one for the ROM's console (NES, SNES, Game Boy, GBA, N64 and Genesis, told by the ROM's header or extension): `mesen_libretro`, `mesen-s_libretro`, `sameboy_libretro`, `mgba_libretro`, `mupen64plus_next_libretro` or `genesis_plus_gx_libretro`, looked for in `$DJ_CORES`, then `cores/` next to `dj` (where a release has them), then `build/cores/` of a source checkout (where the connectors' `build.sh` put them), then `build/cores/` in the current folder. If it isn't there (every core on Windows, for now), dj says which script builds it and asks for a core's file. For another console, give any libretro core's file, or a name to look for in those folders.
- **The first segment** is a root segment of 60 frames from power-on, to record into (`dj new` makes more).

What a flag says isn't asked: `dj init my-game --rom game.nes --name "My Game" --core mesen_libretro --symbols game.dbg --no-segment` asks only for the last yes. **`--yes` (`-y`) asks nothing** and takes the default for everything the flags don't say; so does a `dj init` whose input or output isn't a terminal (a script, CI, an agent). Both need `--rom`:

```sh
dj init --rom game.nes --yes
```

`init` writes `config.toml` with every core option pinned and the console's defaults ([config.md](config.md#console-defaults)), and remembers where the ROM and the core are in `.local/local.toml`. It keeps the ROM out of git: `.local/` goes into the project's `.gitignore`, and so does the ROM when it is in the project's folder. Lines are added to a `.gitignore` that is there, never taken away. A ROM beside `.dj`, in your own repository's folder, is yours to ignore there: dj doesn't edit a `.gitignore` outside the project.

Every command after `init` works on the project it is run in (or the one `-C DIR` names): `dj status` shows the tree. Then open the project in the GUI to record inputs (`dj-gui .dj`), or edit the segment file by hand ([segments.md](segments.md)). A symbol map can also be added later ([coverage.md](coverage.md#symbol-maps)).

## Trying the example

[examples/thwaite/](../examples/thwaite/) is a 16-segment project for Thwaite, a free NES
game. The examples aren't in the ZIP: they are in the repository, and the ROM is built from
source (git, make, a C compiler, and Python 3 with Pillow). With a release's `dj` on your
`PATH`:

```sh
git clone https://github.com/and0p/decomp-dj.git
cd decomp-dj/examples/thwaite
./build-rom.sh                           # the ROM and its symbols, into build/
dj check --rom build/thwaite.nes         # plays every segment and compares its hashes
dj summary --segments                    # coverage, by segment
dj-gui .                                 # macOS: open /path/to/decomp-dj.app --args "$PWD"
```

No core is named: the project's `config.toml` says `mesen_libretro`, and dj finds it in the
release's `cores/`. `--rom` is remembered in `.local/local.toml`, so later commands don't
need it. The example's [README](../examples/thwaite/README.md) walks through the rest.

## Building from source

```sh
cargo build --release                     # Rust 1.95+ (egui 0.37); the binaries are target/release/dj and dj-gui
cargo build --release -p dj-cli           # only the command line (Rust 1.79+)
```

Building needs a C compiler as well as Rust (for the bundled SQLite and zstd, and one small
C file of dj's own).

### Building a core

Each folder in [connectors/](../connectors/) has a `build.sh` that fetches its core at a
pinned commit, applies the connector and builds it into `build/cores/`, where a `dj` built
from source looks. A release's folder has the same scripts; copy the result into its
`cores/`, or give the file with `--core`.

```sh
connectors/mesen/build.sh                 # git, make and a C++11 compiler
connectors/mesen/build.sh --id            # the build id it will report
```

| Core | Needs, besides git and make |
| --- | --- |
| `mesen`, `mesen-s` | A C++11 compiler |
| `sameboy` | A C compiler, `hexdump` |
| `mgba` | A C compiler, cmake |
| `genesis-plus-gx` | A C compiler |
| `mupen64plus` | A C and C++ compiler, pkg-config, zlib and libpng (Homebrew's `libpng` on macOS) |

On Windows, run them from MSYS2's MINGW64 shell. `scripts/cores.py run` builds every core
that is missing and checks each against its example
([connectors/README.md](../connectors/README.md#checking-them)).

### Packaging

- `scripts/package.sh --build --cores` builds the same folder and ZIP as a release, into
  `dist/`. Without `--cores` it packs the cores already in `build/cores/` and builds none.
  Which cores go in is `connectors/cores.toml` (`bundle = false` leaves one out).
- `scripts/cores.py sources DIR` writes each bundled core's source as a tarball, as a
  release carries them (`connectors/<core>/build.sh --source DIR` for one).

## Cores that draw with OpenGL

Cores that render on the GPU (libretro's `SET_HW_RENDER`) are supported with OpenGL, in the
CLI and the GUI alike. Each such core gets a private OpenGL context with no window and a
framebuffer to draw into; frames are read back into memory only when they are shown or
saved, so hashes, previews and screenshots work as with any other core. State hashes come
from memory regions, never from the picture, so GPUs and drivers don't change them.

| Platform | Context | Limits |
| --- | --- | --- |
| macOS | CGL (OpenGL.framework) | Core profiles up to OpenGL 4.1; the compatibility profile is OpenGL 2.1. Without a GPU (a virtual machine) Apple's software renderer is used; `DJ_GL_SOFTWARE=1` asks for it anyway |
| Linux | EGL, without a window (Mesa's surfaceless platform or a GPU device) | Needs `libEGL.so.1`; a machine with no GPU can use Mesa's software renderer |
| Windows | none yet | |

A core that asks for something unavailable (Vulkan, OpenGL ES, Direct3D, or a version too
new) is told no, so it can fall back to software rendering; if it can't, loading fails and
says what it asked for. `crates/dj-core/tests/testcore/glcore.c` is a small OpenGL core for tests.

## Icons

The icons are `assets/icon_gui.png` (dj-gui) and `assets/icon_cli.png` (dj).

- **dj-gui's window** carries its icon on every system: the title bar and taskbar on Windows
  and Linux (X11), the Dock on macOS.
- **Windows:** both `.exe` files have their icon built in (`build.rs`, from
  `assets/icon_*.ico`), as Explorer shows them. This needs a resource compiler: `rc.exe`
  from the Windows SDK (with Visual Studio or its Build Tools), or `windres` with MinGW;
  without one, the build warns and the files get no icon.
- **macOS:** a bare executable has no icon in Finder. `crates/dj-gui/macos/bundle.sh` builds dj-gui and
  wraps it in `target/release/decomp-dj.app`, with the icon, to keep in Applications.
- **Linux:** on Wayland, the taskbar shows the icon of the desktop entry that matches the
  window's app id (`decomp-dj`). `crates/dj-gui/linux/install-desktop-entry.sh` installs that entry and
  the icon for you (in `~/.local/share`), pointing at the dj-gui you built.
- **dj** gets an icon only on Windows: macOS and Linux have nowhere to show one for a
  command-line program.

After changing a PNG, `scripts/make_icons.py` makes the `.ico` files and (on macOS) the
`.icns` again; they're committed, so building doesn't need it.
