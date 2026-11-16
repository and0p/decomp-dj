#!/usr/bin/env python3
"""Generates src/data/chr.c, palette.c and texts.c from Thwaite's source.

Run by hand when the pinned Thwaite commit changes. The output is committed, so the core
builds without this script. Needs only Python 3 (it reads the PNGs itself).

    python3 tools/gen_assets.py ../build/thwaite-nes [../build/thwaite.nes]

With the ROM, it checks the converted CHR against the ROM's, byte for byte.

- CHR: tilesets/maingfx.png (tiles 0-383) then tilesets/cuthouses.png (384-511), converted
  the way Thwaite's own tools/pilbmp2nes.py does: 8x8 tiles left to right, top to bottom,
  each as two bit planes. That is the ROM's 8 KB CHR.
- The NES master palette: Mesen's default ("2C02"), the one the baseline core draws with
  (mesen_palette = "Default" in config.toml), so the decomp's colours match.
- Texts: the screens' text (texts.txt), as tile numbers, one line per row. Read from the
  build's obj/nes/dtescripts.s, which Thwaite's paginate.py makes from texts.txt with
  charset.py's mapping, and decoded from its byte-pair (DTE) compression here.
"""

TEXTS = ["todo_txt", "practice_txt"]
import os
import struct
import sys
import zlib

# Mesen's default palette, Libretro/libretro.cpp `defaultPalette` at the pinned Mesen commit.
MESEN_PALETTE = [
    0x666666, 0x002A88, 0x1412A7, 0x3B00A4, 0x5C007E, 0x6E0040, 0x6C0600, 0x561D00,
    0x333500, 0x0B4800, 0x005200, 0x004F08, 0x00404D, 0x000000, 0x000000, 0x000000,
    0xADADAD, 0x155FD9, 0x4240FF, 0x7527FE, 0xA01ACC, 0xB71E7B, 0xB53120, 0x994E00,
    0x6B6D00, 0x388700, 0x0C9300, 0x008F32, 0x007C8D, 0x000000, 0x000000, 0x000000,
    0xFFFEFF, 0x64B0FF, 0x9290FF, 0xC676FF, 0xF36AFF, 0xFE6ECC, 0xFE8170, 0xEA9E22,
    0xBCBE00, 0x88D800, 0x5CE430, 0x45E082, 0x48CDDE, 0x4F4F4F, 0x000000, 0x000000,
    0xFFFEFF, 0xC0DFFF, 0xD3D2FF, 0xE8C8FF, 0xFBC2FF, 0xFEC4EA, 0xFECCC5, 0xF7D8A5,
    0xE4E594, 0xCFEF96, 0xBDF4AB, 0xB3F3CC, 0xB5EBF2, 0xB8B8B8, 0x000000, 0x000000,
]


def read_indexed_png(path):
    """Width, height and rows of palette indices, for a non-interlaced indexed PNG."""
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path}: not a PNG")
    pos, idat, w = 8, b"", None
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos : pos + 8])
        body = data[pos + 8 : pos + 8 + n]
        pos += 12 + n
        if kind == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if ctype != 3 or interlace:
                raise SystemExit(f"{path}: expected an indexed, non-interlaced PNG")
        elif kind == b"IDAT":
            idat += body
    raw = zlib.decompress(idat)
    stride = (w * depth + 7) // 8
    bpp = 1  # filters work on whole bytes; indexed images have at most one byte per pixel
    rows, prev, i = [], bytearray(stride), 0
    for _ in range(h):
        ftype, line = raw[i], bytearray(raw[i + 1 : i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if ftype == 1:
                line[x] = (line[x] + a) & 0xFF
            elif ftype == 2:
                line[x] = (line[x] + b) & 0xFF
            elif ftype == 3:
                line[x] = (line[x] + (a + b) // 2) & 0xFF
            elif ftype == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pred) & 0xFF
        prev = line
        per = 8 // depth
        px = []
        for x in range(w):
            byte = line[x // per]
            shift = 8 - depth * (x % per + 1)
            px.append((byte >> shift) & ((1 << depth) - 1))
        rows.append(px)
    return w, h, rows


def to_chr(path):
    w, h, rows = read_indexed_png(path)
    out = bytearray()
    for ty in range(0, h, 8):
        for tx in range(0, w, 8):
            lo, hi = bytearray(8), bytearray(8)
            for y in range(8):
                for x in range(8):
                    v = rows[ty + y][tx + x]
                    if v > 3:
                        raise SystemExit(f"{path}: pixel ({tx + x}, {ty + y}) uses colour {v}, past 3")
                    lo[y] |= (v & 1) << (7 - x)
                    hi[y] |= (v >> 1) << (7 - x)
            out += lo + hi
    return bytes(out)


def read_dte_scripts(path):
    """The labelled byte strings of dtescripts.s, and its replacement table."""
    labels, cur = {}, None
    for line in open(path, encoding="utf-8"):
        line = line.split(";")[0].rstrip()
        if not line:
            continue
        if line.endswith(":") and not line.startswith((" ", "\t")):
            cur = line[:-1]
            labels[cur] = []
        elif line.strip().startswith(".byte") and cur:
            labels[cur] += [int(v) for v in line.strip()[5:].split(",")]
    return labels


def undte(data, repl):
    out = []

    def put(c):
        if c >= 0x80:
            put(repl[(c - 0x80) * 2])
            put(repl[(c - 0x80) * 2 + 1])
        else:
            out.append(c)
    for c in data:
        if c == 0:
            break
        put(c)
    return bytes(out)


def c_bytes(data, per_line=16):
    return "\n".join("    " + ", ".join(f"0x{b:02X}" for b in data[i : i + per_line]) + ","
                     for i in range(0, len(data), per_line))


def main():
    if len(sys.argv) not in (2, 3):
        raise SystemExit(__doc__)
    thwaite = sys.argv[1]
    here = os.path.dirname(os.path.abspath(__file__))
    out_dir = os.path.join(here, "..", "src", "data")
    os.makedirs(out_dir, exist_ok=True)

    chr_data = to_chr(os.path.join(thwaite, "tilesets", "maingfx.png")) + \
        to_chr(os.path.join(thwaite, "tilesets", "cuthouses.png"))
    if len(chr_data) != 8192:
        raise SystemExit(f"CHR is {len(chr_data)} bytes, expected 8192")
    if len(sys.argv) == 3:
        rom = open(sys.argv[2], "rb").read()
        prg = rom[4] * 16384
        rom_chr = rom[16 + prg : 16 + prg + 8192]
        if rom_chr != chr_data:
            first = next(i for i in range(8192) if rom_chr[i] != chr_data[i])
            raise SystemExit(f"CHR differs from the ROM's at byte 0x{first:X}")
        print("CHR matches the ROM's")

    open(os.path.join(out_dir, "chr.c"), "w").write(
        "/* Generated by tools/gen_assets.py from tilesets/maingfx.png and cuthouses.png. Do not edit. */\n"
        '#include "../data.h"\n\n'
        f"const unsigned char thw_chr[8192] = {{\n{c_bytes(chr_data)}\n}};\n")
    pal = "\n".join("    " + ", ".join(f"0x{v:06X}" for v in MESEN_PALETTE[i : i + 8]) + ","
                    for i in range(0, 64, 8))
    open(os.path.join(out_dir, "palette.c"), "w").write(
        "/* Generated by tools/gen_assets.py: Mesen's default NES palette, as 0xRRGGBB. Do not edit. */\n"
        '#include "../data.h"\n\n'
        f"const unsigned thw_master_palette[64] = {{\n{pal}\n}};\n")
    labels = read_dte_scripts(os.path.join(thwaite, "obj", "nes", "dtescripts.s"))
    repl = labels["dte_replacements"]
    t = ["/* Generated by tools/gen_assets.py from texts.txt (via dtescripts.s). Do not edit.",
         " * Tile numbers; 0x0A ends a row; 0 ends the text. */", '#include "../data.h"', ""]
    for name in TEXTS:
        text = undte(labels[name], repl) + b"\0"
        t.append(f"const unsigned char thw_{name}[{len(text)}] = {{\n{c_bytes(text)}\n}};")
        shown = text[:-1].decode("latin-1").replace("\x1c", "(c)")
        print(f"{name}:\n  " + "\n  ".join(shown.split("\n")))
    open(os.path.join(out_dir, "texts.c"), "w").write("\n".join(t) + "\n")
    print("wrote chr.c (512 tiles), palette.c and texts.c")


if __name__ == "__main__":
    main()
