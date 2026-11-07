#!/usr/bin/env python3
"""Make a decomp-dj `csv` symbol map from a TetrisNESDisasm build.

usage: dbg2csv.py <disasm dir> <out.csv>

decomp-dj's ca65-dbg reader finds no functions in this disassembly: its code is in
segments named PRG_chunk1..3 rather than CODE, and it has no .proc scopes. This script
takes label addresses from tetris.dbg (written by ld65) and reads the .asm sources to
decide which labels are routines:

- A label is code if the first statement after it is an instruction, data if it is
  .byte, .word, .addr, .incbin, .include, .res or .dbyt.
- Local @labels are never routines. Branch-only Lxxxx labels fold into the routine
  around them; an Lxxxx label is a routine only if something JSRs or JMPs to it or a
  jump table lists it.
- A routine runs up to the next kept label (code or data) in its segment.

Output lines are `name,PRG,offset,size,cpu_addr` in hex, as coverage.md describes.
"""
import glob
import os
import re
import sys

DATA_DIRECTIVES = ('.byte', '.word', '.addr', '.incbin', '.include', '.res', '.dbyt')
IGNORED_DIRECTIVES = ('.if', '.else', '.endif')


def main(disasm, out):
    segs, syms = {}, []
    for line in open(os.path.join(disasm, 'tetris.dbg')):
        kind, _, rest = line.rstrip('\n').partition('\t')
        f = dict(re.findall(r'(\w+)=("[^"]*"|[^,]+)', rest))
        if kind == 'seg' and 'ooffs' in f and f['name'].strip('"') not in ('HEADER', 'CHR'):
            segs[f['id']] = (int(f['start'], 16), int(f['size'], 16), int(f['ooffs']))
        elif kind == 'sym' and f.get('type') == 'lab':
            syms.append((int(f['val'], 16), f['name'].strip('"'), f.get('seg')))
    syms = [s for s in syms if s[2] in segs]

    lines = []
    for fn in sorted(glob.glob(os.path.join(disasm, '**', '*.asm'), recursive=True)):
        rel = os.path.relpath(fn, disasm)
        if rel.startswith(('cc65', 'tools')):
            continue
        lines += open(fn, encoding='latin-1').read().split('\n')
    source = '\n'.join(lines)

    kinds = {}
    label = re.compile(r'^([A-Za-z_][A-Za-z0-9_]*):(.*)$')
    for i, l in enumerate(lines):
        m = label.match(l)
        if not m or ':=' in l:
            continue
        nxt, j = m.group(2).split(';')[0].strip(), i
        while not nxt and j + 1 < len(lines):
            j += 1
            s = lines[j].split(';')[0].strip()
            if s and not s.startswith(IGNORED_DIRECTIVES):
                nxt = s
        kinds[m.group(1)] = 'data' if nxt.startswith(DATA_DIRECTIVES) else 'code'

    called = set(re.findall(r'\b(?:jsr|jmp)\s+([A-Za-z_]\w*)', source))
    called |= set(re.findall(r'\.addr\s+([A-Za-z_]\w*)', source))
    called |= {'reset', 'nmi', 'irq'}

    def keep(name):
        if name.startswith('@'):
            return False
        if re.fullmatch(r'L[0-9A-F]{4}', name):
            return name in called
        return True

    syms = sorted(set(s for s in syms if keep(s[1])))
    rows, ndata = [], 0
    for k, (addr, name, seg) in enumerate(syms):
        start, size, ooffs = segs[seg]
        end = next((a for a, _, s in syms[k + 1:] if s == seg and a > addr), start + size)
        if kinds.get(name, 'data') != 'code':
            ndata += 1
            continue
        offset = ooffs - 16 + (addr - start)  # PRG offset: file offset minus the iNES header
        rows.append(f'{name},PRG,{offset:X},{end - addr:X},{addr:X}')
    with open(out, 'w') as fh:
        fh.write('\n'.join(rows) + '\n')
    code = sum(int(r.split(',')[3], 16) for r in rows)
    print(f'{len(rows)} routines ({code} bytes), {ndata} data labels', file=sys.stderr)


if __name__ == '__main__':
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
