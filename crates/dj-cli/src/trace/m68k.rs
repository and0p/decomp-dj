//! 68000 disassembly for traces (Motorola syntax, lower case): `move.w #$1234,d0`.
//!
//! A trace's execute event carries the first four bytes of an instruction
//! (docs/connectors.md, "Traces"), so an operand in the bytes after them is shown as `…`.

/// `move.w d0,$FF0004` for the instruction at CPU address `pc` whose first bytes are
/// `bytes`. `name` turns a CPU address into a label.
pub fn disasm_m68k(pc: u32, bytes: &[u8], name: &dyn Fn(u32) -> Option<String>) -> String {
    let d = Dis { pc, bytes, name };
    d.instruction().unwrap_or_else(|| match d.word(0) {
        Some(w) => format!("dc.w ${w:04X}"),
        None => "…".into(),
    })
}

const SIZE: [&str; 3] = ["b", "w", "l"];
const COND: [&str; 16] = ["t", "f", "hi", "ls", "cc", "cs", "ne", "eq", "vc", "vs", "pl", "mi", "ge", "lt", "gt", "le"];

struct Dis<'a> {
    pc: u32,
    bytes: &'a [u8],
    name: &'a dyn Fn(u32) -> Option<String>,
}

impl Dis<'_> {
    fn word(&self, at: usize) -> Option<u16> {
        Some(u16::from_be_bytes([*self.bytes.get(at)?, *self.bytes.get(at + 1)?]))
    }

    fn long(&self, at: usize) -> Option<u32> {
        Some((self.word(at)? as u32) << 16 | self.word(at + 2)? as u32)
    }

    fn label(&self, a: u32) -> String {
        let a = a & 0xFF_FFFF;
        (self.name)(a).unwrap_or_else(|| format!("${a:06X}"))
    }

    fn hex(v: i64) -> String {
        if v < 0 { format!("-${:X}", -v) } else { format!("${v:X}") }
    }

    /// An effective address whose extension words start `at` bytes in: its text and how many
    /// bytes they take. Missing bytes show as `…`.
    fn ea(&self, mode: u16, reg: u16, size: usize, at: usize) -> Option<(String, usize)> {
        let unknown = |n: usize, f: &dyn Fn() -> Option<String>| (f().unwrap_or_else(|| "…".into()), n);
        Some(match mode {
            0 => (format!("d{reg}"), 0),
            1 => (if reg == 7 { "sp".into() } else { format!("a{reg}") }, 0),
            2 => (format!("({})", areg(reg)), 0),
            3 => (format!("({})+", areg(reg)), 0),
            4 => (format!("-({})", areg(reg)), 0),
            5 => unknown(2, &|| Some(format!("{}({})", Self::hex(self.word(at)? as i16 as i64), areg(reg)))),
            6 => unknown(2, &|| {
                let x = self.word(at)?;
                Some(format!("{}({},{})", Self::hex(x as u8 as i8 as i64), areg(reg), index(x)))
            }),
            7 => match reg {
                0 => unknown(2, &|| Some(format!("{}.w", self.label(self.word(at)? as i16 as i32 as u32)))),
                1 => unknown(4, &|| Some(self.label(self.long(at)?))),
                2 => unknown(2, &|| Some(format!("{}(pc)", self.label(self.pc.wrapping_add(at as u32).wrapping_add(self.word(at)? as i16 as i32 as u32))))),
                3 => unknown(2, &|| {
                    let x = self.word(at)?;
                    Some(format!("{}(pc,{})", self.label(self.pc.wrapping_add(at as u32).wrapping_add(x as u8 as i8 as i32 as u32)), index(x)))
                }),
                4 if size == 4 => unknown(4, &|| Some(format!("#${:X}", self.long(at)?))),
                4 => unknown(2, &|| Some(format!("#${:X}", if size == 1 { self.word(at)? & 0xFF } else { self.word(at)? }))),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn instruction(&self) -> Option<String> {
        let op = self.word(0)?;
        let (mode, reg) = (op >> 3 & 7, op & 7);
        let ss = (op >> 6 & 3) as usize;
        let dn = op >> 9 & 7;
        let ea = |size: usize, at: usize| self.ea(mode, reg, size, at).map(|e| e.0);
        let sized = |m: &str, s: usize| format!("{m}.{}", SIZE[s]);
        Some(match op >> 12 {
            0x0 => {
                const IMM: [&str; 8] = ["ori", "andi", "subi", "addi", "", "eori", "cmpi", ""];
                match op {
                    0x003C | 0x023C | 0x0A3C => return Some(format!("{} #${:X},ccr", IMM[(op >> 9) as usize], self.word(2)? & 0xFF)),
                    0x007C | 0x027C | 0x0A7C => return Some(format!("{} #${:X},sr", IMM[(op >> 9) as usize], self.word(2)?)),
                    _ => {}
                }
                const BIT: [&str; 4] = ["btst", "bchg", "bclr", "bset"];
                if op & 0x0100 != 0 {
                    if mode == 1 {
                        let d = Self::hex(self.word(2).map(|w| w as i16 as i64)?);
                        return Some(match op >> 7 & 1 {
                            0 => format!("movep.{} {d}(a{reg}),d{dn}", if op & 0x40 != 0 { "l" } else { "w" }),
                            _ => format!("movep.{} d{dn},{d}(a{reg})", if op & 0x40 != 0 { "l" } else { "w" }),
                        });
                    }
                    return Some(format!("{} d{dn},{}", BIT[ss], ea(1, 2)?));
                }
                match dn {
                    4 => format!("{} #{},{}", BIT[ss], self.word(2).map(|w| (w & 0xFF).to_string()).unwrap_or("…".into()), ea(1, 4)?),
                    0..=3 | 5 | 6 if ss != 3 => {
                        let (imm, n) = self.ea(7, 4, 1 << ss, 2)?;
                        format!("{} {imm},{}", sized(IMM[dn as usize], ss), ea(1 << ss, 2 + n)?)
                    }
                    _ => return None,
                }
            }
            0x1..=0x3 => {
                let s = [0, 0, 2, 1][(op >> 12) as usize];
                let (src, n) = self.ea(mode, reg, 1 << s, 2)?;
                let (dmode, dreg) = (op >> 6 & 7, dn);
                if dmode == 7 && dreg > 1 {
                    return None;
                }
                let (dst, _) = self.ea(dmode, dreg, 1 << s, 2 + n)?;
                format!("{} {src},{dst}", sized(if dmode == 1 { "movea" } else { "move" }, s))
            }
            0x4 => return self.line4(op),
            0x5 => {
                let cond = COND[(op >> 8 & 15) as usize];
                match (ss, mode) {
                    (3, 1) => format!("db{} d{reg},{}", if cond == "f" { "ra" } else { cond }, self.word(2).map(|w| self.label(self.pc.wrapping_add(2).wrapping_add(w as i16 as i32 as u32))).unwrap_or("…".into())),
                    (3, _) => format!("s{cond} {}", ea(1, 2)?),
                    _ => format!("{} #{},{}", sized(if op & 0x0100 != 0 { "subq" } else { "addq" }, ss), if dn == 0 { 8 } else { dn }, ea(1 << ss, 2)?),
                }
            }
            0x6 => {
                let m = match op >> 8 & 15 {
                    0 => "bra",
                    1 => "bsr",
                    c => ["", "", "bhi", "bls", "bcc", "bcs", "bne", "beq", "bvc", "bvs", "bpl", "bmi", "bge", "blt", "bgt", "ble"][c as usize],
                };
                match op as u8 {
                    0 => format!("{m}.w {}", self.word(2).map(|w| self.label(self.pc.wrapping_add(2).wrapping_add(w as i16 as i32 as u32))).unwrap_or("…".into())),
                    d => format!("{m}.s {}", self.label(self.pc.wrapping_add(2).wrapping_add(d as i8 as i32 as u32))),
                }
            }
            0x7 if op & 0x0100 == 0 => format!("moveq #{},d{dn}", op as u8 as i8),
            0x8 | 0x9 | 0xB | 0xC | 0xD => {
                let line = op >> 12;
                let opmode = (op >> 6 & 7) as usize;
                let base = match line {
                    0x8 => "or",
                    0x9 => "sub",
                    0xB => "cmp",
                    0xC => "and",
                    _ => "add",
                };
                match opmode {
                    3 | 7 => match line {
                        0x8 => format!("{} {},d{dn}", if opmode == 3 { "divu.w" } else { "divs.w" }, ea(2, 2)?),
                        0xC => format!("{} {},d{dn}", if opmode == 3 { "mulu.w" } else { "muls.w" }, ea(2, 2)?),
                        _ => {
                            let s = if opmode == 3 { 1 } else { 2 };
                            format!("{} {},a{dn}", sized(&format!("{base}a"), s), ea(1 << s, 2)?)
                        }
                    },
                    0..=2 => format!("{} {},d{dn}", sized(base, opmode), ea(1 << opmode, 2)?),
                    _ => {
                        let s = opmode - 4;
                        let rm = op >> 3 & 1 != 0;
                        match (line, opmode, mode) {
                            (0x8 | 0xC, 4, 0 | 1) => {
                                let m = if line == 0x8 { "sbcd" } else { "abcd" };
                                if rm { format!("{m} -(a{reg}),-(a{dn})") } else { format!("{m} d{reg},d{dn}") }
                            }
                            (0xC, 5, 0) => format!("exg d{dn},d{reg}"),
                            (0xC, 5, 1) => format!("exg a{dn},a{reg}"),
                            (0xC, 6, 1) => format!("exg d{dn},a{reg}"),
                            (0x9 | 0xD, _, 0 | 1) => {
                                let m = sized(if line == 0x9 { "subx" } else { "addx" }, s);
                                if rm { format!("{m} -(a{reg}),-(a{dn})") } else { format!("{m} d{reg},d{dn}") }
                            }
                            (0xB, _, 1) => format!("{} (a{reg})+,(a{dn})+", sized("cmpm", s)),
                            (0xB, _, _) => format!("{} d{dn},{}", sized("eor", s), ea(1 << s, 2)?),
                            _ => format!("{} d{dn},{}", sized(base, s), ea(1 << s, 2)?),
                        }
                    }
                }
            }
            0xE => {
                const KIND: [&str; 4] = ["as", "ls", "rox", "ro"];
                let dir = if op & 0x0100 != 0 { "l" } else { "r" };
                if ss == 3 {
                    if op & 0x0800 != 0 {
                        return None;
                    }
                    format!("{}{dir}.w {}", KIND[(op >> 9 & 3) as usize], ea(2, 2)?)
                } else {
                    let count = if op & 0x20 != 0 { format!("d{dn}") } else { format!("#{}", if dn == 0 { 8 } else { dn }) };
                    format!("{} {count},d{reg}", sized(&format!("{}{dir}", KIND[(op >> 3 & 3) as usize]), ss))
                }
            }
            _ => return None, // line A and line F traps, and moveq with bit 8 set
        })
    }

    fn line4(&self, op: u16) -> Option<String> {
        let (mode, reg) = (op >> 3 & 7, op & 7);
        let ss = (op >> 6 & 3) as usize;
        let dn = op >> 9 & 7;
        let ea = |size: usize, at: usize| self.ea(mode, reg, size, at).map(|e| e.0);
        if op & 0x0100 != 0 {
            return match ss {
                2 => Some(format!("chk.w {},d{dn}", ea(2, 2)?)),
                3 => Some(format!("lea {},{}", ea(4, 2)?, areg(dn))),
                _ => None,
            };
        }
        let simple = match op {
            0x4AFC => Some("illegal"),
            0x4E70 => Some("reset"),
            0x4E71 => Some("nop"),
            0x4E73 => Some("rte"),
            0x4E75 => Some("rts"),
            0x4E76 => Some("trapv"),
            0x4E77 => Some("rtr"),
            _ => None,
        };
        if let Some(s) = simple {
            return Some(s.into());
        }
        Some(match op & 0x0FC0 {
            0x00C0 => format!("move.w sr,{}", ea(2, 2)?),
            0x04C0 => format!("move.w {},ccr", ea(2, 2)?),
            0x06C0 => format!("move.w {},sr", ea(2, 2)?),
            0x0800 => format!("nbcd {}", ea(1, 2)?),
            0x0840 if mode == 0 => format!("swap d{reg}"),
            0x0840 if mode == 1 => return None,
            0x0840 => format!("pea {}", ea(4, 2)?),
            0x0880 | 0x08C0 if mode == 0 => format!("ext.{} d{reg}", if ss == 3 { "l" } else { "w" }),
            0x0880 | 0x08C0 | 0x0C80 | 0x0CC0 => {
                let s = if ss == 3 { "l" } else { "w" };
                let list = self.word(2).map(|m| reg_list(m, mode == 4)).unwrap_or("…".into());
                if op & 0x0400 == 0 { format!("movem.{s} {list},{}", ea(4, 4)?) } else { format!("movem.{s} {},{list}", ea(4, 4)?) }
            }
            0x0AC0 => format!("tas {}", ea(1, 2)?),
            0x0E80 => format!("jsr {}", ea(4, 2)?),
            0x0EC0 => format!("jmp {}", ea(4, 2)?),
            0x0E40 => match op & 0x38 {
                0x00 | 0x08 => format!("trap #{}", op & 15),
                0x10 => format!("link a{reg},#{}", self.word(2).map(|w| Self::hex(w as i16 as i64)).unwrap_or("…".into())),
                0x18 => format!("unlk a{reg}"),
                0x20 => format!("move.l a{reg},usp"),
                0x28 => format!("move.l usp,a{reg}"),
                0x30 if op == 0x4E72 => format!("stop #${:X}", self.word(2)?),
                _ => return None,
            },
            _ => match op & 0x0F00 {
                0x0000 | 0x0200 | 0x0400 | 0x0600 | 0x0A00 if ss != 3 => {
                    let m = ["negx", "", "clr", "", "neg", "", "not", "", "", "", "tst"][(op >> 8 & 15) as usize];
                    format!("{m}.{} {}", SIZE[ss], ea(1 << ss, 2)?)
                }
                _ => return None,
            },
        })
    }
}

fn areg(n: u16) -> String {
    if n == 7 { "sp".into() } else { format!("a{n}") }
}

/// The index register of a brief extension word: `d0.w`, `a1.l`.
fn index(x: u16) -> String {
    let r = x >> 12 & 7;
    let r = if x & 0x8000 != 0 { areg(r) } else { format!("d{r}") };
    format!("{r}.{}", if x & 0x0800 != 0 { "l" } else { "w" })
}

/// A MOVEM register mask as `d0-d3/a0`. With `-(An)` the mask runs the other way.
fn reg_list(mask: u16, predec: bool) -> String {
    let mask = if predec { mask.reverse_bits() } else { mask };
    let name = |i: usize| if i < 8 { format!("d{i}") } else { areg(i as u16 - 8) };
    let mut out: Vec<String> = vec![];
    let mut i = 0;
    while i < 16 {
        if mask & 1 << i == 0 {
            i += 1;
            continue;
        }
        let start = i;
        // A run stops at the end of the data or the address registers.
        while i + 1 < 16 && mask & 1 << (i + 1) != 0 && (i + 1) % 8 != 0 {
            i += 1;
        }
        out.push(if i == start { name(start) } else { format!("{}-{}", name(start), name(i)) });
        i += 1;
    }
    if out.is_empty() { "0".into() } else { out.join("/") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dis(words: &[u16]) -> String {
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
        disasm_m68k(0x1000, &bytes, &|a| (a == 0x2000).then(|| "main".into()))
    }

    #[test]
    fn disassembles_m68k() {
        assert_eq!(dis(&[0x4E75]), "rts");
        assert_eq!(dis(&[0x7001]), "moveq #1,d0");
        assert_eq!(dis(&[0x303C, 0x1234]), "move.w #$1234,d0");
        assert_eq!(dis(&[0x33C0, 0x00FF]), "move.w d0,…"); // only four bytes in a trace
        assert_eq!(dis(&[0x33C0, 0x00FF, 0x0004]), "move.w d0,$FF0004");
        assert_eq!(dis(&[0x4EB9, 0x0000, 0x2000]), "jsr main");
        assert_eq!(dis(&[0x6100, 0x0FFE]), "bsr.w main");
        assert_eq!(dis(&[0x66FE]), "bne.s $001000");
        assert_eq!(dis(&[0x51C9, 0xFFFE]), "dbra d1,$001000");
        assert_eq!(dis(&[0x48E7, 0x70C0]), "movem.l d1-d3/a0-a1,-(sp)");
        assert_eq!(dis(&[0x4CDF, 0x030E]), "movem.l (sp)+,d1-d3/a0-a1");
        assert_eq!(dis(&[0x4EFB, 0x0002]), "jmp $001004(pc,d0.w)");
        assert_eq!(dis(&[0x41FA, 0x0FFE]), "lea main(pc),a0");
        assert_eq!(dis(&[0x0C40, 0x0010]), "cmpi.w #$10,d0");
        assert_eq!(dis(&[0x0839, 0x0003]), "btst #3,…");
        assert_eq!(dis(&[0x5279, 0x00FF]), "addq.w #1,…");
        assert_eq!(dis(&[0xD081]), "add.l d1,d0");
        assert_eq!(dis(&[0xD1C8]), "adda.l a0,a0");
        assert_eq!(dis(&[0xB342]), "eor.w d1,d2");
        assert_eq!(dis(&[0xE548]), "lsl.w #2,d0");
        assert_eq!(dis(&[0x46FC, 0x2700]), "move.w #$2700,sr");
        assert_eq!(dis(&[0x4840]), "swap d0");
        assert_eq!(dis(&[0x4E56, 0xFFF0]), "link a6,#-$10");
        assert_eq!(dis(&[0xC0FC, 0x0111]), "mulu.w #$111,d0");
        assert_eq!(dis(&[0x4A79, 0x00FF]), "tst.w …");
        assert_eq!(dis(&[0x3030, 0x0002]), "move.w $2(a0,d0.w),d0");
        assert_eq!(dis(&[0xA000]), "dc.w $A000");
        assert_eq!(dis(&[0x4848]), "dc.w $4848");
    }
}
