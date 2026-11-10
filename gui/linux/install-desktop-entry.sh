#!/usr/bin/env bash
# Installs a desktop entry and icon for dj-gui for this user, so launchers, the taskbar and
# Wayland desktops show its icon (the window's app id is decomp-dj, which the entry's name
# matches). X11 shows the icon the window sets itself without this.
# Usage: install-desktop-entry.sh [path to dj-gui]
#   Default: the dj-gui beside this folder (the portable ZIP), else gui/target/release/dj-gui
#   (a build from source).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
if [ -n "${1:-}" ]; then
  exe="$1"
elif [ -x "$here/../dj-gui" ]; then
  exe="$here/../dj-gui"
else
  exe="$here/../target/release/dj-gui"
fi
[ -x "$exe" ] || { echo "no dj-gui at $exe: build it first (cd gui && cargo build --release), or name it"; exit 1; }
exe="$(cd "$(dirname "$exe")" && pwd)/$(basename "$exe")"
icon="$here/decomp-dj.png"
[ -f "$icon" ] || icon="$here/../../assets/icon_gui.png"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
mkdir -p "$data/icons/hicolor/192x192/apps" "$data/applications"
cp "$icon" "$data/icons/hicolor/192x192/apps/decomp-dj.png"
sed "s|^Exec=dj-gui|Exec=\"$exe\"|" "$here/decomp-dj.desktop" > "$data/applications/decomp-dj.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$data/icons/hicolor" || true
echo "installed $data/applications/decomp-dj.desktop (runs $exe)"
