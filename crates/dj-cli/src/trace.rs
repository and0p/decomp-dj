//! Instruction traces from the connector's trace hook, as text (docs/mcp.md#traces).

use dj_core::libretro::{TraceEvent, TRACE_EXEC, TRACE_READ, TRACE_WRITE};

#[derive(Clone, Copy)]
enum Mode {
    Imp,
    Acc,
    Imm,
    Zp,
    Zpx,
    Zpy,
    Abs,
    Abx,
    Aby,
    Ind,
    Izx,
    Izy,
    Rel,
}

/// Mnemonic and addressing mode of each 6502 opcode, the stable undocumented ones
/// included (lower case).
fn op_6502(op: u8) -> (&'static str, Mode) {
    use Mode::*;
    const T: [(&str, Mode); 256] = [
        ("BRK", Imp), ("ORA", Izx), ("jam", Imp), ("slo", Izx), ("nop", Zp), ("ORA", Zp), ("ASL", Zp), ("slo", Zp),
        ("PHP", Imp), ("ORA", Imm), ("ASL", Acc), ("anc", Imm), ("nop", Abs), ("ORA", Abs), ("ASL", Abs), ("slo", Abs),
        ("BPL", Rel), ("ORA", Izy), ("jam", Imp), ("slo", Izy), ("nop", Zpx), ("ORA", Zpx), ("ASL", Zpx), ("slo", Zpx),
        ("CLC", Imp), ("ORA", Aby), ("nop", Imp), ("slo", Aby), ("nop", Abx), ("ORA", Abx), ("ASL", Abx), ("slo", Abx),
        ("JSR", Abs), ("AND", Izx), ("jam", Imp), ("rla", Izx), ("BIT", Zp), ("AND", Zp), ("ROL", Zp), ("rla", Zp),
        ("PLP", Imp), ("AND", Imm), ("ROL", Acc), ("anc", Imm), ("BIT", Abs), ("AND", Abs), ("ROL", Abs), ("rla", Abs),
        ("BMI", Rel), ("AND", Izy), ("jam", Imp), ("rla", Izy), ("nop", Zpx), ("AND", Zpx), ("ROL", Zpx), ("rla", Zpx),
        ("SEC", Imp), ("AND", Aby), ("nop", Imp), ("rla", Aby), ("nop", Abx), ("AND", Abx), ("ROL", Abx), ("rla", Abx),
        ("RTI", Imp), ("EOR", Izx), ("jam", Imp), ("sre", Izx), ("nop", Zp), ("EOR", Zp), ("LSR", Zp), ("sre", Zp),
        ("PHA", Imp), ("EOR", Imm), ("LSR", Acc), ("alr", Imm), ("JMP", Abs), ("EOR", Abs), ("LSR", Abs), ("sre", Abs),
        ("BVC", Rel), ("EOR", Izy), ("jam", Imp), ("sre", Izy), ("nop", Zpx), ("EOR", Zpx), ("LSR", Zpx), ("sre", Zpx),
        ("CLI", Imp), ("EOR", Aby), ("nop", Imp), ("sre", Aby), ("nop", Abx), ("EOR", Abx), ("LSR", Abx), ("sre", Abx),
        ("RTS", Imp), ("ADC", Izx), ("jam", Imp), ("rra", Izx), ("nop", Zp), ("ADC", Zp), ("ROR", Zp), ("rra", Zp),
        ("PLA", Imp), ("ADC", Imm), ("ROR", Acc), ("arr", Imm), ("JMP", Ind), ("ADC", Abs), ("ROR", Abs), ("rra", Abs),
        ("BVS", Rel), ("ADC", Izy), ("jam", Imp), ("rra", Izy), ("nop", Zpx), ("ADC", Zpx), ("ROR", Zpx), ("rra", Zpx),
        ("SEI", Imp), ("ADC", Aby), ("nop", Imp), ("rra", Aby), ("nop", Abx), ("ADC", Abx), ("ROR", Abx), ("rra", Abx),
        ("nop", Imm), ("STA", Izx), ("nop", Imm), ("sax", Izx), ("STY", Zp), ("STA", Zp), ("STX", Zp), ("sax", Zp),
        ("DEY", Imp), ("nop", Imm), ("TXA", Imp), ("xaa", Imm), ("STY", Abs), ("STA", Abs), ("STX", Abs), ("sax", Abs),
        ("BCC", Rel), ("STA", Izy), ("jam", Imp), ("sha", Izy), ("STY", Zpx), ("STA", Zpx), ("STX", Zpy), ("sax", Zpy),
        ("TYA", Imp), ("STA", Aby), ("TXS", Imp), ("tas", Aby), ("shy", Abx), ("STA", Abx), ("shx", Aby), ("sha", Aby),
        ("LDY", Imm), ("LDA", Izx), ("LDX", Imm), ("lax", Izx), ("LDY", Zp), ("LDA", Zp), ("LDX", Zp), ("lax", Zp),
        ("TAY", Imp), ("LDA", Imm), ("TAX", Imp), ("lax", Imm), ("LDY", Abs), ("LDA", Abs), ("LDX", Abs), ("lax", Abs),
        ("BCS", Rel), ("LDA", Izy), ("jam", Imp), ("lax", Izy), ("LDY", Zpx), ("LDA", Zpx), ("LDX", Zpy), ("lax", Zpy),
        ("CLV", Imp), ("LDA", Aby), ("TSX", Imp), ("las", Aby), ("LDY", Abx), ("LDA", Abx), ("LDX", Aby), ("lax", Aby),
        ("CPY", Imm), ("CMP", Izx), ("nop", Imm), ("dcp", Izx), ("CPY", Zp), ("CMP", Zp), ("DEC", Zp), ("dcp", Zp),
        ("INY", Imp), ("CMP", Imm), ("DEX", Imp), ("axs", Imm), ("CPY", Abs), ("CMP", Abs), ("DEC", Abs), ("dcp", Abs),
        ("BNE", Rel), ("CMP", Izy), ("jam", Imp), ("dcp", Izy), ("nop", Zpx), ("CMP", Zpx), ("DEC", Zpx), ("dcp", Zpx),
        ("CLD", Imp), ("CMP", Aby), ("nop", Imp), ("dcp", Aby), ("nop", Abx), ("CMP", Abx), ("DEC", Abx), ("dcp", Abx),
        ("CPX", Imm), ("SBC", Izx), ("nop", Imm), ("isc", Izx), ("CPX", Zp), ("SBC", Zp), ("INC", Zp), ("isc", Zp),
        ("INX", Imp), ("SBC", Imm), ("NOP", Imp), ("sbc", Imm), ("CPX", Abs), ("SBC", Abs), ("INC", Abs), ("isc", Abs),
        ("BEQ", Rel), ("SBC", Izy), ("jam", Imp), ("isc", Izy), ("nop", Zpx), ("SBC", Zpx), ("INC", Zpx), ("isc", Zpx),
        ("SED", Imp), ("SBC", Aby), ("nop", Imp), ("isc", Aby), ("nop", Abx), ("SBC", Abx), ("INC", Abx), ("isc", Abx),
    ];
    T[op as usize]
}

/// `LDA $0200,X` for an opcode at CPU address `pc` and its operand bytes. `name` turns a
/// CPU address into a label when one is known.
pub fn disasm_6502(pc: u16, op: u8, operands: &[u8], name: &dyn Fn(u16) -> Option<String>) -> String {
    let (m, mode) = op_6502(op);
    let b = |i: usize| operands.get(i).copied().unwrap_or(0);
    let w = u16::from_le_bytes([b(0), b(1)]);
    let lab = |a: u16, digits: usize| name(a).unwrap_or_else(|| if digits == 2 { format!("${a:02X}") } else { format!("${a:04X}") });
    match mode {
        Mode::Imp => m.to_string(),
        Mode::Acc => format!("{m} A"),
        Mode::Imm => format!("{m} #${:02X}", b(0)),
        Mode::Zp => format!("{m} {}", lab(b(0) as u16, 2)),
        Mode::Zpx => format!("{m} {},X", lab(b(0) as u16, 2)),
        Mode::Zpy => format!("{m} {},Y", lab(b(0) as u16, 2)),
        Mode::Abs => format!("{m} {}", lab(w, 4)),
        Mode::Abx => format!("{m} {},X", lab(w, 4)),
        Mode::Aby => format!("{m} {},Y", lab(w, 4)),
        Mode::Ind => format!("{m} ({})", lab(w, 4)),
        Mode::Izx => format!("{m} (${:02X},X)", b(0)),
        Mode::Izy => format!("{m} (${:02X}),Y", b(0)),
        Mode::Rel => format!("{m} {}", lab(pc.wrapping_add(2).wrapping_add(b(0) as i8 as u16), 4)),
    }
}

const MIPS_REGS: [&str; 32] = [
    "zero", "at", "v0", "v1", "a0", "a1", "a2", "a3", "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "t8", "t9", "k0", "k1", "gp", "sp",
    "fp", "ra",
];

/// `sw ra, 0x14(sp)` for the MIPS instruction word `w` at CPU address `pc`: the loads,
/// stores, jumps, branches and integer arithmetic a trace is read for. Anything else (the
/// coprocessors, mostly) is `.word 0x...`. `name` turns a CPU address into a label.
pub fn disasm_mips(pc: u32, w: u32, name: &dyn Fn(u32) -> Option<String>) -> String {
    let (op, rs, rt, rd, sa, func) = (w >> 26, (w >> 21 & 31) as usize, (w >> 16 & 31) as usize, (w >> 11 & 31) as usize, w >> 6 & 31, w & 63);
    let (r, imm) = (MIPS_REGS, w as u16);
    let simm = imm as i16 as i32;
    let hex = |v: i32| if v < 0 { format!("-0x{:X}", -(v as i64)) } else { format!("0x{v:X}") };
    let label = |a: u32| name(a).unwrap_or_else(|| format!("0x{a:08X}"));
    let branch = pc.wrapping_add(4).wrapping_add((simm << 2) as u32);
    let mem = |m: &str| format!("{m} {}, {}({})", r[rt], hex(simm), r[rs]);
    if w == 0 {
        return "nop".into();
    }
    match op {
        0 => match func {
            0 | 2 | 3 => format!("{} {}, {}, {sa}", ["sll", "", "srl", "sra"][func as usize], r[rd], r[rt]),
            4 | 6 | 7 => format!("{} {}, {}, {}", ["sllv", "", "srlv", "srav"][func as usize - 4], r[rd], r[rt], r[rs]),
            8 => format!("jr {}", r[rs]),
            9 => format!("jalr {}", r[rs]),
            12 => "syscall".into(),
            13 => "break".into(),
            16 | 18 => format!("{} {}", if func == 16 { "mfhi" } else { "mflo" }, r[rd]),
            17 | 19 => format!("{} {}", if func == 17 { "mthi" } else { "mtlo" }, r[rs]),
            24..=31 => format!("{} {}, {}", ["mult", "multu", "div", "divu", "dmult", "dmultu", "ddiv", "ddivu"][func as usize - 24], r[rs], r[rt]),
            32..=47 if !matches!(func, 40 | 41) => {
                let m = ["add", "addu", "sub", "subu", "and", "or", "xor", "nor", "", "", "slt", "sltu", "dadd", "daddu", "dsub", "dsubu"][func as usize - 32];
                format!("{m} {}, {}, {}", r[rd], r[rs], r[rt])
            }
            _ => format!(".word 0x{w:08X}"),
        },
        1 => match rt {
            0 | 1 | 16 | 17 => format!("{} {}, {}", ["bltz", "bgez", "bltzal", "bgezal"][(rt & 1) + (rt >> 4) * 2], r[rs], label(branch)),
            _ => format!(".word 0x{w:08X}"),
        },
        2 | 3 => format!("{} {}", if op == 2 { "j" } else { "jal" }, label(pc.wrapping_add(4) & 0xF000_0000 | (w & 0x03FF_FFFF) << 2)),
        4 | 5 | 20 | 21 => format!("{} {}, {}, {}", ["beq", "bne", "beql", "bnel"][(op & 1) as usize + (op >> 4) as usize * 2], r[rs], r[rt], label(branch)),
        6 | 7 | 22 | 23 => format!("{} {}, {}", ["blez", "bgtz", "blezl", "bgtzl"][(op & 1) as usize + (op >> 4) as usize * 2], r[rs], label(branch)),
        8 | 9 | 10 | 11 | 24 | 25 => format!("{} {}, {}, {}", ["addi", "addiu", "slti", "sltiu", "daddi", "daddiu"][if op >= 24 { op as usize - 20 } else { op as usize - 8 }], r[rt], r[rs], hex(simm)),
        12..=14 => format!("{} {}, {}, 0x{imm:X}", ["andi", "ori", "xori"][op as usize - 12], r[rt], r[rs]),
        15 => format!("lui {}, 0x{imm:X}", r[rt]),
        32 => mem("lb"),
        33 => mem("lh"),
        34 => mem("lwl"),
        35 => mem("lw"),
        36 => mem("lbu"),
        37 => mem("lhu"),
        38 => mem("lwr"),
        39 => mem("lwu"),
        40 => mem("sb"),
        41 => mem("sh"),
        42 => mem("swl"),
        43 => mem("sw"),
        46 => mem("swr"),
        55 => mem("ld"),
        63 => mem("sd"),
        49 | 53 | 57 | 61 => format!("{} f{rt}, {}({})", ["lwc1", "ldc1", "swc1", "sdc1"][(op as usize - 49) / 4], hex(simm), r[rs]),
        _ => format!(".word 0x{w:08X}"),
    }
}

/// The instruction of an execute event, as text, for the instruction set `cpu` (a
/// connector region's: "mips3", else the 6502).
pub fn disasm(cpu: &str, e: &TraceEvent, name: &dyn Fn(u32) -> Option<String>) -> String {
    if cpu == "mips3" {
        let w = u32::from_be_bytes([e.value, e.operands[0], e.operands[1], e.operands[2]]);
        disasm_mips(e.address(), w, name)
    } else {
        disasm_6502(e.addr, e.value, &e.operands[..e.operand_count as usize], &|a| name(a as u32))
    }
}

/// A CPU address as text: `$C123` on the 6502, `$80114514` with 32-bit addresses.
pub fn addr_text(cpu: &str, a: u32) -> String {
    if cpu == "mips3" {
        format!("${a:08X}")
    } else {
        format!("${a:04X}")
    }
}

/// Instructions in a trace, each with the data reads and writes it made.
pub struct Step<'a> {
    pub exec: &'a TraceEvent,
    pub accesses: Vec<&'a TraceEvent>,
}

pub fn group(events: &[TraceEvent]) -> Vec<Step<'_>> {
    let mut out: Vec<Step> = vec![];
    for e in events {
        if e.kind == TRACE_EXEC {
            out.push(Step { exec: e, accesses: vec![] });
        } else if let Some(last) = out.last_mut() {
            last.accesses.push(e);
        }
    }
    out
}

/// One line per instruction: `$C123 LDA $0200,X  A:00 X:01 Y:00 S:FD P:24  r$0201=05 w$0300=05`
/// on the 6502. With `cpu` "mips3" there are no registers (the connector doesn't trace
/// them): `$80005BD0 jr ra  r$8009EDA4=00`.
pub fn line(s: &Step, cpu: &str, name: &dyn Fn(u32) -> Option<String>) -> String {
    let e = s.exec;
    let mut t = if cpu == "mips3" {
        format!("{} {:<26}", addr_text(cpu, e.address()), disasm(cpu, e, name))
    } else {
        format!("{} {:<18} A:{:02X} X:{:02X} Y:{:02X} S:{:02X} P:{:02X}", addr_text(cpu, e.address()), disasm(cpu, e, name), e.a, e.x, e.y, e.sp, e.p)
    };
    for a in &s.accesses {
        let k = match a.kind {
            TRACE_READ => 'r',
            TRACE_WRITE => 'w',
            _ => '?',
        };
        t.push_str(&format!(" {k}{}={:02X}", addr_text(cpu, a.address()), a.value));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disassembles() {
        let none = |_: u16| None;
        assert_eq!(disasm_6502(0xC000, 0xA9, &[0x05], &none), "LDA #$05");
        assert_eq!(disasm_6502(0xC000, 0xBD, &[0x00, 0x02], &none), "LDA $0200,X");
        assert_eq!(disasm_6502(0xC000, 0xD0, &[0xFE], &none), "BNE $C000");
        assert_eq!(disasm_6502(0xC000, 0x20, &[0x34, 0x12], &|a| (a == 0x1234).then(|| "f".into())), "JSR f");
        assert_eq!(disasm_6502(0xC000, 0xB1, &[0x10], &none), "LDA ($10),Y");
        assert_eq!(disasm_6502(0xC000, 0x0A, &[], &none), "ASL A");
    }

    #[test]
    fn disassembles_mips() {
        let none = |_: u32| None;
        assert_eq!(disasm_mips(0x8000_5BD0, 0x03E0_0008, &none), "jr ra");
        assert_eq!(disasm_mips(0x8000_5BD4, 0x27BD_0050, &none), "addiu sp, sp, 0x50");
        assert_eq!(disasm_mips(0x800D_9DFC, 0x0C04_5145, &none), "jal 0x80114514");
        assert_eq!(disasm_mips(0x800D_9DFC, 0x0C04_5145, &|a| (a == 0x8011_4514).then(|| "match_tick".into())), "jal match_tick");
        assert_eq!(disasm_mips(0x8000_0000, 0xAFBF_0014, &none), "sw ra, 0x14(sp)");
        assert_eq!(disasm_mips(0x8000_0000, 0xA044_FFFF, &none), "sb a0, -0x1(v0)");
        assert_eq!(disasm_mips(0x8000_0000, 0x1440_FFFE, &none), "bne v0, zero, 0x7FFFFFFC");
        assert_eq!(disasm_mips(0x8000_0000, 0x0082_1021, &none), "addu v0, a0, v0");
        assert_eq!(disasm_mips(0x8000_0000, 0, &none), "nop");
        assert_eq!(disasm_mips(0x8000_0000, 0x4600_0000, &none), ".word 0x46000000");
        assert_eq!(std::mem::size_of::<TraceEvent>(), 40);
    }
}
