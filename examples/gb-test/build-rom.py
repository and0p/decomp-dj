#!/usr/bin/env python3
"""Builds the Game Boy test ROM (build/gb-test.gbc) and its symbol map (symbols/gb-test.csv)
from src/gb-test.asm, with no Game Boy toolchain: this file is a small SM83 assembler for
the subset of RGBDS syntax the source uses.

Usage: ./build-rom.py [--mapper mbc1|mbc3|mbc5] [--dmg] [--out FILE]

The project's ROM is the default (MBC5 with battery RAM, Game Boy Color compatible). The
options make variants of the same program for trying the connector by hand: the code
switches banks by writing to $2100, which MBC1, MBC3 and MBC5 all take.

Source syntax, one statement a line, `;` comments:
  bank N            start ROM bank N (bank 0 at $0000, the others at $4000)
  org $XXXX         go on at this CPU address in the current bank (padding with $FF)
  load $XXXX / endload
                    assemble the lines between for this CPU address, stored where they are
                    (code that the program copies to HRAM)
  proc NAME / endproc
                    a function: a label, and a line in the symbol map with its size
  NAME:  .local:    labels; a `.local` label belongs to the last global one
  NAME equ EXPR     a constant
  db EXPR, ...      bytes;  dw EXPR, ...  little-endian words
  any SM83 instruction, as RGBDS writes it (`ld a, [hl+]`, `ldh [$FF44], a`, `jr nz, .loop`)
Numbers: $hex, %binary or decimal; expressions may add and subtract labels and numbers.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

R8 = ["b", "c", "d", "e", "h", "l", "[hl]", "a"]
R16 = ["bc", "de", "hl", "sp"]
R16_STACK = ["bc", "de", "hl", "af"]
R16_MEM = ["[bc]", "[de]", "[hl+]", "[hl-]"]
COND = ["nz", "z", "nc", "c"]
ALU = ["add", "adc", "sub", "sbc", "and", "xor", "or", "cp"]
ROT = ["rlc", "rrc", "rl", "rr", "sla", "sra", "swap", "srl"]


def templates():
    """Every instruction as (mnemonic, operands) -> (opcode bytes, operand kind). Operand
    kinds: n8, n16, e8 (relative to the next instruction), a8 (ldh: $FF00 + n), s8 (sp + n)."""
    t = {}

    def add(text, code, kind=None):
        m, _, ops = text.partition(" ")
        key = (m, tuple(o.strip() for o in ops.split(",")) if ops else ())
        t[key] = (bytes(code), kind)

    add("nop", [0x00])
    add("ld [n16], sp", [0x08], "n16")
    add("stop", [0x10, 0x00])
    add("jr e8", [0x18], "e8")
    for y in range(4):
        add(f"jr {COND[y]}, e8", [0x20 | y << 3], "e8")
    for p in range(4):
        add(f"ld {R16[p]}, n16", [0x01 | p << 4], "n16")
        add(f"add hl, {R16[p]}", [0x09 | p << 4])
        add(f"ld {R16_MEM[p]}, a", [0x02 | p << 4])
        add(f"ld a, {R16_MEM[p]}", [0x0A | p << 4])
        add(f"inc {R16[p]}", [0x03 | p << 4])
        add(f"dec {R16[p]}", [0x0B | p << 4])
        add(f"pop {R16_STACK[p]}", [0xC1 | p << 4])
        add(f"push {R16_STACK[p]}", [0xC5 | p << 4])
    for y in range(8):
        add(f"inc {R8[y]}", [0x04 | y << 3])
        add(f"dec {R8[y]}", [0x05 | y << 3])
        add(f"ld {R8[y]}, n8", [0x06 | y << 3], "n8")
        add(f"{['rlca', 'rrca', 'rla', 'rra', 'daa', 'cpl', 'scf', 'ccf'][y]}", [0x07 | y << 3])
        add(f"{ALU[y]} a, n8", [0xC6 | y << 3], "n8")
        add(f"rst ${y * 8:02X}", [0xC7 | y << 3])
        for z in range(8):
            if not (y == 6 and z == 6):
                add(f"ld {R8[y]}, {R8[z]}", [0x40 | y << 3 | z])
            add(f"{ALU[y]} a, {R8[z]}", [0x80 | y << 3 | z])
            add(f"{ROT[y]} {R8[z]}", [0xCB, y << 3 | z])
            add(f"bit {y}, {R8[z]}", [0xCB, 0x40 | y << 3 | z])
            add(f"res {y}, {R8[z]}", [0xCB, 0x80 | y << 3 | z])
            add(f"set {y}, {R8[z]}", [0xCB, 0xC0 | y << 3 | z])
    add("halt", [0x76])
    for y in range(4):
        add(f"ret {COND[y]}", [0xC0 | y << 3])
        add(f"jp {COND[y]}, n16", [0xC2 | y << 3], "n16")
        add(f"call {COND[y]}, n16", [0xC4 | y << 3], "n16")
    add("ret", [0xC9])
    add("reti", [0xD9])
    add("jp n16", [0xC3], "n16")
    add("jp hl", [0xE9])
    add("call n16", [0xCD], "n16")
    add("ldh [a8], a", [0xE0], "a8")
    add("ldh a, [a8]", [0xF0], "a8")
    add("ldh [c], a", [0xE2])
    add("ldh a, [c]", [0xF2])
    add("ld [n16], a", [0xEA], "n16")
    add("ld a, [n16]", [0xFA], "n16")
    add("add sp, s8", [0xE8], "s8")
    add("ld hl, sp + s8", [0xF8], "s8")
    add("ld sp, hl", [0xF9])
    add("di", [0xF3])
    add("ei", [0xFB])
    return t


TEMPLATES = templates()
FIXED = set(R8 + R16 + R16_STACK + R16_MEM + COND + ["[c]", "hl"]) | {f"${y * 8:02X}" for y in range(8)} | {str(y) for y in range(8)}


class Error(Exception):
    pass


def split_operands(text):
    return [o.strip() for o in text.split(",")] if text.strip() else []


def shape(mnemonic, ops):
    """The template keys an instruction's operands can match: a register, condition or
    bit number stands for itself; anything else is an expression."""
    choices = []
    for o in ops:
        low = o.lower().replace(" ", "")
        if low in FIXED:
            alts = [low]
            # `rst $38` and `bit 7, a` are fixed numbers, but `ld a, 7` is a number too.
            if not low.startswith("[") and low not in R8 + R16 + R16_STACK + COND:
                alts.append(None)
            choices.append(alts)
        elif low.startswith("[") and low.endswith("]"):
            choices.append(["[n16]", "[a8]"])
        elif low.startswith("sp+") or low.startswith("sp-"):
            choices.append(["sp + s8"])
        else:
            choices.append([None])
    keys = [[]]
    for alts in choices:
        keys = [k + [a] for k in keys for a in alts]
    out = []
    for k in keys:
        for (m, tops), val in TEMPLATES.items():
            if m != mnemonic or len(tops) != len(k):
                continue
            if all(a == b or (a is None and b in ("n8", "n16", "e8", "s8")) for a, b in zip(k, tops)):
                out.append(((m, tops), val))
    # The fixed spelling first (`rst $38`, `bit 7, a`), then the expression ones.
    out.sort(key=lambda kv: sum(1 for o in kv[0][1] if o in ("n8", "n16", "e8", "s8", "[n16]", "[a8]")))
    return out


class Asm:
    def __init__(self, rom_banks):
        self.rom = bytearray([0xFF] * (rom_banks * 0x4000))
        self.labels = {}
        self.consts = {}
        self.procs = []

    def value(self, expr, scope):
        e = expr.strip()
        e = re.sub(r"(?<![\w.])\.(\w+)", lambda m: scope + "." + m.group(1), e)

        def num(tok):
            if tok.startswith("$"):
                return int(tok[1:], 16)
            if tok.startswith("%"):
                return int(tok[1:], 2)
            if tok.isdigit():
                return int(tok)
            if tok in self.consts:
                return self.consts[tok]
            if tok in self.labels:
                return self.labels[tok][0]
            if self.pass_no == 1:
                return 0
            raise Error(f"unknown name {tok}")

        total, sign = 0, 1
        for tok in re.findall(r"[+-]|[^+\s-]+", e):
            if tok == "+":
                sign = 1
            elif tok == "-":
                sign = -1
            else:
                total += sign * num(tok)
                sign = 1
        return total

    def run(self, lines):
        for self.pass_no in (1, 2):
            self.procs = []
            bank, offset, cpu = 0, 0, 0
            scope, proc = "", None
            for n, raw in enumerate(lines, 1):
                line = raw.split(";", 1)[0].strip()
                if not line:
                    continue
                try:
                    m = re.match(r"^([A-Za-z_]\w*|\.\w+):\s*(.*)$", line)
                    if m:
                        name = m.group(1)
                        if name.startswith("."):
                            name = scope + name
                        else:
                            scope = name
                        self.labels[name] = (cpu, offset)
                        line = m.group(2)
                        if not line:
                            continue
                    word, _, rest = line.partition(" ")
                    word = word.lower()
                    m = re.match(r"^(\w+)\s+equ\s+(.+)$", line, re.I)
                    if m:
                        self.consts[m.group(1)] = self.value(m.group(2), scope)
                    elif word == "bank":
                        bank = self.value(rest, scope)
                        offset = bank * 0x4000
                        cpu = 0 if bank == 0 else 0x4000
                    elif word == "org":
                        a = self.value(rest, scope)
                        base = 0 if bank == 0 else 0x4000
                        if a < cpu or a >= base + 0x4000:
                            raise Error(f"org ${a:04X} is behind ${cpu:04X} or outside the bank")
                        offset += a - cpu
                        cpu = a
                    elif word == "load":
                        cpu = self.value(rest, scope)
                    elif word == "endload":
                        # The CPU address follows the bytes in ROM again.
                        cpu = offset - bank * 0x4000 + (0 if bank == 0 else 0x4000)
                    elif word == "proc":
                        scope = rest.strip()
                        self.labels[scope] = (cpu, offset)
                        proc = (scope, offset, cpu)
                    elif word == "endproc":
                        name, start, start_cpu = proc
                        self.procs.append((name, start, offset - start, start_cpu))
                        proc = None
                    elif word in ("db", "dw"):
                        for o in split_operands(rest):
                            v = self.value(o, scope)
                            data = [v & 0xFF] if word == "db" else [v & 0xFF, v >> 8 & 0xFF]
                            self.rom[offset:offset + len(data)] = bytes(data)
                            offset += len(data)
                            cpu += len(data)
                    else:
                        code = self.instruction(word, rest, cpu, scope)
                        self.rom[offset:offset + len(code)] = code
                        offset += len(code)
                        cpu += len(code)
                except Error as e:
                    raise Error(f"line {n}: {e}: {raw.strip()}")

    def instruction(self, mnemonic, rest, cpu, scope):
        ops = split_operands(rest)
        if mnemonic in ALU and len(ops) == 1:
            ops = ["a"] + ops  # `xor a` for `xor a, a`, as RGBDS takes it
        matches = shape(mnemonic, ops)
        if not matches:
            raise Error("not an instruction")
        (m, tops), (code, kind) = matches[0]
        if kind is None:
            return code
        # The operand that is an expression.
        i = next(i for i, t in enumerate(tops) if t in ("n8", "n16", "e8", "s8", "[n16]", "[a8]", "sp + s8"))
        expr = ops[i]
        low = expr.replace(" ", "").lower()
        if low.startswith("[") and low.endswith("]"):
            expr = expr.strip()[1:-1]
        if low.startswith("sp+"):
            expr = expr.split("+", 1)[1]
        elif low.startswith("sp-"):
            expr = "-" + expr.split("-", 1)[1]
        v = self.value(expr, scope)
        if kind == "n8":
            if self.pass_no == 2 and not -128 <= v <= 255:
                raise Error(f"{v} doesn't fit a byte")
            return code + bytes([v & 0xFF])
        if kind == "n16":
            return code + bytes([v & 0xFF, v >> 8 & 0xFF])
        if kind == "a8":
            if self.pass_no == 2 and not 0xFF00 <= v <= 0xFFFF:
                raise Error(f"ldh needs $FF00-$FFFF, not ${v:04X}")
            return code + bytes([v & 0xFF])
        if kind == "s8":
            return code + bytes([v & 0xFF])
        # e8: relative to the next instruction.
        d = v - (cpu + 2)
        if self.pass_no == 2 and not -128 <= d <= 127:
            raise Error(f"jr target ${v:04X} is out of reach")
        return code + bytes([d & 0xFF])


# The logo every cartridge header carries at $0104; the boot ROM checks it.
LOGO = bytes.fromhex(
    "CEED6666CC0D000B03730083000C000D0008111F8889000EDCCC6EE6DDDDD999BBBB67636E0EECCCDDDC999FBBB9333E"
)
CART_TYPE = {"mbc1": 0x03, "mbc3": 0x13, "mbc5": 0x1B}  # each with RAM and a battery


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--mapper", choices=sorted(CART_TYPE), default="mbc5")
    ap.add_argument("--dmg", action="store_true", help="a Game Boy game (no Color flag)")
    ap.add_argument("--out", help="the ROM file (default build/gb-test.gbc)")
    args = ap.parse_args()
    default = args.mapper == "mbc5" and not args.dmg and not args.out

    with open(os.path.join(HERE, "src", "gb-test.asm")) as f:
        lines = f.read().splitlines()
    asm = Asm(rom_banks=4)
    try:
        asm.run(lines)
    except Error as e:
        sys.exit(f"src/gb-test.asm: {e}")
    rom = asm.rom
    rom[0x104:0x134] = LOGO
    title = b"DJ GB TEST"
    rom[0x134:0x143] = title.ljust(15, b"\0")
    rom[0x143] = 0x00 if args.dmg else 0x80  # works on both, with Color features on a Color
    rom[0x144:0x146] = b"\0\0"
    rom[0x146] = 0x00
    rom[0x147] = CART_TYPE[args.mapper]
    rom[0x148] = 0x01  # 64 KB: 4 banks
    rom[0x149] = 0x02  # 8 KB of RAM
    rom[0x14A] = 0x01
    rom[0x14B] = 0x33
    rom[0x14C] = 0x00
    check = 0
    for b in rom[0x134:0x14D]:
        check = (check - b - 1) & 0xFF
    rom[0x14D] = check
    rom[0x14E:0x150] = b"\0\0"
    total = sum(rom) & 0xFFFF
    rom[0x14E:0x150] = bytes([total >> 8, total & 0xFF])

    out = args.out or os.path.join(HERE, "build", "gb-test.gbc")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "wb") as f:
        f.write(rom)
    print(f"built {out}")
    if default:
        csv = os.path.join(HERE, "symbols", "gb-test.csv")
        os.makedirs(os.path.dirname(csv), exist_ok=True)
        with open(csv, "w") as f:
            f.write("name,region,offset,size,cpu_addr\n")
            for name, off, size, cpu in sorted(asm.procs, key=lambda p: p[1]):
                f.write(f"{name},ROM,{off:X},{size:X},{cpu:X}\n")
        print(f"wrote {csv}")


if __name__ == "__main__":
    main()
