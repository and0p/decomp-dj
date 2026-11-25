//! The Motorola 68000 decoder (the Mega Drive / Genesis's main CPU; also the Mega CD's
//! second CPU, the Neo Geo, Amiga and Atari ST), big-endian, 24-bit addresses. No modes.
//!
//! An instruction is an opcode word and up to four extension words: an immediate, a
//! displacement, a MOVEM register mask, and each effective address's own words. What it
//! reads, beyond lengths: branches (`Bcc`, `BRA`, `DBcc`), calls (`BSR`, `JSR`), jumps
//! (`JMP`, tables through `(An)` or `(d8,PC,Xn)` have no target), returns (`RTS`, `RTE`,
//! `RTR`), and data read PC-relative through `(d16,PC)` (`data_ref`), so a table after a
//! function's code isn't read as code. Encodings the 68000 doesn't have (the 68010's and
//! later CPUs' instructions) are not instructions; `ILLEGAL` and the line-A and line-F
//! traps stop.

use crate::{Decoded, Decoder, Kind};

/// The 68000.
#[derive(Clone, Copy, Debug, Default)]
pub struct M68k;

impl Decoder for M68k {
    fn cpu(&self) -> &'static str {
        "m68k"
    }

    fn max_len(&self) -> usize {
        10
    }

    fn alignment(&self, _mode: u32) -> u8 {
        2
    }

    fn decode(&self, bytes: &[u8], pc: u64, _mode: u32) -> Decoded {
        decode(bytes, pc & 0xFF_FFFF)
    }
}

/// Operand sizes, in bytes.
const B: u8 = 1;
const W: u8 = 2;
const L: u8 = 4;

fn not_instruction() -> Decoded {
    Decoded::next(0)
}

fn addr(a: i64) -> u64 {
    (a as u64) & 0xFF_FFFF
}

/// The instruction's bytes, as 16-bit words; reading past the end gives 0 (the length still
/// says how long it is, and the caller sees that it doesn't fit).
struct Words<'a>(&'a [u8]);

impl Words<'_> {
    fn at(&self, byte: usize) -> u16 {
        let b = |i: usize| self.0.get(i).copied().unwrap_or(0);
        u16::from_be_bytes([b(byte), b(byte + 1)])
    }
}

/// An effective address: how many extension bytes it takes, and where it points when the
/// bytes say (absolute and PC-relative addresses).
struct Ea {
    ext: u8,
    target: Option<u64>,
    pc_relative: bool,
}

/// The effective address `mode`/`reg` of an operand of `size` bytes, whose extension words
/// start `at` bytes into the instruction at `pc`. None: not an addressing mode the 68000
/// has.
fn ea(w: &Words, pc: u64, at: usize, mode: u16, reg: u16, size: u8) -> Option<Ea> {
    let plain = |ext| Some(Ea { ext, target: None, pc_relative: false });
    match mode {
        0..=4 => plain(0),
        5 | 6 => plain(2),
        7 => match reg {
            0 => Some(Ea { ext: 2, target: Some(addr(w.at(at) as i16 as i64)), pc_relative: false }),
            1 => Some(Ea { ext: 4, target: Some(addr(((w.at(at) as u32) << 16 | w.at(at + 2) as u32) as i64)), pc_relative: false }),
            // The displacement is from the address of the extension word.
            2 => Some(Ea { ext: 2, target: Some(addr(pc as i64 + at as i64 + w.at(at) as i16 as i64)), pc_relative: true }),
            3 => Some(Ea { ext: 2, target: None, pc_relative: true }),
            4 => plain(if size == L { 4 } else { 2 }),
            _ => None,
        },
        _ => None,
    }
}

fn decode(bytes: &[u8], pc: u64) -> Decoded {
    if bytes.is_empty() {
        return not_instruction();
    }
    let w = Words(bytes);
    let op = w.at(0);
    let mode = (op >> 3) & 7;
    let reg = op & 7;
    let ss = (op >> 6) & 3;
    // The common size field: 00 byte, 01 word, 10 long (11 is another instruction).
    let size = |s: u16| [B, W, L][s as usize];

    // An instruction of `pre` bytes (opcode word and fixed extension words) and then the
    // effective address in its low six bits.
    let with_ea = |pre: usize, size: u8, reads: bool| -> Decoded {
        let Some(e) = ea(&w, pc, pre, mode, reg, size) else { return not_instruction() };
        let d = Decoded::next(pre as u8 + e.ext);
        match e.target {
            Some(t) if e.pc_relative && reads => Decoded { data_ref: Some((t, size)), ..d },
            _ => d,
        }
    };

    match op >> 12 {
        0x0 => {
            // To CCR and SR: ORI, ANDI, EORI.
            match op {
                0x003C | 0x023C | 0x0A3C | 0x007C | 0x027C | 0x0A7C => return Decoded::next(4),
                _ => {}
            }
            if op & 0x0100 != 0 {
                // MOVEP, then BTST/BCHG/BCLR/BSET with the bit number in a register.
                if mode == 1 {
                    return Decoded::next(4);
                }
                return with_ea(2, B, true);
            }
            match (op >> 9) & 7 {
                // ORI, ANDI, SUBI, ADDI, EORI, CMPI.
                0 | 1 | 2 | 3 | 5 | 6 if ss != 3 => {
                    let imm = if ss == 2 { 4 } else { 2 };
                    with_ea(2 + imm, size(ss), true)
                }
                // BTST/BCHG/BCLR/BSET with the bit number in an extension word.
                4 => with_ea(4, B, true),
                _ => not_instruction(),
            }
        }
        // MOVE.B, MOVE.L, MOVE.W (and MOVEA): the source, then the destination.
        0x1..=0x3 => {
            let sz = match op >> 12 {
                1 => B,
                2 => L,
                _ => W,
            };
            let Some(src) = ea(&w, pc, 2, mode, reg, sz) else { return not_instruction() };
            let (dmode, dreg) = ((op >> 6) & 7, (op >> 9) & 7);
            if dmode == 7 && dreg > 1 || dmode == 1 && sz == B {
                return not_instruction();
            }
            let Some(dst) = ea(&w, pc, 2 + src.ext as usize, dmode, dreg, sz) else { return not_instruction() };
            let d = Decoded::next(2 + src.ext + dst.ext);
            match src.target {
                Some(t) if src.pc_relative => Decoded { data_ref: Some((t, sz)), ..d },
                _ => d,
            }
        }
        0x4 => line4(&w, pc, op, mode, reg, with_ea),
        0x5 => {
            if ss != 3 {
                return with_ea(2, size(ss), true); // ADDQ, SUBQ
            }
            let cond = (op >> 8) & 15;
            if mode == 1 {
                // DBcc: DBT never branches; the rest loop on a counter.
                let d = Decoded::next(4);
                if cond == 0 {
                    return d;
                }
                let target = addr(pc as i64 + 2 + w.at(2) as i16 as i64);
                return Decoded { kind: Kind::Jump, conditional: true, target: Some(target), site: "DBcc", ..d };
            }
            with_ea(2, B, false) // Scc
        }
        0x6 => {
            let cond = (op >> 8) & 15;
            let d8 = op as u8;
            let (len, disp) = if d8 == 0 { (4, w.at(2) as i16 as i64) } else { (2, d8 as i8 as i64) };
            let target = Some(addr(pc as i64 + 2 + disp));
            let d = Decoded::next(len);
            match cond {
                0 => Decoded { kind: Kind::Jump, target, site: "BRA", ..d },
                1 => Decoded { kind: Kind::Call, target, site: "BSR", ..d },
                _ => Decoded { kind: Kind::Jump, conditional: true, target, ..d },
            }
        }
        0x7 if op & 0x0100 == 0 => Decoded::next(2), // MOVEQ
        0x7 => not_instruction(),
        // OR, SUB, CMP and EOR, AND, ADD, with their word-only and two-register forms.
        0x8 | 0x9 | 0xB | 0xC | 0xD => {
            let line = op >> 12;
            let opmode = (op >> 6) & 7;
            match opmode {
                // DIVU, DIVS, MULU, MULS (words); SUBA, CMPA, ADDA (.W and .L).
                3 | 7 => {
                    let sz = if matches!(line, 0x8 | 0xC) || opmode == 3 { W } else { L };
                    with_ea(2, sz, true)
                }
                0..=2 => with_ea(2, size(opmode), true),
                _ => {
                    let two_regs = match line {
                        0x8 | 0xC if opmode == 4 => mode <= 1,          // SBCD, ABCD
                        0xC => (opmode == 5 && mode <= 1) || (opmode == 6 && mode == 1), // EXG
                        0x9 | 0xD => mode <= 1,                        // SUBX, ADDX
                        0xB => mode == 1,                              // CMPM
                        _ => false,
                    };
                    if two_regs {
                        return Decoded::next(2);
                    }
                    // A register into memory: no immediates or PC-relative destinations.
                    if mode == 7 && reg > 1 || mode == 1 && line != 0xB {
                        return not_instruction();
                    }
                    with_ea(2, size(opmode - 4), true)
                }
            }
        }
        // The line-A trap (unused on the Mega Drive).
        0xA => Decoded { kind: Kind::Stop, ..Decoded::next(2) },
        0xE => {
            if ss != 3 {
                return Decoded::next(2); // shifts and rotates of a register
            }
            if op & 0x0800 != 0 {
                return not_instruction(); // bit fields (68020)
            }
            with_ea(2, W, true) // a word in memory, by one bit
        }
        // The line-F trap (coprocessors on later CPUs).
        _ => Decoded { kind: Kind::Stop, ..Decoded::next(2) },
    }
}

fn line4(w: &Words, pc: u64, op: u16, mode: u16, reg: u16, with_ea: impl Fn(usize, u8, bool) -> Decoded) -> Decoded {
    let ss = (op >> 6) & 3;
    let size = |s: u16| [B, W, L][s as usize];
    let control = |kind: Kind, site: &'static str| -> Decoded {
        // JSR and JMP: only addressing modes that name a place.
        if matches!(mode, 0 | 1 | 3 | 4) || mode == 7 && reg == 4 {
            return not_instruction();
        }
        let Some(e) = ea(w, pc, 2, mode, reg, L) else { return not_instruction() };
        Decoded { kind, target: e.target, site, ..Decoded::next(2 + e.ext) }
    };
    if op & 0x0100 != 0 {
        return match ss {
            2 => with_ea(2, W, true),  // CHK
            3 => with_ea(2, L, false), // LEA
            _ => not_instruction(),
        };
    }
    match op & 0x0FC0 {
        0x00C0 => return with_ea(2, W, false), // MOVE from SR
        0x04C0 | 0x06C0 => return with_ea(2, W, true), // MOVE to CCR, to SR
        0x0800 => return with_ea(2, B, false), // NBCD
        0x0840 if mode == 0 => return Decoded::next(2), // SWAP
        0x0840 if mode == 1 => return not_instruction(), // BKPT (68010)
        0x0840 => return with_ea(2, L, false), // PEA
        0x0880 | 0x08C0 if mode == 0 => return Decoded::next(2), // EXT
        0x0880 | 0x08C0 | 0x0C80 | 0x0CC0 => return with_ea(4, if ss == 3 { L } else { W }, true), // MOVEM
        0x0AC0 if op == 0x4AFC => return Decoded { kind: Kind::Stop, ..Decoded::next(2) }, // ILLEGAL
        0x0AC0 => return with_ea(2, B, false), // TAS
        0x0E80 => return control(Kind::Call, "JSR"),
        0x0EC0 => return control(Kind::Jump, "JMP"),
        _ => {}
    }
    match op & 0x0F00 {
        // NEGX, CLR, NEG, NOT, TST.
        0x0000 | 0x0200 | 0x0400 | 0x0600 | 0x0A00 if ss != 3 => return with_ea(2, size(ss), true),
        _ => {}
    }
    match op & 0x0FF8 {
        0x0E40 | 0x0E48 => return Decoded::next(2), // TRAP: the handler returns with RTE
        0x0E50 => return Decoded::next(4),          // LINK
        0x0E58 | 0x0E60 | 0x0E68 => return Decoded::next(2), // UNLK, MOVE USP
        _ => {}
    }
    match op {
        0x4E70 | 0x4E71 | 0x4E76 => Decoded::next(2), // RESET, NOP, TRAPV
        0x4E72 => Decoded::next(4),                   // STOP: on after an interrupt
        0x4E73 | 0x4E75 | 0x4E77 => Decoded { kind: Kind::Return, ..Decoded::next(2) }, // RTE, RTS, RTR
        _ => not_instruction(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn k(words: &[u16]) -> (u8, Kind, bool, Option<u64>) {
        let d = M68k.decode(&bytes(words), 0x1000, 0);
        (d.len, d.kind, d.conditional, d.target)
    }

    fn len(words: &[u16]) -> u8 {
        let mut b = bytes(words);
        b.extend([0; 10]);
        M68k.decode(&b, 0x1000, 0).len
    }

    #[test]
    fn lengths() {
        assert_eq!(len(&[0x4E71]), 2); // nop
        assert_eq!(len(&[0x7001]), 2); // moveq #1,d0
        assert_eq!(len(&[0x303C, 0x1234]), 4); // move.w #$1234,d0
        assert_eq!(len(&[0x203C, 0x1234, 0x5678]), 6); // move.l #$12345678,d0
        assert_eq!(len(&[0x23FC, 0, 0, 0x00FF, 0]), 10); // move.l #0,$FF0000
        assert_eq!(len(&[0x33C0, 0x00C0, 0x0000]), 6); // move.w d0,$C00000
        assert_eq!(len(&[0x3140, 0x0004]), 4); // move.w d0,4(a0)
        assert_eq!(len(&[0x41F9, 0x00FF, 0x0000]), 6); // lea $FF0000,a0
        assert_eq!(len(&[0x41FA, 0x0010]), 4); // lea 16(pc),a0
        assert_eq!(len(&[0x0C40, 0x0010]), 4); // cmpi.w #16,d0
        assert_eq!(len(&[0x0C80, 0, 0x10]), 6); // cmpi.l #16,d0
        assert_eq!(len(&[0x0839, 0x0003, 0x00A1, 0x0001]), 8); // btst #3,$A10001
        assert_eq!(len(&[0x48E7, 0xFFFE]), 4); // movem.l d0-a6,-(sp)
        assert_eq!(len(&[0x4CDF, 0x7FFF]), 4); // movem.l (sp)+,d0-a6
        assert_eq!(len(&[0x4CB9, 0x00FF, 0x00FF, 0x0000]), 8); // movem.w $FF0000,d0-d7
        assert_eq!(len(&[0x4E56, 0xFFF0]), 4); // link a6,#-16
        assert_eq!(len(&[0x46FC, 0x2700]), 4); // move #$2700,sr
        assert_eq!(len(&[0x007C, 0x0700]), 4); // ori #$700,sr
        assert_eq!(len(&[0x5279, 0x00FF, 0x0000]), 6); // addq.w #1,$FF0000
        assert_eq!(len(&[0xD0BC, 0, 1]), 6); // add.l #1,d0
        assert_eq!(len(&[0xD1B9, 0x00FF, 0x0000]), 6); // add.l d0,$FF0000
        assert_eq!(len(&[0xC0FC, 0x0003]), 4); // mulu #3,d0
        assert_eq!(len(&[0x80FC, 0x0003]), 4); // divu #3,d0
        assert_eq!(len(&[0xB1C8]), 2); // cmpa.l a0,a0
        assert_eq!(len(&[0xB308]), 2); // cmpm.b (a0)+,(a1)+
        assert_eq!(len(&[0xC141]), 2); // exg d0,d1
        assert_eq!(len(&[0xE548]), 2); // lsl.w #2,d0
        assert_eq!(len(&[0xE1F9, 0x00FF, 0x0000]), 6); // asl.w $FF0000
        assert_eq!(len(&[0x4A79, 0x00FF, 0x0000]), 6); // tst.w $FF0000
        assert_eq!(len(&[0x4279, 0x00FF, 0x0000]), 6); // clr.w $FF0000
        assert_eq!(len(&[0x0188, 0x0004]), 4); // movep.w d0,4(a0)
        assert_eq!(len(&[0x4E72, 0x2000]), 4); // stop #$2000
        assert_eq!(len(&[0x3030, 0x0000]), 4); // move.w 0(a0,d0.w),d0
        assert_eq!(len(&[0x303B, 0x0006]), 4); // move.w 6(pc,d0.w),d0
        // Not the 68000's.
        assert_eq!(len(&[0x4E74, 0]), 0); // rtd (68010)
        assert_eq!(len(&[0x4848]), 0); // bkpt (68010)
        assert_eq!(len(&[0x00C0, 0]), 0); // chk2/cmp2 (68020)
        assert_eq!(len(&[0x303D]), 0); // mode 7, register 5
        assert_eq!(len(&[0x7100]), 0); // moveq with bit 8 set
        assert_eq!(len(&[0xE8C0, 0]), 0); // bftst (68020)
        assert_eq!(len(&[0x13FC, 0x0001, 0x00FF, 0x0000]), 8); // move.b #1,$FF0000
        assert_eq!(len(&[0x307C, 0x0001]), 4); // movea.w #1,a0
        assert_eq!(len(&[0x103C, 0x0001]), 4); // move.b #1,d0
    }

    #[test]
    fn flow() {
        assert_eq!(k(&[0x4E75]), (2, Kind::Return, false, None)); // rts
        assert_eq!(k(&[0x4E73]), (2, Kind::Return, false, None)); // rte
        assert_eq!(k(&[0x4E77]), (2, Kind::Return, false, None)); // rtr
        assert_eq!(k(&[0x60FE]), (2, Kind::Jump, false, Some(0x1000))); // bra.s *
        assert_eq!(k(&[0x6000, 0x0010]), (4, Kind::Jump, false, Some(0x1012))); // bra.w
        assert_eq!(k(&[0x6604]), (2, Kind::Jump, true, Some(0x1006))); // bne.s
        assert_eq!(k(&[0x6700, 0xFFFE]), (4, Kind::Jump, true, Some(0x1000))); // beq.w *
        assert_eq!(k(&[0x6100, 0x0100]), (4, Kind::Call, false, Some(0x1102))); // bsr.w
        assert_eq!(k(&[0x6110]), (2, Kind::Call, false, Some(0x1012))); // bsr.s
        assert_eq!(k(&[0x51C8, 0xFFFE]), (4, Kind::Jump, true, Some(0x1000))); // dbf d0,*
        assert_eq!(k(&[0x50C8, 0xFFFE]), (4, Kind::Next, false, None)); // dbt never branches
        assert_eq!(k(&[0x4EB9, 0x0000, 0x0200]), (6, Kind::Call, false, Some(0x200))); // jsr $200
        assert_eq!(k(&[0x4EB8, 0x8000]), (4, Kind::Call, false, Some(0xFF8000))); // jsr ($8000).w
        assert_eq!(k(&[0x4EBA, 0x0010]), (4, Kind::Call, false, Some(0x1012))); // jsr 16(pc)
        assert_eq!(k(&[0x4E90]), (2, Kind::Call, false, None)); // jsr (a0)
        assert_eq!(k(&[0x4EF9, 0x0000, 0x0300]), (6, Kind::Jump, false, Some(0x300))); // jmp $300
        assert_eq!(k(&[0x4ED0]), (2, Kind::Jump, false, None)); // jmp (a0)
        assert_eq!(k(&[0x4EFB, 0x0002]), (4, Kind::Jump, false, None)); // jmp 2(pc,d0.w): a table
        assert_eq!(k(&[0x4AFC]), (2, Kind::Stop, false, None)); // illegal
        assert_eq!(k(&[0xA000]), (2, Kind::Stop, false, None)); // line A
        assert_eq!(k(&[0xF000]), (2, Kind::Stop, false, None)); // line F
        assert_eq!(k(&[0x4E40]), (2, Kind::Next, false, None)); // trap #0
        assert_eq!(k(&[0x4E80]), (0, Kind::Next, false, None)); // jsr d0: not an instruction
        // Branch targets wrap at 24 bits.
        assert_eq!(M68k.decode(&bytes(&[0x60FC]), 0x000000, 0).target, Some(0xFF_FFFE));
    }

    #[test]
    fn pc_relative_data() {
        // move.w 16(pc),d0: the word at the extension word's address + 16.
        assert_eq!(M68k.decode(&[0x30, 0x3A, 0x00, 0x10], 0x1000, 0).data_ref, Some((0x1012, 2)));
        // lea and jmp don't read there.
        assert_eq!(M68k.decode(&[0x41, 0xFA, 0x00, 0x10], 0x1000, 0).data_ref, None);
        assert_eq!(M68k.decode(&[0x4E, 0xFA, 0x00, 0x10], 0x1000, 0).data_ref, None);
        // cmp.l 8(pc),d0
        assert_eq!(M68k.decode(&[0xB0, 0xBA, 0x00, 0x08], 0x1000, 0).data_ref, Some((0x100A, 4)));
    }
}
