# Install

## Portable builds

Each release has a ZIP for Linux (x86_64), macOS (Apple silicon) and Windows (x86_64), with
`SHA256SUMS` to check them against. Nothing to install: unzip it anywhere and run.

| In the folder | What it is |
| --- | --- |
| `dj` (`dj.exe`) | The command line ([cli.md](cli.md)) |
| `dj-gui` (`dj-gui.exe`); `decomp-dj.app` on macOS | The GUI ([gui.md](gui.md)) |
| `decoders/` | Decoder plugins: the C header and an example plugin ([decoders.md](decoders.md)). A bare `[[decoders]]` path is looked for here |
| `connectors/` | The patches and scripts that build a core with a coverage connector. Cores aren't included |
| `linux/` | Linux: `install-desktop-entry.sh`, so launchers and Wayland desktops show dj-gui's icon |
| `docs/`, `LICENSE.txt` | These docs, and the licence |

- **macOS:** the app isn't signed with a Developer ID, so macOS stops it the first time.
  After unzipping, run `xattr -dr com.apple.quarantine decomp-dj-*` once in the folder you
  unzipped into, or right-click `decomp-dj.app` and choose Open.
- **Windows:** SmartScreen may warn about an unrecognised app: More info > Run anyway.
- `scripts/package.sh --build` builds the same folder and ZIP from source, into `dist/`.

## Building from source

decomp-dj has three parts: the CLI, the GUI and a patched core that reports coverage.

```sh
cd player && cargo build --release        # Rust 1.79+; the binary is player/target/release/dj
cd gui && cargo build --release           # Rust 1.95+ (egui 0.37); the binary is gui/target/release/dj-gui
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
- **macOS:** a bare executable has no icon in Finder. `gui/macos/bundle.sh` builds dj-gui and
  wraps it in `gui/target/release/decomp-dj.app`, with the icon, to keep in Applications.
- **Linux:** on Wayland, the taskbar shows the icon of the desktop entry that matches the
  window's app id (`decomp-dj`). `gui/linux/install-desktop-entry.sh` installs that entry and
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
| macOS | CGL (OpenGL.framework) | Core profiles up to OpenGL 4.1; the compatibility profile is OpenGL 2.1 |
| Linux | EGL, without a window (Mesa's surfaceless platform or a GPU device) | Needs `libEGL.so.1`; a machine with no GPU can use Mesa's software renderer |
| Windows | none yet | |

A core that asks for something unavailable (Vulkan, OpenGL ES, Direct3D, or a version too
new) is told no, so it can fall back to software rendering; if it can't, loading fails and
says what it asked for. `player/tests/testcore/glcore.c` is a small OpenGL core for tests.

## Trying the example

[examples/thwaite/](../examples/thwaite/) is a 12-segment project for Thwaite, a free NES
game. Its README walks through it; in short, from the repository root:

```sh
(cd examples/thwaite && ./build-rom.sh)          # the ROM and symbols, into examples/thwaite/build/
dj run examples/thwaite --rom examples/thwaite/build/thwaite.nes --core build/cores/mesen_libretro   # the .so/.dylib/.dll suffix is added
dj report examples/thwaite
gui/target/release/dj-gui examples/thwaite
```

## A first project

```sh
dj init my-game --name "My Game" --rom game.nes --core build/cores/mesen_libretro.so
dj new my-game --title "Boot to title" --frames 240
```

`init` writes `config.toml` with every core option pinned, remembers the ROM and core
paths in `.local/local.toml`, and creates a `.gitignore` that lists `.local/` when the project has none. Pass `--symbols FILE` to
add a symbol map ([coverage.md](coverage.md#symbol-maps)). Then open the project in the
GUI to record inputs, or edit the segment file by hand ([segments.md](segments.md)).
