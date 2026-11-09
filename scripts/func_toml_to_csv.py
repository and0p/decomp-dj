#!/usr/bin/env python3
"""Converts `[[func]]` symbol TOML files (`addr` = VRAM, `name`) into dj's csv symbol format,
for N64 code that runs from RAM (docs/coverage.md#symbol-maps).

Each segment is given as NAME:VRAM_START:VRAM_END:ROM_START (hex), for the code that's copied
from ROM_START in the ROM file to VRAM_START. Functions outside every segment are skipped. A
function runs to the next symbol in its segment (or the segment's end), and lands in the
RDRAM region at its physical address (VRAM - 0x80000000, KSEG0), with the ROM file offset
of its first word in the sixth column. Needs Python 3.11 (tomllib).

Usage:
  scripts/func_toml_to_csv.py OUT.csv FILE.toml... --segment main:80000450:80040380:1050 ...

Example, WWF No Mercy (USA) (Rev 1), from and0p/aki_n64_decomp's layout table:
  scripts/func_toml_to_csv.py symbols/nomercy.csv _re_docs/symbols/nomercy/*.toml \\
    --segment main:80000450:80040380:1050 --segment ovl_e:800D9960:80144670:144BE0
"""
import argparse, sys, tomllib


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out")
    ap.add_argument("toml", nargs="+")
    ap.add_argument("--segment", action="append", required=True, help="NAME:VRAM_START:VRAM_END:ROM_START, hex")
    a = ap.parse_args()

    segs = []
    for s in a.segment:
        name, start, end, rom = s.split(":")
        segs.append((name, int(start, 16), int(end, 16), int(rom, 16)))

    funcs = {}
    for path in a.toml:
        with open(path, "rb") as f:
            for e in tomllib.load(f).get("func", []):
                funcs.setdefault(int(e["addr"]), e["name"])

    rows, skipped = [], 0
    for name, start, end, rom in segs:
        inside = sorted((addr, n) for addr, n in funcs.items() if start <= addr < end)
        for i, (addr, n) in enumerate(inside):
            nxt = inside[i + 1][0] if i + 1 < len(inside) else end
            rows.append((n, addr - 0x80000000, nxt - addr, addr, rom + (addr - start)))
    skipped = len(funcs) - len(rows)

    with open(a.out, "w") as out:
        out.write("name,region,offset,size,cpu,file\n")
        for n, off, size, addr, file in sorted(rows, key=lambda r: r[1]):
            out.write(f"{n},RDRAM,{off:X},{size:X},{addr:X},{file:X}\n")
    print(f"{len(rows)} functions written to {a.out}; {skipped} outside the segments skipped", file=sys.stderr)


if __name__ == "__main__":
    main()
