#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# A small two-pass 68000 assembler for the Genesis test ROM (examples/genesis-test), so the
# example needs no Mega Drive toolchain: Python 3's standard library only.
#
# Usage: asm68k.py SOURCE ROM SYMBOLS [SIZE]
#   writes the ROM (padded with 0xFF to SIZE bytes, default 0x10000) and a symbol map in
#   dj's csv format: one line per `.func NAME` ... `.endfunc` block.
#
# Syntax (Motorola style, as in src/main.s):
#   label:            a global label; `.name:` is local to the last global label
#   op.s  src,dst     sizes .b .w .l, and .s or .w on branches (default .w)
#   ; comment
#   .org EXPR  .align N  .equ NAME,EXPR  .dc.b/.dc.w/.dc.l ITEMS  .ds.b N  .func NAME  .endfunc
# Operands: d0-d7 a0-a7 sp, (an) (an)+ -(an) d(an) d(an,xn.w|l) d(pc) d(pc,xn.w|l)
#   #imm  abs (long) or abs.w (word)  sr ccr  and movem register lists (d0-d3/a0-a2).
# Instructions: only those the test ROM needs (see ENCODERS); anything else is an error.
import re
import sys

SIZES = {"b": 0, "w": 1, "l": 2}
COND = {"t": 0, "f": 1, "hi": 2, "ls": 3, "cc": 4, "hs": 4, "cs": 5, "lo": 5, "ne": 6, "eq": 7,
        "vc": 8, "vs": 9, "pl": 10, "mi": 11, "ge": 12, "lt": 13, "gt": 14, "le": 15}


class AsmError(Exception):
    pass


# ---- expressions -------------------------------------------------------------------------

TOKEN = re.compile(r"\s*(\$[0-9a-fA-F]+|0x[0-9a-fA-F]+|%[01]+|\d+|'[^']*'|[A-Za-z_.][\w.]*|<<|>>|[-+*/()&|~])")


def tokens(text):
    pos, out = 0, []
    text = text.strip()
    while pos < len(text):
        m = TOKEN.match(text, pos)
        if not m:
            raise AsmError(f"can't read expression {text!r}")
        out.append(m.group(1))
        pos = m.end()
    return out


def evaluate(text, symbols, scope, final):
    toks = tokens(text)
    i = 0

    def atom():
        nonlocal i
        t = toks[i]
        i += 1
        if t == "(":
            v = expr()
            i += 1
            return v
        if t == "-":
            return -atom()
        if t == "~":
            return ~atom()
        if t.startswith("$"):
            return int(t[1:], 16)
        if t.startswith("0x"):
            return int(t, 16)
        if t.startswith("%"):
            return int(t[1:], 2)
        if t[0].isdigit():
            return int(t)
        if t.startswith("'"):
            v = 0
            for ch in t[1:-1]:
                v = v << 8 | ord(ch)
            return v
        name = scope + t if t.startswith(".") else t
        if name in symbols:
            return symbols[name]
        if final:
            raise AsmError(f"unknown symbol {t}")
        return 0

    def expr():
        nonlocal i
        v = atom()
        while i < len(toks) and toks[i] in ("+", "-", "*", "/", "<<", ">>", "&", "|"):
            o = toks[i]
            i += 1
            r = atom()
            v = {"+": v + r, "-": v - r, "*": v * r, "/": v // r if r else 0, "<<": v << r, ">>": v >> r, "&": v & r, "|": v | r}[o]
        return v

    v = expr()
    if i != len(toks):
        raise AsmError(f"can't read expression {text!r}")
    return v


# ---- operands ----------------------------------------------------------------------------

def reg_num(name):
    name = name.lower()
    if name == "sp":
        return ("a", 7)
    m = re.fullmatch(r"([da])([0-7])", name)
    return (m.group(1), int(m.group(2))) if m else None


def split_operands(text):
    parts, depth, cur = [], 0, ""
    for ch in text:
        if ch == "," and depth == 0:
            parts.append(cur.strip())
            cur = ""
            continue
        depth += ch == "("
        depth -= ch == ")"
        cur += ch
    if cur.strip():
        parts.append(cur.strip())
    return parts


class Operand:
    """An addressing mode: kind is one of dn an ind postinc predec disp index absw absl
    pcdisp pcindex imm sr ccr list."""

    def __init__(self, kind, reg=0, expr=None, xreg=None, mask=0):
        self.kind, self.reg, self.expr, self.xreg, self.mask = kind, reg, expr, xreg, mask


def parse_operand(text):
    t = text.strip()
    low = t.lower()
    if low in ("sr", "ccr"):
        return Operand(low)
    r = reg_num(t)
    if r:
        return Operand("dn" if r[0] == "d" else "an", r[1])
    if t.startswith("#"):
        return Operand("imm", expr=t[1:])
    if re.fullmatch(r"([da][0-7]|sp)(-([da][0-7]|sp))?(/([da][0-7]|sp)(-([da][0-7]|sp))?)*", low) and ("/" in low or "-" in low):
        mask = 0
        for part in low.split("/"):
            ends = part.split("-")
            a, b = reg_num(ends[0]), reg_num(ends[-1])
            lo, hi = (a[0] == "a") * 8 + a[1], (b[0] == "a") * 8 + b[1]
            for n in range(lo, hi + 1):
                mask |= 1 << n
        return Operand("list", mask=mask)
    m = re.fullmatch(r"\((\w+)\)(\+?)", low)
    if m and reg_num(m.group(1)) and reg_num(m.group(1))[0] == "a":
        return Operand("postinc" if m.group(2) else "ind", reg_num(m.group(1))[1])
    m = re.fullmatch(r"-\((\w+)\)", low)
    if m and reg_num(m.group(1)):
        return Operand("predec", reg_num(m.group(1))[1])
    m = re.fullmatch(r"(.*)\((\w+)(?:,(\w+)(?:\.([wl]))?)?\)", t)
    if m:
        base = m.group(2).lower()
        disp = m.group(1).strip() or "0"
        x = None
        if m.group(3):
            xr = reg_num(m.group(3))
            if not xr:
                raise AsmError(f"bad index register in {text}")
            x = (xr, m.group(4) or "w")
        if base == "pc":
            return Operand("pcindex" if x else "pcdisp", expr=disp, xreg=x)
        br = reg_num(base)
        if br and br[0] == "a":
            return Operand("index" if x else "disp", br[1], expr=disp, xreg=x)
    if low.endswith(".w"):
        return Operand("absw", expr=t[:-2])
    if low.endswith(".l"):
        return Operand("absl", expr=t[:-2])
    return Operand("absl", expr=t)


# ---- the assembler -----------------------------------------------------------------------

class Assembler:
    def __init__(self):
        self.symbols = {}
        self.funcs = []

    def ea(self, op, size, ext_at, final):
        """(mode, reg, extension bytes) for an operand whose extension words start at
        address ext_at."""
        ev = lambda e: evaluate(e, self.symbols, self.scope, final)

        def index_word(x, disp):
            (kind, n), sz = x
            if final and not -128 <= disp <= 127:
                raise AsmError(f"index displacement {disp} out of range")
            return ((kind == "a") << 15 | n << 12 | (sz == "l") << 11 | (disp & 0xFF)).to_bytes(2, "big")

        k = op.kind
        if k == "dn":
            return 0, op.reg, b""
        if k == "an":
            return 1, op.reg, b""
        if k == "ind":
            return 2, op.reg, b""
        if k == "postinc":
            return 3, op.reg, b""
        if k == "predec":
            return 4, op.reg, b""
        if k == "disp":
            return 5, op.reg, (ev(op.expr) & 0xFFFF).to_bytes(2, "big")
        if k == "index":
            return 6, op.reg, index_word(op.xreg, ev(op.expr))
        if k == "absw":
            return 7, 0, (ev(op.expr) & 0xFFFF).to_bytes(2, "big")
        if k == "absl":
            return 7, 1, (ev(op.expr) & 0xFFFFFFFF).to_bytes(4, "big")
        if k == "pcdisp":
            return 7, 2, ((ev(op.expr) - ext_at) & 0xFFFF).to_bytes(2, "big")
        if k == "pcindex":
            return 7, 3, index_word(op.xreg, ev(op.expr) - ext_at)
        if k == "imm":
            v = ev(op.expr)
            return 7, 4, (v & 0xFFFFFFFF).to_bytes(4, "big") if size == 2 else (v & 0xFFFF).to_bytes(2, "big") if size == 1 else (v & 0xFF).to_bytes(2, "big")
        raise AsmError(f"operand {k} not allowed here")

    def instruction(self, mnem, size, ops, pc, final):
        ev = lambda e: evaluate(e, self.symbols, self.scope, final)
        w = lambda v: (v & 0xFFFF).to_bytes(2, "big")
        sz = SIZES.get(size, 1)

        def with_ea(opcode, op, s=sz, pre=b""):
            mode, reg, ext = self.ea(op, s, pc + 2 + len(pre), final)
            return w(opcode | mode << 3 | reg) + pre + ext

        if mnem in ("move", "movea"):
            src, dst = ops
            if dst.kind == "sr":
                return with_ea(0x46C0, src, 1)
            code = {0: 0x1000, 1: 0x3000, 2: 0x2000}[sz]
            smode, sreg, sext = self.ea(src, sz, pc + 2, final)
            dmode, dreg, dext = self.ea(dst, sz, pc + 2 + len(sext), final)
            return w(code | dreg << 9 | dmode << 6 | smode << 3 | sreg) + sext + dext
        if mnem == "moveq":
            return w(0x7000 | ops[1].reg << 9 | (ev(ops[0].expr) & 0xFF))
        if mnem == "lea":
            return with_ea(0x41C0 | ops[1].reg << 9, ops[0], 2)
        if mnem == "pea":
            return with_ea(0x4840, ops[0], 2)
        if mnem in ("clr", "tst", "not", "neg"):
            base = {"clr": 0x4200, "tst": 0x4A00, "not": 0x4600, "neg": 0x4400}[mnem]
            return with_ea(base | sz << 6, ops[0])
        if mnem in ("addq", "subq"):
            q = ev(ops[0].expr) & 7
            return with_ea((0x5000 if mnem == "addq" else 0x5100) | q << 9 | sz << 6, ops[1])
        if mnem in ("addi", "subi", "andi", "ori", "eori", "cmpi"):
            base = {"ori": 0x0000, "andi": 0x0200, "subi": 0x0400, "addi": 0x0600, "eori": 0x0A00, "cmpi": 0x0C00}[mnem]
            imm = self.ea(ops[0], sz, pc + 2, final)[2]
            return with_ea(base | sz << 6, ops[1], sz, imm)
        if mnem in ("add", "sub", "and", "or", "cmp", "eor"):
            line = {"or": 0x8000, "sub": 0x9000, "cmp": 0xB000, "eor": 0xB000, "and": 0xC000, "add": 0xD000}[mnem]
            src, dst = ops
            if dst.kind == "an":  # adda, suba, cmpa
                return with_ea(line | dst.reg << 9 | (7 if sz == 2 else 3) << 6, src, sz)
            if dst.kind == "dn" and mnem != "eor":
                return with_ea(line | dst.reg << 9 | sz << 6, src)
            return with_ea(line | src.reg << 9 | (4 + sz) << 6, dst)
        if mnem in ("mulu", "muls", "divu", "divs"):
            base = {"mulu": 0xC0C0, "muls": 0xC1C0, "divu": 0x80C0, "divs": 0x81C0}[mnem]
            return with_ea(base | ops[1].reg << 9, ops[0], 1)
        if mnem in ("lsl", "lsr", "asl", "asr", "rol", "ror"):
            kind = {"as": 0, "ls": 1, "ro": 3}[mnem[:2]]
            left = mnem.endswith("l")
            if len(ops) == 1:  # a word in memory, by one
                return with_ea(0xE0C0 | kind << 9 | left << 8, ops[0], 1)
            cnt, dst = ops
            if cnt.kind == "imm":
                n = ev(cnt.expr) & 7
                return w(0xE000 | n << 9 | left << 8 | sz << 6 | kind << 3 | dst.reg)
            return w(0xE020 | cnt.reg << 9 | left << 8 | sz << 6 | kind << 3 | dst.reg)
        if mnem in ("btst", "bchg", "bclr", "bset"):
            t = ("btst", "bchg", "bclr", "bset").index(mnem)
            bit, dst = ops
            if bit.kind == "imm":
                return with_ea(0x0800 | t << 6, dst, 0, w(ev(bit.expr)))
            return with_ea(0x0100 | bit.reg << 9 | t << 6, dst, 0)
        if mnem == "swap":
            return w(0x4840 | ops[0].reg)
        if mnem == "ext":
            return w((0x48C0 if sz == 2 else 0x4880) | ops[0].reg)
        if mnem == "exg":
            return w(0xC140 | ops[0].reg << 9 | ops[1].reg)
        if mnem == "movem":
            src, dst = ops
            to_mem = src.kind in ("list", "dn", "an")
            mask = src.mask if src.kind == "list" else 1 << (src.reg + 8 * (src.kind == "an")) if to_mem else dst.mask if dst.kind == "list" else 1 << (dst.reg + 8 * (dst.kind == "an"))
            target = dst if to_mem else src
            if to_mem and target.kind == "predec":
                mask = int(f"{mask:016b}"[::-1], 2)
            return with_ea((0x4880 if to_mem else 0x4C80) | (sz == 2) << 6, target, 1, w(mask))
        if mnem in ("jsr", "jmp"):
            return with_ea(0x4E80 if mnem == "jsr" else 0x4EC0, ops[0], 2)
        if mnem == "link":
            return w(0x4E50 | ops[0].reg) + w(ev(ops[1].expr))
        if mnem == "unlk":
            return w(0x4E58 | ops[0].reg)
        simple = {"nop": 0x4E71, "rts": 0x4E75, "rte": 0x4E73, "rtr": 0x4E77, "illegal": 0x4AFC}
        if mnem in simple:
            return w(simple[mnem])
        if mnem == "trap":
            return w(0x4E40 | ev(ops[0].expr) & 15)
        if mnem == "stop":
            return w(0x4E72) + w(ev(ops[0].expr))
        m = re.fullmatch(r"b(ra|sr|" + "|".join(COND) + ")", mnem)
        if m:
            c = {"ra": 0, "sr": 1}.get(m.group(1), COND.get(m.group(1)))
            target = ev(ops[0].expr)
            disp = target - (pc + 2)
            if size == "s":
                if final and (not -128 <= disp <= 127 or disp == 0):
                    raise AsmError(f"short branch to {ops[0].expr} out of range ({disp})")
                return w(0x6000 | c << 8 | (disp & 0xFF))
            return w(0x6000 | c << 8) + w(disp)
        m = re.fullmatch(r"db(ra|" + "|".join(COND) + ")", mnem)
        if m:
            c = 1 if m.group(1) == "ra" else COND[m.group(1)]
            disp = ev(ops[1].expr) - (pc + 2)
            return w(0x50C8 | c << 8 | ops[0].reg) + w(disp)
        raise AsmError(f"unknown instruction {mnem}")

    def assemble(self, lines, final):
        out = {}
        pc = 0
        self.scope = ""
        func = None
        self.funcs = []
        for n, raw in enumerate(lines, 1):
            line = raw.split(";", 1)[0].rstrip()
            if not line.strip():
                continue
            try:
                m = re.match(r"^([A-Za-z_.][\w.]*):", line.strip())
                if m:
                    name = m.group(1)
                    if not name.startswith("."):
                        self.scope = name
                    else:
                        name = self.scope + name
                    if not final:
                        if name in self.symbols and self.symbols[name] != pc and self.pass1_seen.get(name):
                            raise AsmError(f"{name} defined twice")
                        self.pass1_seen[name] = True
                        self.symbols[name] = pc
                    line = line.strip()[m.end():]
                    if not line.strip():
                        continue
                parts = line.strip().split(None, 1)
                head = parts[0].lower()
                args = parts[1] if len(parts) > 1 else ""
                mnem, _, size = head.partition(".")
                if head.startswith("."):
                    mnem, _, size = head[1:].partition(".")
                    if mnem == "org":
                        pc = evaluate(args, self.symbols, self.scope, final)
                    elif mnem == "align":
                        a = evaluate(args, self.symbols, self.scope, final)
                        while pc % a:
                            out[pc] = 0xFF
                            pc += 1
                    elif mnem == "equ":
                        name, e = args.split(",", 1)
                        self.symbols[name.strip()] = evaluate(e, self.symbols, self.scope, final)
                    elif mnem == "func":
                        name = args.strip()
                        self.scope = name
                        if not final:
                            self.symbols[name] = pc
                        func = (name, pc)
                    elif mnem == "endfunc":
                        self.funcs.append((func[0], func[1], pc - func[1]))
                        func = None
                    elif mnem == "ds":
                        for _ in range(evaluate(args, self.symbols, self.scope, final)):
                            out[pc] = 0
                            pc += 1
                    elif mnem == "dc":
                        width = {"b": 1, "w": 2, "l": 4}[size]
                        for item in split_operands(args):
                            if width == 1 and item.startswith('"'):
                                data = item.strip('"').encode("ascii")
                            else:
                                data = (evaluate(item, self.symbols, self.scope, final) & ((1 << 8 * width) - 1)).to_bytes(width, "big")
                            for b in data:
                                out[pc] = b
                                pc += 1
                    else:
                        raise AsmError(f"unknown directive .{mnem}")
                    continue
                ops = [parse_operand(o) for o in split_operands(args)]
                code = self.instruction(mnem, size, ops, pc, final)
                if pc & 1:
                    raise AsmError("instruction at an odd address")
                for b in code:
                    out[pc] = b
                    pc += 1
            except (AsmError, IndexError, AttributeError, KeyError, TypeError) as e:
                raise SystemExit(f"line {n}: {raw.strip()}: {e}")
        return out


def main():
    if len(sys.argv) not in (4, 5):
        raise SystemExit(__doc__ or "usage: asm68k.py SOURCE ROM SYMBOLS [SIZE]")
    src, rom_path, sym_path = sys.argv[1:4]
    size = int(sys.argv[4], 0) if len(sys.argv) == 5 else 0x10000
    lines = open(src, encoding="utf-8").read().splitlines()
    a = Assembler()
    a.pass1_seen = {}
    a.assemble(lines, final=False)
    a.symbols["ROM_SIZE"] = size
    out = a.assemble(lines, final=True)
    if max(out) >= size:
        raise SystemExit(f"the program ({max(out) + 1} bytes) doesn't fit in {size}")
    rom = bytearray([0xFF] * size)
    for addr, b in out.items():
        rom[addr] = b
    # The header's checksum: the words after the header, as the BIOS of a later model would
    # check it.
    total = sum(int.from_bytes(rom[i:i + 2], "big") for i in range(0x200, size, 2)) & 0xFFFF
    rom[0x18E:0x190] = total.to_bytes(2, "big")
    with open(rom_path, "wb") as f:
        f.write(rom)
    with open(sym_path, "w", encoding="utf-8") as f:
        f.write("name,region,offset,size,cpu_addr\n")
        for name, start, length in sorted(a.funcs, key=lambda x: x[1]):
            f.write(f"{name},ROM,{start:x},{length:x},{start:x}\n")


if __name__ == "__main__":
    main()
