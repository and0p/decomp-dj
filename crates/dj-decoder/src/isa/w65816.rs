//! The 65816 (the SNES's main CPU, "65816").
//!
//! An instruction's length can depend on two flags of the CPU's P register: M (8-bit
//! accumulator and memory) and X (8-bit index registers), which `REP` and `SEP` change.
//! The decoder's mode carries them: bit 0 set means a 16-bit accumulator, bit 1 a 16-bit
//! X and Y. Mode 0 is both 8-bit, as after reset (and always in emulation mode).

use crate::{Decoded, Decoder, Kind};

/// The 65816 (WDC W65C816S; the SNES's 5A22).
#[derive(Clone, Copy, Debug, Default)]
pub struct W65816;

/// Mode bit: the accumulator (and memory) is 16 bits wide (P.M clear).
pub const MODE_A16: u32 = 1;
/// Mode bit: X and Y are 16 bits wide (P.X clear).
pub const MODE_X16: u32 = 2;

/// The length of the instruction with this opcode in `mode` (`MODE_A16`, `MODE_X16`).
pub fn op_len_65816(op: u8, mode: u32) -> u8 {
    let even_row = (op >> 4) & 1 == 0;
    let m = if mode & MODE_A16 != 0 { 3 } else { 2 };
    let x = if mode & MODE_X16 != 0 { 3 } else { 2 };
    match op & 0x0f {
        0x0 if !even_row => 2, // branches
        0x0 => match op {
            0x20 => 3,               // JSR abs
            0x40 | 0x60 => 1,        // RTI, RTS
            0xA0 | 0xC0 | 0xE0 => x, // LDY, CPY, CPX #
            _ => 2,                  // BRK, BRA
        },
        0x2 => match op {
            0x22 => 4,               // JSL
            0x62 | 0x82 => 3,        // PER, BRL
            0xA2 => x,               // LDX #
            _ => 2,                  // COP, WDM, REP, SEP, (dp)
        },
        0x4 => match op {
            0x44 | 0x54 | 0xF4 => 3, // MVP, MVN, PEA
            _ => 2,
        },
        0x1 | 0x3 | 0x5 | 0x6 | 0x7 => 2,
        0x8 | 0xA | 0xB => 1,
        0x9 if even_row => m, // ORA AND EOR ADC BIT LDA CMP SBC #
        0x9 => 3,
        0xC if op == 0x5C => 4, // JML long
        0xC | 0xD | 0xE => 3,
        _ => 4, // 0xF: long addressing
    }
}

impl Decoder for W65816 {
    fn cpu(&self) -> &'static str {
        "65816"
    }

    fn max_len(&self) -> usize {
        4
    }

    /// The connector's mode bits (docs/connectors.md): `0x04` 8-bit accumulator, `0x08`
    /// 16-bit, `0x10` 8-bit index registers, `0x20` 16-bit. An opcode that ran with both
    /// widths of either says nothing.
    fn mode_from_flags(&self, flags: u8) -> Option<u32> {
        let a = match flags & 0x0C {
            0x04 => 0,
            0x08 => MODE_A16,
            _ => return None,
        };
        let x = match flags & 0x30 {
            0x10 => 0,
            0x20 => MODE_X16,
            _ => return None,
        };
        Some(a | x)
    }

    fn decode(&self, bytes: &[u8], pc: u64, mode: u32) -> Decoded {
        let Some(&op) = bytes.first() else { return Decoded::next(0) };
        let len = op_len_65816(op, mode);
        let b = |i: usize| bytes.get(i).copied().unwrap_or(0) as u64;
        let bank = pc & 0xFF_0000;
        // Within the bank the instruction is in: PC wraps at the bank's end.
        let in_bank = |a: u64| bank | (a & 0xFFFF);
        let abs = in_bank(b(1) | b(2) << 8);
        let long = b(1) | b(2) << 8 | b(3) << 16;
        let d = Decoded { mode_after: mode, ..Decoded::next(len) };
        match op {
            // BPL BMI BVC BVS BCC BCS BNE BEQ
            0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0 => {
                Decoded { kind: Kind::Jump, conditional: true, target: Some(in_bank((pc + 2).wrapping_add(b(1) as u8 as i8 as u64))), ..d }
            }
            0x80 => Decoded { kind: Kind::Jump, target: Some(in_bank((pc + 2).wrapping_add(b(1) as u8 as i8 as u64))), site: "BRA", ..d },
            0x82 => Decoded { kind: Kind::Jump, target: Some(in_bank((pc + 3).wrapping_add((b(1) | b(2) << 8) as u16 as i16 as u64))), site: "BRL", ..d },
            0x4C => Decoded { kind: Kind::Jump, target: Some(abs), site: "JMP", ..d },
            0x5C => Decoded { kind: Kind::Jump, target: Some(long), site: "JML", ..d },
            // JMP (abs), JMP (abs,X), JML [abs]: jump tables and pointers.
            0x6C | 0x7C => Decoded { kind: Kind::Jump, site: "JMP", ..d },
            0xDC => Decoded { kind: Kind::Jump, site: "JML", ..d },
            0x20 => Decoded { kind: Kind::Call, target: Some(abs), site: "JSR", ..d },
            0x22 => Decoded { kind: Kind::Call, target: Some(long), site: "JSL", ..d },
            0xFC => Decoded { kind: Kind::Call, site: "JSR", ..d },
            0x40 | 0x60 | 0x6B => Decoded { kind: Kind::Return, ..d }, // RTI, RTS, RTL
            0x00 | 0xDB => Decoded { kind: Kind::Stop, ..d },           // BRK, STP
            // REP clears P bits (wider registers), SEP sets them (narrower).
            0xC2 => Decoded { mode_after: mode | widths(b(1) as u8), ..d },
            0xE2 => Decoded { mode_after: mode & !widths(b(1) as u8), ..d },
            _ => d,
        }
    }
}

/// The mode bits a REP or SEP operand names: P.M (0x20) and P.X (0x10).
fn widths(p: u8) -> u32 {
    (if p & 0x20 != 0 { MODE_A16 } else { 0 }) | (if p & 0x10 != 0 { MODE_X16 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        // Every opcode's length with 8-bit registers, row by row (a reference table).
        const LEN8: [u8; 256] = [
            2, 2, 2, 2, 2, 2, 2, 2, 1, 2, 1, 1, 3, 3, 3, 4, // 0x
            2, 2, 2, 2, 2, 2, 2, 2, 1, 3, 1, 1, 3, 3, 3, 4, // 1x
            3, 2, 4, 2, 2, 2, 2, 2, 1, 2, 1, 1, 3, 3, 3, 4, // 2x
            2, 2, 2, 2, 2, 2, 2, 2, 1, 3, 1, 1, 3, 3, 3, 4, // 3x
            1, 2, 2, 2, 3, 2, 2, 2, 1, 2, 1, 1, 3, 3, 3, 4, // 4x
            2, 2, 2, 2, 3, 2, 2, 2, 1, 3, 1, 1, 4, 3, 3, 4, // 5x
            1, 2, 3, 2, 2, 2, 2, 2, 1, 2, 1, 1, 3, 3, 3, 4, // 6x
            2, 2, 2, 2, 2, 2, 2, 2, 1, 3, 1, 1, 3, 3, 3, 4, // 7x
            2, 2, 3, 2, 2, 2, 2, 2, 1, 2, 1, 1, 3, 3, 3, 4, // 8x
            2, 2, 2, 2, 2, 2, 2, 2, 1, 3, 1, 1, 3, 3, 3, 4, // 9x
            2, 2, 2, 2, 2, 2, 2, 2, 1, 2, 1, 1, 3, 3, 3, 4, // Ax
            2, 2, 2, 2, 2, 2, 2, 2, 1, 3, 1, 1, 3, 3, 3, 4, // Bx
            2, 2, 2, 2, 2, 2, 2, 2, 1, 2, 1, 1, 3, 3, 3, 4, // Cx
            2, 2, 2, 2, 2, 2, 2, 2, 1, 3, 1, 1, 3, 3, 3, 4, // Dx
            2, 2, 2, 2, 2, 2, 2, 2, 1, 2, 1, 1, 3, 3, 3, 4, // Ex
            2, 2, 2, 2, 3, 2, 2, 2, 1, 3, 1, 1, 3, 3, 3, 4, // Fx
        ];
        for op in 0..=255u8 {
            assert_eq!(op_len_65816(op, 0), LEN8[op as usize], "opcode {op:02X}");
            let m = [0x09, 0x29, 0x49, 0x69, 0x89, 0xA9, 0xC9, 0xE9].contains(&op) as u8;
            let x = [0xA0, 0xA2, 0xC0, 0xE0].contains(&op) as u8;
            assert_eq!(op_len_65816(op, MODE_A16), LEN8[op as usize] + m, "opcode {op:02X}, 16-bit A");
            assert_eq!(op_len_65816(op, MODE_X16), LEN8[op as usize] + x, "opcode {op:02X}, 16-bit X");
        }
    }

    #[test]
    fn kinds() {
        let k = |b: &[u8], mode: u32| {
            let d = W65816.decode(b, 0x01_8000, mode);
            (d.len, d.kind, d.conditional, d.target, d.mode_after)
        };
        assert_eq!(k(&[0xA9, 0x05], 0), (2, Kind::Next, false, None, 0));
        assert_eq!(k(&[0xA9, 0x34, 0x12], MODE_A16), (3, Kind::Next, false, None, MODE_A16));
        assert_eq!(k(&[0xC2, 0x30], 0), (2, Kind::Next, false, None, MODE_A16 | MODE_X16));
        assert_eq!(k(&[0xE2, 0x20], MODE_A16 | MODE_X16), (2, Kind::Next, false, None, MODE_X16));
        assert_eq!(k(&[0xD0, 0xFE], 0), (2, Kind::Jump, true, Some(0x01_8000), 0));
        assert_eq!(k(&[0x80, 0x10], 0), (2, Kind::Jump, false, Some(0x01_8012), 0));
        assert_eq!(k(&[0x82, 0xFD, 0xFF], 0), (3, Kind::Jump, false, Some(0x01_8000), 0));
        assert_eq!(k(&[0x20, 0x34, 0x92], 0), (3, Kind::Call, false, Some(0x01_9234), 0));
        assert_eq!(k(&[0x22, 0x00, 0x80, 0x02], 0), (4, Kind::Call, false, Some(0x02_8000), 0));
        assert_eq!(k(&[0x5C, 0x00, 0x80, 0x00], 0), (4, Kind::Jump, false, Some(0x00_8000), 0));
        assert_eq!(k(&[0x7C, 0x00, 0x90], 0), (3, Kind::Jump, false, None, 0));
        assert_eq!(k(&[0x6B], 0), (1, Kind::Return, false, None, 0));
        assert_eq!(k(&[0x60], 0), (1, Kind::Return, false, None, 0));
        assert_eq!(k(&[0x00, 0x00], 0), (2, Kind::Stop, false, None, 0));
        // A branch at the end of a bank stays in it.
        assert_eq!(W65816.decode(&[0x80, 0x10], 0x00_FFF0, 0).target, Some(0x00_0002));
    }

    #[test]
    fn modes_from_flags() {
        assert_eq!(W65816.mode_from_flags(0x01 | 0x04 | 0x10), Some(0));
        assert_eq!(W65816.mode_from_flags(0x01 | 0x08 | 0x20), Some(MODE_A16 | MODE_X16));
        assert_eq!(W65816.mode_from_flags(0x01 | 0x08 | 0x10), Some(MODE_A16));
        assert_eq!(W65816.mode_from_flags(0x01 | 0x04 | 0x08 | 0x10), None);
        assert_eq!(W65816.mode_from_flags(0x01), None);
    }
}
