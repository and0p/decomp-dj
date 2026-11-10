//! The ARMv4T decoder (the ARM7TDMI: GBA, and the DS's second CPU): ARM and Thumb code,
//! little-endian. Modes: [`ARM`] and [`THUMB`]; a function's `mode` says which it starts in
//! (from an ELF symbol's bit 0, or the connector's mode bits where it ran).
//!
//! What it reads, beyond lengths: branches and calls (`B`, `BL`, Thumb's two-halfword `BL`),
//! anything else that writes the PC (`BX`, `MOV pc`, `ADD pc` jump tables, `LDR pc`, `LDM`
//! and `POP` with the PC), conditional execution on every ARM instruction, the switch to
//! ARM through `bx pc`, and PC-relative literal loads (`data_ref`), so literal pools inside
//! a function aren't read as code. Checked against LLVM's disassembler
//! (decomp-dj's .agents/DECODER_PLAN.md).

use crate::{Decoded, Decoder, Kind};

/// ARM mode: 4-byte instructions.
pub const ARM: u32 = 0;
/// Thumb mode: 2-byte instructions (4 for `BL`).
pub const THUMB: u32 = 1;

/// The connector's mode bits on an opcode's first byte (docs/connectors.md): it ran in
/// Thumb, or in ARM state.
pub const FLAG_THUMB: u8 = 0x04;
pub const FLAG_ARM: u8 = 0x08;

/// ARMv4T, little-endian.
#[derive(Clone, Copy, Debug, Default)]
pub struct Armv4t;

impl Decoder for Armv4t {
    fn cpu(&self) -> &'static str {
        "armv4t"
    }

    fn max_len(&self) -> usize {
        4
    }

    fn alignment(&self, mode: u32) -> u8 {
        if mode == THUMB { 2 } else { 4 }
    }

    fn mode_from_flags(&self, flags: u8) -> Option<u32> {
        match flags & (FLAG_THUMB | FLAG_ARM) {
            FLAG_THUMB => Some(THUMB),
            FLAG_ARM => Some(ARM),
            _ => None,
        }
    }

    fn decode(&self, bytes: &[u8], pc: u64, mode: u32) -> Decoded {
        let d = if mode == THUMB { thumb(bytes, pc) } else { arm(bytes, pc) };
        Decoded { mode_after: mode, ..d }
    }
}

fn next(len: u8) -> Decoded {
    Decoded { len, kind: Kind::Next, conditional: false, target: None, delay_slots: 0, mode_after: 0, site: "", target_mode: None, data_ref: None }
}

fn wrap(a: i64) -> u64 {
    (a as u64) & 0xFFFF_FFFF
}

/// One ARM instruction. `pc` reads as the instruction's address + 8.
fn arm(bytes: &[u8], pc: u64) -> Decoded {
    let Some(b) = bytes.get(..4) else { return next(4) };
    let w = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let d = next(4);
    let cond = w >> 28;
    // 0xF is "never" on ARMv4 (other instructions from ARMv5 on).
    if cond == 0xF {
        return d;
    }
    let conditional = cond != 0xE;
    let r15 = pc as i64 + 8;
    let (rn, rd, rm) = ((w >> 16) & 15, (w >> 12) & 15, w & 15);
    let bit = |n: u32| (w >> n) & 1 == 1;
    let jump = |target: Option<u64>| Decoded { kind: Kind::Jump, conditional, target, ..d };
    let ret = Decoded { kind: Kind::Return, conditional, ..d };

    // BX Rm
    if w & 0x0FFF_FFF0 == 0x012F_FF10 {
        return match rm {
            14 => ret,
            15 => Decoded { site: "BX", ..jump(Some(wrap(r15))) },
            _ => jump(None),
        };
    }
    match (w >> 25) & 7 {
        // B, BL
        0b101 => {
            let target = wrap(r15 + ((((w & 0x00FF_FFFF) << 8) as i32 >> 6) as i64));
            if bit(24) {
                Decoded { kind: Kind::Call, target: Some(target), site: "BL", ..d }
            } else {
                Decoded { site: "B", ..jump(Some(target)) }
            }
        }
        // Multiplies, swaps and halfword transfers share this space with data processing.
        0b000 if w & 0x90 == 0x90 => {
            let sh = (w >> 5) & 3;
            // LDRH, LDRSB, LDRSH from [pc, #imm]: a literal.
            if bit(20) && sh != 0 && rn == 15 && bit(22) && bit(24) && !bit(21) {
                let imm = (((w >> 4) & 0xF0) | (w & 0xF)) as i64;
                let addr = wrap(if bit(23) { r15 + imm } else { r15 - imm });
                return Decoded { data_ref: Some((addr, if sh == 2 { 1 } else { 2 })), ..d };
            }
            d
        }
        0b000 | 0b001 => {
            let op = (w >> 21) & 15;
            // TST, TEQ, CMP, CMN (and MRS, MSR) don't write Rd.
            if (8..=11).contains(&op) || rd != 15 {
                return d;
            }
            let imm = bit(25);
            match op {
                // MOV pc, lr; MOVS pc, lr
                0b1101 if !imm && w & 0xFFF == 14 => ret,
                // SUBS pc, lr, #n: the exception return
                0b0010 if imm && bit(20) && rn == 14 => ret,
                _ => jump(None),
            }
        }
        0b010 | 0b011 => {
            // Bit 4 with a register offset is undefined.
            if bit(25) && bit(4) {
                return d;
            }
            let (load, byte, pre, up, wb, reg) = (bit(20), bit(22), bit(24), bit(23), bit(21), bit(25));
            let imm = (w & 0xFFF) as i64;
            let literal = (load && rn == 15 && !reg && pre && !wb).then(|| (wrap(if up { r15 + imm } else { r15 - imm }), if byte { 1 } else { 4 }));
            let d = Decoded { data_ref: literal, ..d };
            // LDRT (post-indexed with W) and byte loads into the PC are unpredictable.
            if !load || rd != 15 || byte || (!pre && wb) {
                return d;
            }
            // LDR pc, [sp], #4 pops the return address; anything else loading the PC jumps.
            if rn == 13 && !pre && up && !reg && imm == 4 {
                Decoded { kind: Kind::Return, conditional, ..d }
            } else {
                Decoded { kind: Kind::Jump, conditional, ..d }
            }
        }
        // LDM with the PC in the list
        0b100 if bit(20) && bit(15) => {
            if rn == 13 {
                ret
            } else {
                jump(None)
            }
        }
        // SWI: a call into the BIOS, which comes back.
        0b111 if bit(24) => Decoded { kind: Kind::Call, conditional, site: "SWI", ..d },
        _ => d,
    }
}

/// One Thumb instruction (two halfwords for `BL`). `pc` reads as the address + 4.
fn thumb(bytes: &[u8], pc: u64) -> Decoded {
    let Some(b) = bytes.get(..2) else { return next(2) };
    let h = u16::from_le_bytes([b[0], b[1]]) as u32;
    let d = next(2);
    let r15 = pc as i64 + 4;
    let sext = |v: u32, bits: u32| ((v << (32 - bits)) as i32 >> (32 - bits)) as i64;
    match h >> 11 {
        // LDR Rd, [pc, #imm]: a literal.
        0b01001 => Decoded { data_ref: Some((wrap((r15 & !3) + (h & 0xFF) as i64 * 4), 4)), ..d },
        // B
        0b11100 => Decoded { kind: Kind::Jump, target: Some(wrap(r15 + (sext(h & 0x7FF, 11) << 1))), site: "B", ..d },
        // BL: the high part of the offset, then the low part and the call.
        0b11110 => match bytes.get(2..4).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32) {
            Some(lo) if lo >> 11 == 0b11111 => {
                let target = wrap(r15 + (sext(h & 0x7FF, 11) << 12) + (((lo & 0x7FF) as i64) << 1));
                Decoded { len: 4, kind: Kind::Call, target: Some(target), site: "BL", ..d }
            }
            _ => d,
        },
        // Bcc, SWI
        0b11010 | 0b11011 => match (h >> 8) & 15 {
            0xF => Decoded { kind: Kind::Call, site: "SWI", ..d },
            0xE => d,
            _ => Decoded { kind: Kind::Jump, conditional: true, target: Some(wrap(r15 + (sext(h & 0xFF, 8) << 1))), ..d },
        },
        // POP {..., pc}
        0b10111 if h & 0xFF00 == 0xBD00 => Decoded { kind: Kind::Return, ..d },
        // High register operations and BX
        0b01000 if h & 0xFC00 == 0x4400 => {
            let op = (h >> 8) & 3;
            let rs = (h >> 3) & 15;
            let rd = (h & 7) | ((h >> 4) & 8);
            match op {
                // BX (with H1 set it's BLX, from ARMv5)
                3 if h & 0x80 != 0 => d,
                3 => match rs {
                    14 => Decoded { kind: Kind::Return, ..d },
                    // bx pc: on to the next word, in ARM mode.
                    15 => Decoded { kind: Kind::Jump, target: Some(wrap(r15 & !3)), target_mode: Some(ARM), site: "BX", ..d },
                    _ => Decoded { kind: Kind::Jump, ..d },
                },
                // MOV pc, lr returns; other writes to the PC (ADD pc, Rm jump tables) jump.
                2 if rd == 15 && rs == 14 => Decoded { kind: Kind::Return, ..d },
                0 | 2 if rd == 15 => Decoded { kind: Kind::Jump, ..d },
                _ => d,
            }
        }
        _ => d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(w: u32, pc: u64) -> Decoded {
        Armv4t.decode(&w.to_le_bytes(), pc, ARM)
    }

    fn t(h: &[u16], pc: u64) -> Decoded {
        let bytes: Vec<u8> = h.iter().flat_map(|h| h.to_le_bytes()).collect();
        Armv4t.decode(&bytes, pc, THUMB)
    }

    #[test]
    fn arm_kinds() {
        let pc = 0x0800_0100;
        let k = |d: Decoded| (d.kind, d.conditional, d.target);
        assert_eq!(k(a(0xEAFF_FFFE, pc)), (Kind::Jump, false, Some(pc))); // b .
        assert_eq!(k(a(0x1A00_0001, pc)), (Kind::Jump, true, Some(pc + 12))); // bne +4
        assert_eq!(k(a(0xEB00_0010, pc)), (Kind::Call, false, Some(pc + 72))); // bl
        assert_eq!(k(a(0xE12F_FF1E, pc)), (Kind::Return, false, None)); // bx lr
        assert_eq!(k(a(0x012F_FF1E, pc)), (Kind::Return, true, None)); // bxeq lr
        assert_eq!(k(a(0xE12F_FF13, pc)), (Kind::Jump, false, None)); // bx r3
        assert_eq!(k(a(0xE1A0_F00E, pc)), (Kind::Return, false, None)); // mov pc, lr
        assert_eq!(k(a(0xE25E_F004, pc)), (Kind::Return, false, None)); // subs pc, lr, #4
        assert_eq!(k(a(0xE49D_F004, pc)), (Kind::Return, false, None)); // ldr pc, [sp], #4
        assert_eq!(k(a(0xE8BD_8010, pc)), (Kind::Return, false, None)); // pop {r4, pc}
        assert_eq!(k(a(0xE890_8002, pc)), (Kind::Jump, false, None)); // ldm r0, {r1, pc}
        assert_eq!(k(a(0xE08F_F100, pc)), (Kind::Jump, false, None)); // add pc, pc, r0, lsl #2
        assert_eq!(k(a(0xE15F_000E, pc)), (Kind::Next, false, None)); // cmp pc, lr
        assert_eq!(k(a(0xEF00_0005, pc)), (Kind::Call, false, None)); // svc #5
        assert_eq!(a(0xE59F_000C, pc).data_ref, Some((pc + 20, 4))); // ldr r0, [pc, #12]
        assert_eq!(a(0xE51F_F004, pc).data_ref, Some((pc + 4, 4))); // ldr pc, [pc, #-4]
        assert_eq!(a(0xE1DF_00B4, pc).data_ref, Some((pc + 12, 2))); // ldrh r0, [pc, #4]
        assert_eq!(a(0xE59F_000C, pc).len, 4);
    }

    #[test]
    fn thumb_kinds() {
        let pc = 0x0800_0100;
        let k = |d: Decoded| (d.len, d.kind, d.conditional, d.target);
        assert_eq!(k(t(&[0xE7FE], pc)), (2, Kind::Jump, false, Some(pc))); // b .
        assert_eq!(k(t(&[0xD001], pc)), (2, Kind::Jump, true, Some(pc + 6))); // beq +2
        assert_eq!(k(t(&[0x4770], pc)), (2, Kind::Return, false, None)); // bx lr
        assert_eq!(k(t(&[0x4718], pc)), (2, Kind::Jump, false, None)); // bx r3
        assert_eq!(k(t(&[0xBD10], pc)), (2, Kind::Return, false, None)); // pop {r4, pc}
        assert_eq!(k(t(&[0x46F7], pc)), (2, Kind::Return, false, None)); // mov pc, lr
        assert_eq!(k(t(&[0x4487], pc)), (2, Kind::Jump, false, None)); // add pc, r0
        assert_eq!(k(t(&[0xF000, 0xF810], pc)), (4, Kind::Call, false, Some(pc + 36))); // bl
        assert_eq!(k(t(&[0xF7FF, 0xFFFE], pc)), (4, Kind::Call, false, Some(pc))); // bl .
        assert_eq!(k(t(&[0xDF05], pc)), (2, Kind::Call, false, None)); // svc #5
        assert_eq!(k(t(&[0xF000, 0x2000], pc)), (2, Kind::Next, false, None)); // a lone BL half
        let bxpc = t(&[0x4778], pc + 2);
        assert_eq!((bxpc.kind, bxpc.target, bxpc.target_mode), (Kind::Jump, Some(pc + 4), Some(ARM)));
        assert_eq!(t(&[0x4803], pc + 2).data_ref, Some((pc + 16, 4))); // ldr r0, [pc, #12]
        assert_eq!(t(&[0x4803], pc).mode_after, THUMB);
    }
}
