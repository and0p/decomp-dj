# Install

## Portable builds

Each release has a ZIP for Linux (x86_64), macOS (Apple silicon) and Windows (x86_64), with
`SHA256SUMS` to check them against. Nothing to install: unzip it anywhere and run.

| In the folder | What it is |
| --- | --- |
| `dj` (`dj.exe`) | The command line ([cli.md](cli.md)) |
| `dj-gui` (`dj-gui.exe`); `decomp-dj.app` on macOS | The GUI ([gui.md](gui.md)) |
| `decoders/` | Decoder plugins: an example plugin ([decoders.md](decoders.md)). A bare `[[decoders]]` path is looked for here |
| `connectors/` | The patches, source files and scripts that build a core with a coverage connector. Cores aren't included |
| `include/` | The C headers for decoder plugins, connectors and comparison cores (MIT) |
| `linux/` | Linux: `install-desktop-entry.sh`, so launchers and Wayland desktops show dj-gui's icon |
| `docs/`, `LICENSE.txt` | These docs, and the licence (MIT; each connector's patch has its core's licence, in the connector's own `LICENSE.txt`) |

- **macOS:** the app isn't signed with a Developer ID, so macOS stops it the first time.
  After unzipping, run `xattr -dr com.apple.quarantine decomp-dj-*` once in the folder you
  unzipped into, or right-click `decomp-dj.app` and choose Open.
- **Windows:** SmartScreen may warn about an unrecognised app: More info > Run anyway.
  `dj-gui.exe` opens no console window; what it would print there doesn't show (a debug
  build from source, `cargo run` without `--release`, keeps its console for that).
- `scripts/package.sh --build` builds the same folder and ZIP from source, into `dist/`.

## Building from source

decomp-dj has three parts: the CLI, the GUI and a patched core that reports coverage.

```sh
cargo build --release                     # Rust 1.95+ (egui 0.37); the binaries are target/release/dj and dj-gui
cargo build --release -p dj-cli           # only the command line (Rust 1.79+)
connectors/mesen/build.sh                 # git, make, a C++11 compiler; output in build/cores/
```

The core is Mesen (NES), patched as described in
[connectors/mesen/README.md](../connectors/mesen/README.md). Any other libretro core plays
segments too, but records no coverage until it has a [connector](connectors.md).

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

## Trying the example

[examples/thwaite/](../examples/thwaite/) is a 12-segment project for Thwaite, a free NES
game. Its README walks through it; in short, from the repository root:

```sh
(cd examples/thwaite && ./build-rom.sh)          # the ROM and symbols, into examples/thwaite/build/
dj -C examples/thwaite check --rom examples/thwaite/build/thwaite.nes --core build/cores/mesen_libretro   # the .so/.dylib/.dll suffix is added
dj -C examples/thwaite summary --segments
target/release/dj-gui examples/thwaite
```

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
- **The core** suggested is the one for the ROM's console (NES, SNES, Game Boy, GBA, N64 and Genesis, told by the ROM's header or extension): `mesen_libretro`, `mesen-s_libretro`, `sameboy_libretro`, `mgba_libretro`, `mupen64plus_next_libretro` or `genesis_plus_gx_libretro`, looked for in `$DJ_CORES`, then `cores/` next to `dj`, then `build/cores/` of a source checkout (where the connectors' `build.sh` put them), then `build/cores/` in the current folder. If it isn't there, dj says which script builds it and asks for a core's file. For another console, give any libretro core's file, or a name to look for in those folders.
- **The first segment** is a root segment of 60 frames from power-on, to record into (`dj new` makes more).

What a flag says isn't asked: `dj init my-game --rom game.nes --name "My Game" --core mesen_libretro --symbols game.dbg --no-segment` asks only for the last yes. **`--yes` (`-y`) asks nothing** and takes the default for everything the flags don't say; so does a `dj init` whose input or output isn't a terminal (a script, CI, an agent). Both need `--rom`:

```sh
dj init --rom game.nes --yes
```

`init` writes `config.toml` with every core option pinned and the console's defaults ([config.md](config.md#console-defaults)), and remembers where the ROM and the core are in `.local/local.toml`. It keeps the ROM out of git: `.local/` goes into the project's `.gitignore`, and so does the ROM when it is in the project's folder. Lines are added to a `.gitignore` that is there, never taken away. A ROM beside `.dj`, in your own repository's folder, is yours to ignore there: dj doesn't edit a `.gitignore` outside the project.

Every command after `init` works on the project it is run in (or the one `-C DIR` names): `dj status` shows the tree. Then open the project in the GUI to record inputs (`dj-gui .dj`), or edit the segment file by hand ([segments.md](segments.md)). A symbol map can also be added later ([coverage.md](coverage.md#symbol-maps)).
