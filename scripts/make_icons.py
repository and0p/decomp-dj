#!/usr/bin/env python3
"""Makes the application icons from assets/icon_gui.png and assets/icon_cli.png: a Windows
.ico for each (embedded in dj-gui.exe and dj.exe by their build scripts) and, on macOS, an
.icns for the GUI's app bundle (gui/macos/bundle.sh). The outputs are committed, so builds
don't need this; run it again after changing a PNG.

The icons are pixel art. Every size is made nearest-neighbour (from the art's own grid when
the image is a whole-number scale of one, else from the image), so each pixel keeps one of
the art's colours and edges stay sharp. Standard library only, plus macOS's iconutil for
the .icns.

Usage: scripts/make_icons.py
"""
import os, shutil, struct, subprocess, sys, tempfile, zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(HERE, "..", "assets")


def read_png(path):
    """(width, height, rows of RGBA tuples) for a non-interlaced PNG: 8-bit RGB or RGBA, or a
    palette of 1, 2, 4 or 8 bits."""
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", path
    pos, idat, palette, alpha = 8, b"", [], b""
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, color = struct.unpack(">IIBB", body[:10])
            assert body[12] == 0, f"{path}: interlaced PNGs aren't read"
            assert (color in (2, 6) and depth == 8) or (color == 3 and depth in (1, 2, 4, 8)), f"{path}: 8-bit RGB(A) or a palette"
        elif kind == b"PLTE":
            palette = [tuple(body[i:i + 3]) for i in range(0, len(body), 3)]
        elif kind == b"tRNS":
            alpha = body
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    bits = depth * {2: 3, 6: 4, 3: 1}[color]
    bpp = max(1, bits // 8)
    stride = (w * bits + 7) // 8
    raw = zlib.decompress(idat)
    rows, prev = [], bytearray(stride)
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        if color == 3:
            per = 8 // depth
            idx = [(line[x // per] >> (8 - depth * (x % per + 1))) & ((1 << depth) - 1) for x in range(w)]
            rows.append([palette[i] + ((alpha[i] if i < len(alpha) else 255),) for i in idx])
        else:
            n = 3 if color == 2 else 4
            rows.append([tuple(line[x * n:x * n + 3]) + ((line[x * n + 3],) if n == 4 else (255,)) for x in range(w)])
        prev = line
    return w, h, rows


def write_png(path, size, pixel):
    """A size x size RGBA PNG, pixel(x, y) -> RGBA."""
    raw = b"".join(b"\x00" + bytes(c for x in range(size) for c in pixel(x, y)) for y in range(size))
    chunk = lambda kind, body: struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    open(path, "wb").write(png)
    return png


def grid(rows):
    """The pixel art's own grid: the largest block size every block of which is one colour."""
    n = len(rows)
    for scale in range(n, 0, -1):
        if n % scale == 0 and all(rows[y][x] == rows[y - y % scale][x - x % scale] for y in range(n) for x in range(n)):
            return [[rows[y * scale][x * scale] for x in range(n // scale)] for y in range(n // scale)]


def sized(art, size):
    g = len(art)
    return lambda x, y: art[y * g // size][x * g // size]


def make(name, icns):
    w, h, rows = read_png(os.path.join(ASSETS, f"{name}.png"))
    assert w == h, "square icons only"
    art = grid(rows)
    print(f"{name}.png: {w}x{h}" + (f", pixel art on a {len(art)}x{len(art)} grid" if len(art) < w else ""))
    # .ico: PNG-compressed entries (Windows Vista and later), 16 to 256.
    sizes = [16, 24, 32, 48, 64, 128, 256]
    with tempfile.TemporaryDirectory() as tmp:
        pngs = [write_png(os.path.join(tmp, f"{s}.png"), s, sized(art, s)) for s in sizes]
    head = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    entries = b""
    for s, png in zip(sizes, pngs):
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(png), offset)
        offset += len(png)
    ico = os.path.join(ASSETS, f"{name}.ico")
    open(ico, "wb").write(head + entries + b"".join(pngs))
    print(f"  wrote {os.path.relpath(ico)}")
    if not icns:
        return
    if not shutil.which("iconutil"):
        print("  no iconutil (macOS only): .icns not made")
        return
    with tempfile.TemporaryDirectory() as tmp:
        iconset = os.path.join(tmp, f"{name}.iconset")
        os.mkdir(iconset)
        for s in [16, 32, 128, 256, 512]:
            write_png(os.path.join(iconset, f"icon_{s}x{s}.png"), s, sized(art, s))
            write_png(os.path.join(iconset, f"icon_{s}x{s}@2x.png"), 2 * s, sized(art, 2 * s))
        out = os.path.join(ASSETS, f"{name}.icns")
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", out], check=True)
        print(f"  wrote {os.path.relpath(out)}")


if __name__ == "__main__":
    make("icon_gui", icns=True)
    make("icon_cli", icns=False)
