//! The ARMv4T decoder (GBA), from the `dj-decoder` crate, and tests of the sweep on ARM
//! and Thumb code.

pub use dj_decoder::isa::arm::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Function;
    use crate::decode::analyze;

    fn le16(h: &[u16]) -> Vec<u8> {
        h.iter().flat_map(|h| h.to_le_bytes()).collect()
    }

    #[test]
    fn thumb_function_with_a_literal_pool() {
        // 08000000 (Thumb): push {lr}; ldr r0, [pc, #12]; cmp r0, #0; beq 0C; bl 0800002C;
        //   0C: pop {pc}; a halfword of padding; 10: .word 0x08001234 (the pool); 14: nop,
        //   code again.
        let mut code = le16(&[0xB500, 0x4803, 0x2800, 0xD001, 0xF000, 0xF810, 0xBD00]);
        code.extend([0, 0]); // pad to the word the literal sits at
        code.extend(0x0800_1234u32.to_le_bytes());
        code.extend(le16(&[0x46C0]));
        let f = Function { name: "f".into(), region: "ROM".into(), offset: 0, size: code.len() as u64, cpu_addr: Some(0x0800_0000), file_offset: None, mode: THUMB };
        let a = &analyze(&Armv4t, std::slice::from_ref(&f), &code, &[], |f| Some(f.offset))[0];
        let at: Vec<u64> = a.instructions.iter().map(|i| i.offset).collect();
        // ldr r0, [pc, #12] at 2 reads ((2 + 4) & !3) + 12 = 0x10: the pool isn't code.
        assert_eq!(a.instructions.iter().map(|i| i.len).collect::<Vec<_>>(), vec![2, 2, 2, 2, 4, 2, 2, 2]);
        assert!(!at.contains(&0x10) && !at.contains(&0x12), "{at:x?}");
        assert_eq!(a.blocks.iter().map(|b| b.offset).collect::<Vec<_>>(), vec![0, 8, 0xC, 0xE, 0x14]);
        assert_eq!(a.calls.iter().map(|c| (c.at, c.target, c.kind)).collect::<Vec<_>>(), vec![(8, 0x0800_002C, "BL")]);
    }

    #[test]
    fn thumb_to_arm_through_bx_pc() {
        // 08000000 (Thumb): bx pc; nop; then ARM at 08000004: mov r0, #1; bx lr
        let mut code = le16(&[0x4778, 0x46C0]);
        code.extend(0xE3A0_0001u32.to_le_bytes());
        code.extend(0xE12F_FF1Eu32.to_le_bytes());
        let f = Function { name: "f".into(), region: "ROM".into(), offset: 0, size: 12, cpu_addr: Some(0x0800_0000), file_offset: None, mode: THUMB };
        let a = &analyze(&Armv4t, std::slice::from_ref(&f), &code, &[], |f| Some(f.offset))[0];
        assert_eq!(a.instructions.iter().map(|i| (i.offset, i.len)).collect::<Vec<_>>(), vec![(0, 2), (2, 2), (4, 4), (8, 4)]);
        assert_eq!(a.blocks.iter().map(|b| b.offset).collect::<Vec<_>>(), vec![0, 2, 4]);
        assert_eq!(a.successors, vec![vec![4], vec![4], vec![]]);
    }

    #[test]
    fn arm_function_with_a_pool_and_a_conditional_return() {
        // 08000000 (ARM): ldr r0, [pc, #8]; cmp r0, #0; bxeq lr; b 08000000; 10: .word pool
        let words = [0xE59F_0008u32, 0xE350_0000, 0x012F_FF1E, 0xEAFF_FFFB, 0x1234_5678];
        let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let f = Function { name: "f".into(), region: "ROM".into(), offset: 0, size: 20, cpu_addr: Some(0x0800_0000), file_offset: None, mode: 0 };
        let a = &analyze(&Armv4t, std::slice::from_ref(&f), &code, &[], |f| Some(f.offset))[0];
        assert_eq!(a.instructions.len(), 4);
        assert_eq!(a.blocks.iter().map(|b| b.offset).collect::<Vec<_>>(), vec![0, 12]);
        // The conditional return falls through; the branch goes back to the start.
        assert_eq!(a.successors, vec![vec![12], vec![0]]);
    }
}
