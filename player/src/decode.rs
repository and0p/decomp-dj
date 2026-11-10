//! Instruction decoders, one per instruction set, behind one interface, and the sweep that
//! splits symbol-map functions into instructions and basic blocks with any of them
//! (.agents/DECODER_PLAN.md).
//!
//! A decoder only answers "what is this one instruction?": its length, how control leaves
//! it, and its delay slots. Everything built from those answers (blocks, successors, call
//! sites) is worked out here once, for every instruction set.

use crate::analysis::{AnalyzedFunction, Block, CallSite, Function, Instruction};

/// How control leaves an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// On to the next instruction.
    Next,
    /// To `target` (None: indirect, such as a jump table). A conditional jump is a branch.
    Jump,
    /// To `target` (None: indirect), coming back after the instruction and its delay slots.
    Call,
    /// Back to the caller.
    Return,
    /// Execution doesn't go on (BRK, KIL, BREAK).
    Stop,
}

/// One decoded instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Bytes. 0: not an instruction.
    pub len: u8,
    pub kind: Kind,
    /// Whether execution may also go on to the next instruction (a branch, a conditional
    /// return or call). Calls always come back, so this only matters for the others.
    pub conditional: bool,
    /// The CPU address control goes to, for jumps and calls whose target the bytes say.
    pub target: Option<u64>,
    /// Instructions that run before control moves (MIPS branches and jumps: 1).
    pub delay_slots: u8,
    /// The mode the next instruction runs in (65816 REP/SEP, ARM/Thumb); 0 where the
    /// instruction set has none.
    pub mode_after: u32,
    /// How a call site through this instruction is listed ("JSR", "JAL", ...), for jumps
    /// and calls.
    pub site: &'static str,
    /// The mode code runs in at `target`, when the jump changes it (ARM `bx pc` from Thumb).
    /// None: the same mode.
    pub target_mode: Option<u32>,
    /// Data the instruction reads at a CPU address its bytes give (a PC-relative literal on
    /// ARM): the address and how many bytes. The sweep doesn't read those bytes as code.
    pub data_ref: Option<(u64, u8)>,
}

impl Decoded {
    fn next(len: u8) -> Decoded {
        Decoded { len, kind: Kind::Next, conditional: false, target: None, delay_slots: 0, mode_after: 0, site: "", target_mode: None, data_ref: None }
    }

    /// Whether this instruction ends a basic block (after its delay slots). Calls don't:
    /// control comes back.
    pub fn ends_block(&self) -> bool {
        !matches!(self.kind, Kind::Next | Kind::Call)
    }
}

/// An instruction set the player can split into instructions and blocks.
pub trait Decoder: Sync {
    /// The name connectors report for it (`dj_region.cpu`): "6502", "mips3".
    fn cpu(&self) -> &'static str;
    /// The most bytes one instruction takes; the sweep offers this many where it can.
    fn max_len(&self) -> usize;
    /// Every instruction this long, for fixed-width instruction sets. Lets callers skip
    /// decoding lengths.
    fn fixed_len(&self) -> Option<u8> {
        None
    }
    /// Instructions in `mode` start only at multiples of this many bytes (from the
    /// function's start).
    fn alignment(&self, _mode: u32) -> u8 {
        1
    }
    /// The mode an opcode ran in, from the connector's mode bits on its first byte
    /// (`0x04`-`0x20`, defined per instruction set in docs/connectors.md). None: the bits
    /// don't say (no mode bits, or more than one mode).
    fn mode_from_flags(&self, _flags: u8) -> Option<u32> {
        None
    }
    /// Decodes the instruction at the start of `bytes` (in the ROM file's order), which runs
    /// at CPU address `pc` in `mode`. `bytes` may be shorter than `max_len` at the end of the
    /// code; a length past it means the instruction doesn't fit.
    fn decode(&self, bytes: &[u8], pc: u64, mode: u32) -> Decoded;
}

/// The built-in decoder for an instruction set, by the name connectors report.
pub fn decoder_for(cpu: &str) -> Option<&'static dyn Decoder> {
    BUILT_IN.iter().copied().find(|d| d.cpu() == cpu)
}

static BUILT_IN: [&dyn Decoder; 3] = [&Mos6502, &Mips3, &crate::arm::Armv4t];

// ---- 6502 ---------------------------------------------------------------------------

/// The 6502 (NES 2A03 included), with the stable undocumented opcodes.
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

// ---- the sweep ----------------------------------------------------------------------

/// Splits each function into instructions and basic blocks with `dec`, by a linear sweep
/// from its start. `code` holds the bytes and `base(f)` says where `f.offset` lies in it
/// (None: no bytes, the function is skipped). `data` (region offsets, inclusive, such as
/// jump tables from annotations.toml) isn't read as code: the sweep starts again after each
/// range, with a new block there.
///
/// A block ends after an instruction that leaves the straight line (and its delay slots),
/// and one starts at every jump or branch target inside the function. Calls don't end
/// blocks. Literals an instruction loads from further on in the function (`data_ref`) are
/// skipped like data. The mode starts as the function's `mode`, follows each instruction's
/// `mode_after`, and changes at a target reached with another mode (`target_mode`).
pub fn analyze(dec: &dyn Decoder, funcs: &[Function], code: &[u8], data: &[(u64, u64)], base: impl Fn(&Function) -> Option<u64>) -> Vec<AnalyzedFunction> {
    let mut data: Vec<(u64, u64)> = data.to_vec();
    data.sort();
    funcs.iter().filter_map(|f| base(f).map(|b| sweep(dec, f, code, b, &data))).collect()
}

fn sweep(dec: &dyn Decoder, f: &Function, code: &[u8], base: u64, data: &[(u64, u64)]) -> AnalyzedFunction {
    let start = f.offset;
    // Past the code's end, there's nothing to read.
    let end = (f.offset + f.size).min(f.offset + (code.len() as u64).saturating_sub(base));
    let mut mode = f.mode;
    let cpu_of = |o: u64| f.cpu_addr.map(|c| c + (o - start));
    let off_of_cpu = |cpu: u64| -> Option<u64> {
        let b = f.cpu_addr?;
        (cpu >= b && cpu < b + f.size).then(|| start + (cpu - b))
    };
    let align_of = |mode: u32| dec.alignment(mode).max(1) as u64;
    let align_up = |o: u64, mode: u32| start + (o - start).div_ceil(align_of(mode)) * align_of(mode);

    // The data ranges that reach into the function, by start; literals found on the way
    // join them.
    let mut data: Vec<(u64, u64)> = data.iter().filter(|(a, e)| *e >= start && *a < end).copied().collect();
    let mut d = 0; // the next data range that may still matter
    // Targets inside the function reached in another mode.
    let mut modes: std::collections::BTreeMap<u64, u32> = Default::default();

    let mut insns: Vec<Instruction> = vec![];
    let mut decoded: Vec<Decoded> = vec![];
    let mut leaders: Vec<u64> = vec![start];
    let mut off = start;
    while off < end {
        if let Some(&m) = modes.get(&off) {
            mode = m;
        }
        while d < data.len() && data[d].1 < off {
            d += 1;
        }
        if let Some(&(a, e)) = data.get(d) {
            if a <= off {
                off = align_up(e + 1, mode);
                leaders.push(off);
                continue;
            }
        }
        let at = (base + (off - start)) as usize;
        let bytes = &code[at..(at + dec.max_len()).min(code.len())];
        let ins = dec.decode(bytes, cpu_of(off).unwrap_or(0), mode);
        if ins.len == 0 {
            off += align_of(mode);
            leaders.push(off);
            continue;
        }
        let len = ins.len as u64;
        if off + len > end {
            break;
        }
        // An instruction can't run on into data: the data starts where it would be.
        if let Some(&(a, _)) = data.get(d) {
            if a < off + len {
                off = a;
                continue;
            }
        }
        insns.push(Instruction { offset: off, len: ins.len });
        decoded.push(ins);
        // A literal further on in the function: not code. One already passed stays as read.
        if let Some((lo, hi)) = ins.data_ref.and_then(|(addr, n)| Some((off_of_cpu(addr)?, n as u64))).map(|(o, n)| (o, (o + n - 1).min(end - 1))) {
            if lo >= off + len && !data.iter().any(|(a, e)| *a <= lo && hi <= *e) {
                let at = data.partition_point(|(a, _)| *a <= lo);
                data.insert(at.max(d), (lo, hi));
            }
        }
        if ins.ends_block() {
            if let Some(o) = ins.target.and_then(off_of_cpu) {
                if ins.kind == Kind::Jump {
                    leaders.push(o);
                    if let Some(m) = ins.target_mode {
                        modes.insert(o, m);
                    }
                }
            }
            // The delay slots run first; they're as wide as the instruction on every
            // instruction set that has them.
            leaders.push(off + len * (1 + ins.delay_slots as u64));
        }
        mode = ins.mode_after;
        off += len;
    }
    leaders.sort_unstable();
    leaders.dedup();

    let mut blocks: Vec<Block> = vec![];
    let mut first_insn: Vec<usize> = vec![]; // index into insns of each block's first instruction
    for (k, i) in insns.iter().enumerate() {
        if blocks.is_empty() || leaders.binary_search(&i.offset).is_ok() {
            blocks.push(Block { offset: i.offset, instructions: vec![], cpu_addr: cpu_of(i.offset) });
            first_insn.push(k);
        }
        blocks.last_mut().unwrap().instructions.push(i.offset);
    }

    let is_block = |o: u64| o < end && blocks.binary_search_by_key(&o, |b| b.offset).is_ok();
    let successors = blocks
        .iter()
        .enumerate()
        .map(|(n, b)| {
            let (lo, hi) = (first_insn[n], first_insn[n] + b.instructions.len() - 1);
            // The instruction that ends the block is its last, or the one whose delay
            // slots the last ones are. A jump in another's delay slot (undefined on MIPS)
            // leaves the first one in charge.
            let ctl = (lo..=hi).find(|&k| decoded[k].ends_block() && k + decoded[k].delay_slots as usize >= hi);
            let mut out = match ctl {
                None => vec![insns[hi].offset + insns[hi].len as u64],
                Some(k) => {
                    let (i, ins) = (&insns[k], &decoded[k]);
                    let next = i.offset + i.len as u64 * (1 + ins.delay_slots as u64);
                    let target = ins.target.and_then(off_of_cpu);
                    match ins.kind {
                        Kind::Jump => [ins.conditional.then_some(next), target].into_iter().flatten().collect(),
                        _ => ins.conditional.then_some(next).into_iter().collect(),
                    }
                }
            };
            out.retain(|o| is_block(*o));
            out.dedup();
            out
        })
        .collect();

    // Direct calls, and jumps that leave the function (tail calls), for callers lists.
    let calls = insns
        .iter()
        .zip(&decoded)
        .filter_map(|(i, ins)| {
            let t = ins.target?;
            let listed = match ins.kind {
                Kind::Call => true,
                Kind::Jump => !ins.conditional && off_of_cpu(t).is_none(),
                _ => false,
            };
            listed.then(|| CallSite { at: i.offset, target: t, kind: ins.site })
        })
        .collect();

    AnalyzedFunction { func: f.clone(), instructions: insns, blocks, successors, calls }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn func(region: &str, offset: u64, size: u64, cpu: u64, file: Option<u64>) -> Function {
        Function { name: "f".into(), region: region.into(), offset, size, cpu_addr: Some(cpu), file_offset: file, mode: 0 }
    }

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

    #[test]
    fn mips_blocks_end_after_the_delay_slot() {
        // 80001000 addiu $sp,-0x18; beq $a0,$0,+3; nop; jal 0x80002000; nop; jr $ra; addiu $sp,0x18
        let words: [u32; 7] = [0x27BD_FFE8, 0x1080_0003, 0, 0x0C00_0800, 0, 0x03E0_0008, 0x27BD_0018];
        let mut rom = vec![0xFFu8; 0x10];
        rom.extend(words.iter().flat_map(|w| w.to_be_bytes()));
        let f = func("RDRAM", 0x1000, 28, 0x8000_1000, Some(0x10));
        let a = &analyze(&Mips3, std::slice::from_ref(&f), &rom, &[], |f| f.file_offset)[0];
        assert_eq!(a.instructions.len(), 7);
        let blocks: Vec<(u64, usize)> = a.blocks.iter().map(|b| (b.offset, b.instructions.len())).collect();
        // The branch and its slot; the call, its slot (calls don't end blocks); the return.
        assert_eq!(blocks, vec![(0x1000, 3), (0x100C, 2), (0x1014, 2)]);
        assert_eq!(a.successors, vec![vec![0x100C, 0x1014], vec![0x1014], vec![]]);
        assert_eq!(a.calls, vec![CallSite { at: 0x100C, target: 0x8000_2000, kind: "JAL" }]);
        // Without a file offset there are no bytes to read; data isn't decoded.
        assert!(analyze(&Mips3, &[Function { file_offset: None, ..f.clone() }], &rom, &[], |f| f.file_offset).is_empty());
        let a = &analyze(&Mips3, std::slice::from_ref(&f), &rom, &[(0x1008, 0x100B)], |f| f.file_offset)[0];
        assert_eq!(a.instructions.len(), 6);
        // Data ending inside a word: the sweep starts again at the next whole word.
        let a = &analyze(&Mips3, std::slice::from_ref(&f), &rom, &[(0x1006, 0x1009)], |f| f.file_offset)[0];
        assert_eq!(a.instructions.iter().map(|i| i.offset).collect::<Vec<_>>(), vec![0x1000, 0x100C, 0x1010, 0x1014, 0x1018]);
    }

    #[test]
    fn break_and_eret_have_no_delay_slot() {
        // break; addiu; eret; addiu
        let words: [u32; 4] = [0x0000_000D, 0x27BD_FFE8, 0x4200_0018, 0x27BD_FFE8];
        let rom: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
        let f = func("RDRAM", 0, 16, 0x8000_0000, Some(0));
        let a = &analyze(&Mips3, &[f], &rom, &[], |f| f.file_offset)[0];
        let blocks: Vec<(u64, usize)> = a.blocks.iter().map(|b| (b.offset, b.instructions.len())).collect();
        assert_eq!(blocks, vec![(0, 1), (4, 2), (12, 1)]);
        assert_eq!(a.successors, vec![Vec::<u64>::new(); 3]);
    }

    #[test]
    fn mos6502_sweep() {
        // $8000: LDA #1; BNE $8006; INC $10; $8006 JMP $8000; $8009 JSR $9000; JMP $9000; RTS
        let prg = [0xA9, 0x01, 0xD0, 0x02, 0xE6, 0x10, 0x4C, 0x00, 0x80, 0x20, 0x00, 0x90, 0x4C, 0x00, 0x90, 0x60];
        let f = func("PRG", 0, 16, 0x8000, None);
        let a = &analyze(&Mos6502, std::slice::from_ref(&f), &prg, &[], |f| Some(f.offset))[0];
        let blocks: Vec<u64> = a.blocks.iter().map(|b| b.offset).collect();
        assert_eq!(blocks, vec![0, 4, 6, 9, 15]);
        assert_eq!(a.successors, vec![vec![4, 6], vec![6], vec![0], vec![], vec![]]);
        // The JMP back to the start stays inside the function, so it isn't a call site.
        let calls: Vec<(u64, &str)> = a.calls.iter().map(|c| (c.at, c.kind)).collect();
        assert_eq!(calls, vec![(9, "JSR"), (12, "JMP")]);
    }
}
