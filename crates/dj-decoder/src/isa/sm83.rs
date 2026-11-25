//! The SM83 (the Game Boy's and Game Boy Color's CPU, often called the GBZ80): lengths and
//! control flow for the sweep, and text for traces.

use crate::{Decoded, Decoder, Kind};

/// The SM83, as SameBoy's connector reports it (`sm83`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Sm83;

/// The opcodes that lock the CPU up.
fn illegal(op: u8) -> bool {
    matches!(op, 0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD)
}

/// The length of the instruction an opcode starts: 1 byte, 2 for an 8-bit operand (and
/// `CB` with its second opcode byte, and `STOP` as assemblers write it), 3 for a 16-bit one.
pub fn op_len_sm83(op: u8) -> u8 {
    match op {
        0x01 | 0x08 | 0x11 | 0x21 | 0x31 | 0xC2 | 0xC3 | 0xC4 | 0xCA | 0xCC | 0xCD | 0xD2 | 0xD4 | 0xDA | 0xDC | 0xEA | 0xFA => 3,
        0x10 | 0x18 | 0x20 | 0x28 | 0x30 | 0x38 | 0xCB | 0xE0 | 0xE8 | 0xF0 | 0xF8 => 2,
        op if op < 0x40 && op & 0x07 == 0x06 => 2, // ld r, n8
        op if op >= 0xC0 && op & 0x07 == 0x06 => 2, // add a, n8 and the other ALU ops
        _ => 1,
    }
}

impl Decoder for Sm83 {
    fn cpu(&self) -> &'static str {
        "sm83"
    }

    fn max_len(&self) -> usize {
        3
    }

    fn decode(&self, bytes: &[u8], pc: u64, _mode: u32) -> Decoded {
        let Some(&op) = bytes.first() else { return Decoded::next(0) };
        let len = op_len_sm83(op);
        let b = |i: usize| bytes.get(i).copied().unwrap_or(0);
        let abs = || u16::from_le_bytes([b(1), b(2)]) as u64;
        let rel = || (pc as i64 + 2 + b(1) as i8 as i64) as u64 & 0xFFFF;
        let d = Decoded::next(len);
        match op {
            0x18 => Decoded { kind: Kind::Jump, target: Some(rel()), site: "JR", ..d },
            0x20 | 0x28 | 0x30 | 0x38 => Decoded { kind: Kind::Jump, conditional: true, target: Some(rel()), site: "JR", ..d },
            0xC3 => Decoded { kind: Kind::Jump, target: Some(abs()), site: "JP", ..d },
            0xC2 | 0xCA | 0xD2 | 0xDA => Decoded { kind: Kind::Jump, conditional: true, target: Some(abs()), site: "JP", ..d },
            0xE9 => Decoded { kind: Kind::Jump, site: "JP", ..d }, // jp hl: jump tables
            0xCD => Decoded { kind: Kind::Call, target: Some(abs()), site: "CALL", ..d },
            0xC4 | 0xCC | 0xD4 | 0xDC => Decoded { kind: Kind::Call, conditional: true, target: Some(abs()), site: "CALL", ..d },
            op if op & 0xC7 == 0xC7 => Decoded { kind: Kind::Call, target: Some((op & 0x38) as u64), site: "RST", ..d },
            0xC9 | 0xD9 => Decoded { kind: Kind::Return, ..d },
            0xC0 | 0xC8 | 0xD0 | 0xD8 => Decoded { kind: Kind::Return, conditional: true, ..d },
            op if illegal(op) => Decoded { kind: Kind::Stop, ..d },
            _ => d,
        }
    }
}

const R8: [&str; 8] = ["b", "c", "d", "e", "h", "l", "[hl]", "a"];
const R16: [&str; 4] = ["bc", "de", "hl", "sp"];
const R16_STACK: [&str; 4] = ["bc", "de", "hl", "af"];
const R16_MEM: [&str; 4] = ["[bc]", "[de]", "[hl+]", "[hl-]"];
const COND: [&str; 4] = ["nz", "z", "nc", "c"];
const ALU: [&str; 8] = ["add", "adc", "sub", "sbc", "and", "xor", "or", "cp"];
const ROT: [&str; 8] = ["rlc", "rrc", "rl", "rr", "sla", "sra", "swap", "srl"];

/// `ld a, [hl+]` for the instruction at CPU address `pc` with opcode `op` and its operand
/// bytes, as RGBDS writes it. `name` turns a CPU address into a label when one is known.
/// Bytes that aren't an instruction come out as `db $xx`.
pub fn disasm_sm83(pc: u16, op: u8, operands: &[u8], name: &dyn Fn(u16) -> Option<String>) -> String {
    let b = |i: usize| operands.get(i).copied().unwrap_or(0);
    let n8 = || format!("${:02X}", b(0));
    let n16 = || u16::from_le_bytes([b(0), b(1)]);
    let addr = |a: u16| name(a).unwrap_or_else(|| format!("${a:04X}"));
    let rel = || addr((pc as i32 + 2 + b(0) as i8 as i32) as u16);
    let e8 = || {
        let e = b(0) as i8 as i32;
        if e < 0 { format!("-${:02X}", -e) } else { format!("${e:02X}") }
    };
    let (x, y, z) = ((op >> 6) as usize, ((op >> 3) & 7) as usize, (op & 7) as usize);
    let (p, q) = (y >> 1, y & 1);
    match x {
        0 => match (z, op) {
            (_, 0x00) => "nop".into(),
            (_, 0x08) => format!("ld [{}], sp", addr(n16())),
            (_, 0x10) => "stop".into(),
            (_, 0x18) => format!("jr {}", rel()),
            (0, _) => format!("jr {}, {}", COND[y - 4], rel()),
            (1, _) if q == 0 => format!("ld {}, {}", R16[p], addr(n16())),
            (1, _) => format!("add hl, {}", R16[p]),
            (2, _) if q == 0 => format!("ld {}, a", R16_MEM[p]),
            (2, _) => format!("ld a, {}", R16_MEM[p]),
            (3, _) => format!("{} {}", if q == 0 { "inc" } else { "dec" }, R16[p]),
            (4, _) => format!("inc {}", R8[y]),
            (5, _) => format!("dec {}", R8[y]),
            (6, _) => format!("ld {}, {}", R8[y], n8()),
            _ => ["rlca", "rrca", "rla", "rra", "daa", "cpl", "scf", "ccf"][y].into(),
        },
        1 if op == 0x76 => "halt".into(),
        1 => format!("ld {}, {}", R8[y], R8[z]),
        2 => format!("{} a, {}", ALU[y], R8[z]),
        _ => match op {
            0xC0 | 0xC8 | 0xD0 | 0xD8 => format!("ret {}", COND[y]),
            0xC9 => "ret".into(),
            0xD9 => "reti".into(),
            0xC2 | 0xCA | 0xD2 | 0xDA => format!("jp {}, {}", COND[y], addr(n16())),
            0xC3 => format!("jp {}", addr(n16())),
            0xE9 => "jp hl".into(),
            0xC4 | 0xCC | 0xD4 | 0xDC => format!("call {}, {}", COND[y], addr(n16())),
            0xCD => format!("call {}", addr(n16())),
            0xCB => {
                let o = b(0);
                let (cx, cy, cz) = ((o >> 6) as usize, ((o >> 3) & 7) as usize, (o & 7) as usize);
                match cx {
                    0 => format!("{} {}", ROT[cy], R8[cz]),
                    _ => format!("{} {cy}, {}", ["", "bit", "res", "set"][cx], R8[cz]),
                }
            }
            0xE0 => format!("ldh [{}], a", addr(0xFF00 | b(0) as u16)),
            0xF0 => format!("ldh a, [{}]", addr(0xFF00 | b(0) as u16)),
            0xE2 => "ldh [c], a".into(),
            0xF2 => "ldh a, [c]".into(),
            0xEA => format!("ld [{}], a", addr(n16())),
            0xFA => format!("ld a, [{}]", addr(n16())),
            0xE8 => format!("add sp, {}", e8()),
            0xF8 => format!("ld hl, sp + {}", e8()),
            0xF9 => "ld sp, hl".into(),
            0xF3 => "di".into(),
            0xFB => "ei".into(),
            op if illegal(op) => format!("db ${op:02X}"),
            op if z == 1 => format!("pop {}", R16_STACK[op as usize >> 4 & 3]),
            op if z == 5 => format!("push {}", R16_STACK[op as usize >> 4 & 3]),
            _ if z == 6 => format!("{} a, {}", ALU[y], n8()),
            op => format!("rst ${:02X}", op & 0x38),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        // Every opcode by the table in the Pan Docs: 3 bytes, 2 bytes, else 1.
        let three = [0x01, 0x08, 0x11, 0x21, 0x31, 0xC2, 0xC3, 0xC4, 0xCA, 0xCC, 0xCD, 0xD2, 0xD4, 0xDA, 0xDC, 0xEA, 0xFA];
        let two = [0x06, 0x0E, 0x10, 0x16, 0x18, 0x1E, 0x20, 0x26, 0x28, 0x2E, 0x30, 0x36, 0x38, 0x3E, 0xC6, 0xCB, 0xCE, 0xD6, 0xDE, 0xE0, 0xE6, 0xE8, 0xEE, 0xF0, 0xF6, 0xF8, 0xFE];
        for op in 0..=255u8 {
            let want = if three.contains(&op) { 3 } else if two.contains(&op) { 2 } else { 1 };
            assert_eq!(op_len_sm83(op), want, "{op:02X}");
        }
    }

    #[test]
    fn kinds() {
        let k = |b: &[u8]| {
            let d = Sm83.decode(b, 0x4000, 0);
            (d.len, d.kind, d.conditional, d.target, d.site)
        };
        assert_eq!(k(&[0x3E, 0x05]), (2, Kind::Next, false, None, ""));
        assert_eq!(k(&[0x18, 0xFE]), (2, Kind::Jump, false, Some(0x4000), "JR"));
        assert_eq!(k(&[0x20, 0x02]), (2, Kind::Jump, true, Some(0x4004), "JR"));
        assert_eq!(k(&[0xC3, 0x50, 0x01]), (3, Kind::Jump, false, Some(0x0150), "JP"));
        assert_eq!(k(&[0xDA, 0x00, 0x40]), (3, Kind::Jump, true, Some(0x4000), "JP"));
        assert_eq!(k(&[0xE9]), (1, Kind::Jump, false, None, "JP"));
        assert_eq!(k(&[0xCD, 0x34, 0x12]), (3, Kind::Call, false, Some(0x1234), "CALL"));
        assert_eq!(k(&[0xC4, 0x34, 0x12]), (3, Kind::Call, true, Some(0x1234), "CALL"));
        assert_eq!(k(&[0xEF]), (1, Kind::Call, false, Some(0x28), "RST"));
        assert_eq!(k(&[0xC9]), (1, Kind::Return, false, None, ""));
        assert_eq!(k(&[0xD9]), (1, Kind::Return, false, None, ""));
        assert_eq!(k(&[0xD8]), (1, Kind::Return, true, None, ""));
        assert_eq!(k(&[0xD3]), (1, Kind::Stop, false, None, ""));
        assert_eq!(k(&[0xCB, 0x7C]), (2, Kind::Next, false, None, ""));
        // A backward jr wraps within the 16-bit address space.
        assert_eq!(Sm83.decode(&[0x18, 0x80], 0x0010, 0).target, Some(0xFF92));
    }

    #[test]
    fn disassembles() {
        let none = |_: u16| None;
        let d = |op: u8, o: &[u8]| disasm_sm83(0x0150, op, o, &none);
        assert_eq!(d(0x00, &[]), "nop");
        assert_eq!(d(0x3E, &[0x05]), "ld a, $05");
        assert_eq!(d(0x21, &[0x00, 0xC0]), "ld hl, $C000");
        assert_eq!(d(0x2A, &[]), "ld a, [hl+]");
        assert_eq!(d(0x36, &[0x12]), "ld [hl], $12");
        assert_eq!(d(0x78, &[]), "ld a, b");
        assert_eq!(d(0x76, &[]), "halt");
        assert_eq!(d(0xAF, &[]), "xor a, a");
        assert_eq!(d(0xFE, &[0x90]), "cp a, $90");
        assert_eq!(d(0x20, &[0xFE]), "jr nz, $0150");
        assert_eq!(d(0xCD, &[0x00, 0x40]), "call $4000");
        assert_eq!(disasm_sm83(0x0150, 0xCD, &[0x00, 0x40], &|a| (a == 0x4000).then(|| "on_a".into())), "call on_a");
        assert_eq!(d(0xE0, &[0x46]), "ldh [$FF46], a");
        assert_eq!(d(0xEA, &[0x00, 0x21]), "ld [$2100], a");
        assert_eq!(d(0xCB, &[0x7C]), "bit 7, h");
        assert_eq!(d(0xCB, &[0x37]), "swap a");
        assert_eq!(d(0xF5, &[]), "push af");
        assert_eq!(d(0xC1, &[]), "pop bc");
        assert_eq!(d(0xFF, &[]), "rst $38");
        assert_eq!(d(0xE8, &[0xFE]), "add sp, -$02");
        assert_eq!(d(0xF8, &[0x02]), "ld hl, sp + $02");
        assert_eq!(d(0xD8, &[]), "ret c");
        assert_eq!(d(0xD3, &[]), "db $D3");
        assert_eq!(d(0x08, &[0x00, 0xC1]), "ld [$C100], sp");
        assert_eq!(d(0x10, &[0x00]), "stop");
    }
}
