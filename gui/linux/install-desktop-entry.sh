#!/usr/bin/env bash
# Installs a desktop entry and icon for dj-gui for this user, so launchers, the taskbar and
# Wayland desktops show its icon (the window's app id is decomp-dj, which the entry's name
# matches). X11 shows the icon the window sets itself without this.
# Usage: gui/linux/install-desktop-entry.sh [path to dj-gui]   (default: gui/target/release/dj-gui)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
exe="$(cd "$(dirname "${1:-$here/../target/release/dj-gui}")" && pwd)/$(basename "${1:-dj-gui}")"
[ -x "$exe" ] || { echo "no dj-gui at $exe: build it first (cd gui && cargo build --release)"; exit 1; }
data="${XDG_DATA_HOME:-$HOME/.local/share}"
mkdir -p "$data/icons/hicolor/192x192/apps" "$data/applications"
cp "$here/../../assets/icon_gui.png" "$data/icons/hicolor/192x192/apps/decomp-dj.png"
sed "s|^Exec=dj-gui|Exec=\"$exe\"|" "$here/decomp-dj.desktop" > "$data/applications/decomp-dj.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$data/icons/hicolor" || true
echo "installed $data/applications/decomp-dj.desktop (runs $exe)"
