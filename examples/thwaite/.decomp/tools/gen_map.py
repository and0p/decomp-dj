#!/usr/bin/env python3
"""Generates src/data/map.c and src/data/breakpoints.tsv from Thwaite's source and its
ca65 debug file.

Run by hand when the pinned Thwaite commit changes (examples/thwaite/build-rom.sh builds
both inputs). The output is committed, so the core builds without this script.

    python3 tools/gen_map.py ../build/thwaite-nes ../build/thwaite.dbg

What it writes:

- every RAM symbol (zero page and BSS labels, and the fixed buffers), with its address and
  size, for copying, comparing and naming diffs;
- the breakpoints: each wait loop's exit (the first instruction after the `beq` that spins
  on `nmis`), each `lda nmis` that game logic reads, and the entries of the routines that
  read the controllers, as PRG offsets;
- for each wait loop, the PC range of its spin, so a jump can tell the original is waiting.
"""
import os
import re
import sys

# Waits in these files are boot-time timing loops, which the decomp doesn't model.
SKIP_FILES = {"paldetect.s"}

# Where the original has just read the controllers' raw bits: (file, proc, instruction,
# where). The decomp takes the raw bytes from the original's RAM there and runs its own
# version of the rest. Raw bits depend on what the emulator puts in each port (a mouse, a
# joypad read past its 8 bits), which is not game logic.
#   read_pads: after `jsr read_pads_once`, $00 and $01 hold ports 1 and 2.
#   read_mouse: at its `rts`, $01-$03 hold the report and X the port.
INPUT_POINTS = [
    ("pads.s", "read_pads", "jsr read_pads_once", "after", "input:read_pads"),
    ("mouse.s", "read_mouse", "rts", "at", "input:read_mouse"),
    # Not input, but from outside the game logic all the same: after splitBalloon starts the
    # split sound, it reloads X from $02, where pently_start_sound left the channel (0 or 4,
    # depending on the sound engine's state).
    ("missiles.s", "splitBalloon", "jsr pently_start_sound", "after", "sound:splitBalloon"),
]

# Fixed buffers defined as equates (global.inc), not labels.
FIXED = [("xferBuf", 0x100, 0x80), ("dte_output_buf", 0x180, 0x20), ("OAM", 0x200, 0x100)]


def parse_dbg(path):
    recs = {"file": {}, "seg": {}, "span": {}, "line": [], "sym": []}
    for raw in open(path, encoding="utf-8"):
        kind, _, rest = raw.rstrip("\n").partition("\t")
        if kind not in recs:
            continue
        f = {}
        for m in re.finditer(r'(\w+)=("([^"]*)"|[^,]*)', rest):
            f[m.group(1)] = m.group(3) if m.group(3) is not None else m.group(2)
        if kind in ("line", "sym"):
            recs[kind].append(f)
        else:
            recs[kind][int(f["id"])] = f
    return recs


def num(s):
    return int(s, 16) if s.startswith("0x") else int(s)


class Dbg:
    def __init__(self, path):
        r = parse_dbg(path)
        self.files = {int(i): f["name"] for i, f in r["file"].items()}
        self.segs = r["seg"]
        self.spans = r["span"]
        # (file name, line) -> PRG offsets of the code on that line
        self.lines = {}
        for l in r["line"]:
            if "span" not in l:
                continue
            name = os.path.basename(self.files[int(l["file"])])
            for sid in l["span"].split("+"):
                off = self.prg_offset(int(sid))
                if off is not None:
                    self.lines.setdefault((name, int(l["line"])), []).append(off)
        self.syms = r["sym"]

    def prg_offset(self, span_id):
        sp = self.spans[span_id]
        seg = self.segs[int(sp["seg"])]
        if seg.get("type") != "ro" or "ooffs" not in seg or seg["name"] in ("INESHDR", "CHR"):
            return None
        return num(seg["ooffs"]) - 16 + num(sp["start"])

    def label_offset(self, name):
        for s in self.syms:
            if s.get("name") == name and s.get("type") == "lab" and "seg" in s:
                seg = self.segs[int(s["seg"])]
                if seg.get("type") == "ro" and "ooffs" in seg:
                    return num(seg["ooffs"]) - 16 + num(s["val"]) - num(seg["start"])
        raise SystemExit(f"no code label {name} in the debug file")

    def line_offset(self, name, line):
        offs = self.lines.get((name, line))
        return min(offs) if offs else None

    def ram_symbols(self):
        rw = {int(i) for i, s in self.segs.items() if s.get("type") == "rw"}
        out = {}
        for s in self.syms:
            if s.get("type") != "lab" or "seg" not in s or int(s["seg"]) not in rw:
                continue
            addr = num(s["val"])
            size = int(s.get("size", "1"))
            key = (s["name"], addr)
            out[key] = max(size, out.get(key, 0))
        for name, addr, size in FIXED:
            out[(name, addr)] = size
        return sorted(((a, n, sz) for (n, a), sz in out.items()), key=lambda t: (t[0], t[1]))


INSN = re.compile(r"^\s*(?:[@\w]*:\s*)?([a-z]{3})\b\s*([^;]*)", re.I)
LABEL_ONLY = re.compile(r"^\s*[@\w]*:\s*(;.*)?$")
PROC = re.compile(r"^\s*\.proc\s+(\w+)", re.I)
ENDPROC = re.compile(r"^\s*\.endproc", re.I)


def instructions(lines):
    """(line number, mnemonic, operand, enclosing proc) for each instruction line."""
    procs = []
    for i, text in enumerate(lines, 1):
        m = PROC.match(text)
        if m:
            procs.append(m.group(1))
            continue
        if ENDPROC.match(text):
            if procs:
                procs.pop()
            continue
        if LABEL_ONLY.match(text) or text.strip().startswith(";") or text.strip().startswith("."):
            continue
        m = INSN.match(text)
        if m:
            yield i, m.group(1).lower(), m.group(2).strip(), procs[-1] if procs else "(none)"


def find_breakpoints(src_dir, dbg):
    waits, reads, skipped = [], [], []
    for name in sorted(os.listdir(src_dir)):
        if not name.endswith(".s"):
            continue
        ins = list(instructions(open(os.path.join(src_dir, name), encoding="utf-8").read().split("\n")))
        for k, (line, op, arg, proc) in enumerate(ins):
            if op != "lda" or arg != "nmis":
                continue
            nxt = ins[k + 1 : k + 3]
            is_wait = len(nxt) == 2 and nxt[0][1:3] == ("cmp", "nmis") and nxt[1][1] == "beq"
            if name in SKIP_FILES:
                skipped.append((name, line, proc))
                continue
            lda = dbg.line_offset(name, line)
            if lda is None:
                raise SystemExit(f"{name}:{line}: no code address in the debug file")
            if is_wait:
                exit_line = ins[k + 3][0]
                exit_off = dbg.line_offset(name, exit_line)
                waits.append({"proc": proc, "file": name, "line": exit_line, "offset": exit_off,
                              "spin_lo": lda, "spin_hi": exit_off})
            else:
                reads.append({"proc": proc, "file": name, "line": line, "offset": lda})
    return waits, reads, skipped


def find_input_points(src_dir, dbg):
    out = []
    for fname, proc, text, where, name in INPUT_POINTS:
        ins = list(instructions(open(os.path.join(src_dir, fname), encoding="utf-8").read().split("\n")))
        for k, (line, op, arg, p) in enumerate(ins):
            if p == proc and f"{op} {arg}".strip() == text:
                at = ins[k + 1][0] if where == "after" else line
                out.append({"name": name, "offset": dbg.line_offset(fname, at), "file": fname, "line": at})
                break
        else:
            raise SystemExit(f"{fname}: no `{text}` in {proc}")
    return out


def name_all(items, prefix):
    counts = {}
    for it in items:
        counts[it["proc"]] = counts.get(it["proc"], 0) + 1
    seen = {}
    for it in items:
        p = it["proc"]
        seen[p] = seen.get(p, 0) + 1
        it["name"] = f"{prefix}{p}" + (f".{seen[p]}" if counts[p] > 1 else "")


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    thwaite, dbg_path = sys.argv[1], sys.argv[2]
    here = os.path.dirname(os.path.abspath(__file__))
    out_dir = os.path.join(here, "..", "src", "data")
    os.makedirs(out_dir, exist_ok=True)

    dbg = Dbg(dbg_path)
    waits, reads, skipped = find_breakpoints(os.path.join(thwaite, "src"), dbg)
    name_all(waits, "wait:")
    name_all(reads, "nmis:")
    inputs = find_input_points(os.path.join(thwaite, "src"), dbg)
    syms = dbg.ram_symbols()

    c = ["/* Generated by tools/gen_map.py from Thwaite's source and thwaite.dbg. Do not edit. */",
         '#include "../map.h"', "",
         "const struct thw_ramsym thw_ramsyms[] = {"]
    for addr, name, size in syms:
        c.append(f'    {{"{name}", 0x{addr:03X}, {size}}},')
    c += ["};", f"const unsigned thw_ramsym_count = {len(syms)};", "",
          "const struct thw_bp thw_bps[] = {"]
    bps = [(w, "THW_BP_WAIT") for w in waits] + [(r, "THW_BP_NMIS_READ") for r in reads] + \
        [(i, "THW_BP_INPUT") for i in inputs]
    tsv = ["# id\tkind\tname\tprg\tspin_lo\tspin_hi\tsource"]
    for i, (b, kind) in enumerate(bps):
        lo, hi = b.get("spin_lo", 0), b.get("spin_hi", 0)
        c.append(f'    {{{i}, {kind}, "{b["name"]}", 0x{b["offset"]:04X}, 0x{lo:04X}, 0x{hi:04X}}}, /* {b["file"]}:{b["line"]} */')
        short = {"THW_BP_WAIT": "wait", "THW_BP_NMIS_READ": "nmis", "THW_BP_INPUT": "input"}[kind]
        tsv.append(f'{i}\t{short}\t{b["name"]}\tPRG:0x{b["offset"]:X}\t0x{lo:X}\t0x{hi:X}\t{b["file"]}:{b["line"]}')
    c += ["};", f"const unsigned thw_bp_count = {len(bps)};", ""]
    open(os.path.join(out_dir, "map.c"), "w").write("\n".join(c))
    open(os.path.join(out_dir, "breakpoints.tsv"), "w").write("\n".join(tsv) + "\n")

    print(f"{len(syms)} RAM symbols, {len(waits)} wait exits, {len(reads)} counter reads, {len(inputs)} input routines")
    for b, _ in bps:
        print(f'  {b["name"]:28} PRG:0x{b["offset"]:04X}  {b["file"]}:{b["line"]}')
    for name, line, proc in skipped:
        print(f"  skipped (boot timing): {name}:{line} in {proc}")


if __name__ == "__main__":
    main()
