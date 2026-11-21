//! The decoders built into the player: the 6502, MIPS III and ARMv4T.

use crate::{Decoded, Decoder, Kind};

pub mod arm;

pub use arm::Armv4t;

// ---- 6502 ---------------------------------------------------------------------------

/// The 6502 (NES 2A03 included), with the stable undocumented opcodes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mos6502;

/// Instruction length for each 6502 opcode, including the stable undocumented ones.
pub fn op_len_6502(op: u8) -> u8 {
    let odd_row = (op >> 4) & 1 == 1;
    match op & 0x0f {
        0x0 if odd_row => 2,
        0x0 => match op {
            0x20 => 3,
            0x00 | 0x40 | 0x60 => 1,
            _ => 2,
        },
        0x1 | 0x3 | 0x4 | 0x5 | 0x6 | 0x7 => 2,
        0x2 => match op {
            0x82 | 0xA2 | 0xC2 | 0xE2 => 2,
            _ => 1, // KIL / JAM
        },
        0x8 | 0xA => 1,
        0x9 | 0xB if odd_row => 3,
        0x9 | 0xB => 2,
        _ => 3, // 0xC..0xF
    }
}

impl Decoder for Mos6502 {
    fn cpu(&self) -> &'static str {
        "6502"
    }

    fn max_len(&self) -> usize {
        3
    }

    fn decode(&self, bytes: &[u8], pc: u64, _mode: u32) -> Decoded {
        let Some(&op) = bytes.first() else { return Decoded::next(0) };
        let len = op_len_6502(op);
        let b = |i: usize| bytes.get(i).copied().unwrap_or(0);
        let abs = || u16::from_le_bytes([b(1), b(2)]) as u64;
        let d = Decoded::next(len);
        match op {
            0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0 => {
                let target = ((pc as i64 + 2 + b(1) as i8 as i64) & 0xffff) as u64;
                Decoded { kind: Kind::Jump, conditional: true, target: Some(target), ..d }
            }
            0x20 => Decoded { kind: Kind::Call, target: Some(abs()), site: "JSR", ..d },
            0x4C => Decoded { kind: Kind::Jump, target: Some(abs()), site: "JMP", ..d },
            0x6C => Decoded { kind: Kind::Jump, site: "JMP", ..d },
            0x40 | 0x60 => Decoded { kind: Kind::Return, ..d },
            0x00 => Decoded { kind: Kind::Stop, ..d },
            op if op & 0x0f == 0x2 && !matches!(op, 0x82 | 0xA2 | 0xC2 | 0xE2) => Decoded { kind: Kind::Stop, ..d },
            _ => d,
        }
    }
}

// ---- MIPS (the N64's VR4300, "mips3") -----------------------------------------------

/// MIPS III, big-endian (the N64's VR4300; ROMs in .z64 order).
#[derive(Clone, Copy, Debug, Default)]
pub struct Mips3;

impl Decoder for Mips3 {
    fn cpu(&self) -> &'static str {
        "mips3"
    }

    fn max_len(&self) -> usize {
        4
    }

    fn fixed_len(&self) -> Option<u8> {
        Some(4)
    }

    fn alignment(&self, _mode: u32) -> u8 {
        4
    }

    fn decode(&self, bytes: &[u8], pc: u64, _mode: u32) -> Decoded {
        let Some(w) = bytes.get(..4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])) else { return Decoded::next(4) };
        let op = w >> 26;
        let (rs, rt, funct) = ((w >> 21) & 31, (w >> 16) & 31, w & 63);
        let branch = pc.wrapping_add(4).wrapping_add(((w as u16 as i16 as i64) << 2) as u64) & 0xFFFF_FFFF;
        let region = (pc.wrapping_add(4) & 0xF000_0000) | (((w & 0x03FF_FFFF) as u64) << 2);
        let d = Decoded::next(4);
        // Branches and jumps run the next instruction (the delay slot) before control moves.
        let slot = Decoded { delay_slots: 1, ..d };
        let jump = |target| Decoded { kind: Kind::Jump, target, site: "J", ..slot };
        let call = |target| Decoded { kind: Kind::Call, target, site: "JAL", ..slot };
        let branch_to = |target| Decoded { kind: Kind::Jump, conditional: true, target: Some(target), ..slot };
        match op {
            0 => match funct {
                0x08 if rs == 31 => Decoded { kind: Kind::Return, ..slot }, // jr $ra
                0x08 => jump(None),                                         // jr: jump tables
                0x09 => call(None),                                         // jalr
                0x0D => Decoded { kind: Kind::Stop, ..d },                  // break: no delay slot
                _ => d,
            },
            // REGIMM: BLTZ, BGEZ, their likely forms, and the linking ones (BAL is BGEZAL $0).
            1 => match rt {
                0x00..=0x03 => branch_to(branch),
                0x10..=0x13 => call(Some(branch)),
                _ => d,
            },
            2 => jump(Some(region)),
            3 => call(Some(region)),
            // BEQ $0,$0 (and BEQL) is an unconditional branch.
            4 | 0x14 if rs == 0 && rt == 0 => jump(Some(branch)),
            4..=7 | 0x14..=0x17 => branch_to(branch),
            0x10 if rs == 0x10 && funct == 0x18 => Decoded { kind: Kind::Return, ..d }, // eret: no delay slot
            // BC1F, BC1T and their likely forms.
            0x11 if rs == 8 => branch_to(branch),
            _ => d,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mips_kinds() {
        let pc = 0x8000_1004;
        let k = |w: u32| {
            let d = Mips3.decode(&w.to_be_bytes(), pc, 0);
            (d.kind, d.conditional, d.target, d.delay_slots)
        };
        assert_eq!(k(0x27BD_FFE8), (Kind::Next, false, None, 0)); // addiu $sp,$sp,-0x18
        assert_eq!(k(0x1080_0003), (Kind::Jump, true, Some(0x8000_1014), 1)); // beq $a0,$0,+3
        assert_eq!(k(0x5080_FFFF), (Kind::Jump, true, Some(0x8000_1004), 1)); // beql $a0,$0,-1
        assert_eq!(k(0x1000_0002), (Kind::Jump, false, Some(0x8000_1010), 1)); // b +2
        assert_eq!(k(0x0411_0004), (Kind::Call, false, Some(0x8000_1018), 1)); // bal +4
        assert_eq!(k(0x0C00_0800), (Kind::Call, false, Some(0x8000_2000), 1)); // jal 0x80002000
        assert_eq!(k(0x0800_0400), (Kind::Jump, false, Some(0x8000_1000), 1)); // j 0x80001000
        assert_eq!(k(0x03E0_0008), (Kind::Return, false, None, 1)); // jr $ra
        assert_eq!(k(0x0100_0008), (Kind::Jump, false, None, 1)); // jr $t0 (a jump table)
        assert_eq!(k(0x0100_F809), (Kind::Call, false, None, 1)); // jalr $t0
        assert_eq!(k(0x4200_0018), (Kind::Return, false, None, 0)); // eret
        assert_eq!(k(0x4501_0003), (Kind::Jump, true, Some(0x8000_1014), 1)); // bc1t +3
        assert_eq!(k(0x0000_000D), (Kind::Stop, false, None, 0)); // break
    }

    #[test]
    fn mos6502_kinds() {
        let k = |b: &[u8]| {
            let d = Mos6502.decode(b, 0xC000, 0);
            (d.len, d.kind, d.conditional, d.target, d.site)
        };
        assert_eq!(k(&[0xA9, 0x05]), (2, Kind::Next, false, None, ""));
        assert_eq!(k(&[0xD0, 0xFE]), (2, Kind::Jump, true, Some(0xC000), ""));
        assert_eq!(k(&[0x20, 0x34, 0x12]), (3, Kind::Call, false, Some(0x1234), "JSR"));
        assert_eq!(k(&[0x4C, 0x34, 0x12]), (3, Kind::Jump, false, Some(0x1234), "JMP"));
        assert_eq!(k(&[0x6C, 0x34, 0x12]), (3, Kind::Jump, false, None, "JMP"));
        assert_eq!(k(&[0x60]), (1, Kind::Return, false, None, ""));
        assert_eq!(k(&[0x00]), (1, Kind::Stop, false, None, ""));
        assert_eq!(k(&[0x02]), (1, Kind::Stop, false, None, ""));
        assert_eq!(k(&[0xA2, 0x00]), (2, Kind::Next, false, None, ""));
    }
}
