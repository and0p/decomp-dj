//! Instruction decoders, one per instruction set, behind one interface, and the sweep that
//! splits symbol-map functions into instructions and basic blocks with any of them
//! (.agents/DECODER_PLAN.md).
//!
//! A decoder only answers "what is this one instruction?": its length, how control leaves
//! it, and its delay slots. Everything built from those answers (blocks, successors, call
//! sites) is worked out here once, for every instruction set.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{anyhow, bail, Context, Result};

use crate::analysis::{AnalyzedFunction, Block, CallSite, Function, Instruction};
pub use dj_decoder::isa::{op_len_6502, Mips3, Mos6502};
pub use dj_decoder::{Decoded, Decoder, Kind};

/// The built-in decoder for an instruction set, by the name connectors report.
pub fn decoder_for(cpu: &str) -> Option<&'static dyn Decoder> {
    BUILT_IN.iter().copied().find(|d| d.cpu() == cpu)
}

static BUILT_IN: [&dyn Decoder; 3] = [&Mos6502, &Mips3, &crate::arm::Armv4t];

// ---- plugins ------------------------------------------------------------------------

/// `[[decoders]]` in config.toml or .local/local.toml (docs/decoders.md).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct DecoderConfig {
    /// The library. A bare name is looked for in the project folder, then in `decoders/`
    /// next to the dj executable; a relative path is from the project folder. The platform's
    /// suffix may be left out, as for cores.
    pub path: PathBuf,
    /// Pins the library, like `[core] sha256`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    /// Optional: entries with a name merge with local.toml's by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Where a `[[decoders]]` path points (docs/decoders.md).
pub fn resolve_library(root: &Path, path: &Path, folder: &str) -> PathBuf {
    if path.is_absolute() {
        return crate::libretro::library_path(path);
    }
    let in_project = crate::libretro::library_path(&root.join(path));
    if in_project.exists() || path.components().count() > 1 {
        return in_project;
    }
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| crate::libretro::library_path(&d.join(folder).join(path))));
    beside.filter(|p| p.exists()).unwrap_or(in_project)
}

/// A decoder from a plugin library (`dj_decoders`, decoders/include/dj_decoder.h).
pub struct PluginDecoder {
    cpu: &'static str,
    id: String,
    table: &'static dj_decoder::ffi::DjDecoder,
    max_len: usize,
    /// Answers out of range (a kind it doesn't know, a length over `max_length`): read as
    /// "not an instruction", and counted for the report.
    invalid: AtomicUsize,
    _library: Arc<libloading::Library>,
}

impl PluginDecoder {
    /// Every decoder in the library at `path`; none when it doesn't export `dj_decoders`.
    /// `what` names the library in messages ("decoder plugin", "core").
    pub fn load(path: &Path, what: &str) -> Result<Vec<PluginDecoder>> {
        use dj_decoder::ffi::{DjDecodersFn, DJ_DECODERS_SYMBOL, DJ_DECODER_API, DJ_DECODER_V1_SIZE};
        let sha = crate::collection::sha256_file(path).with_context(|| format!("reading the {what} {}", path.display()))?;
        // SAFETY: loading a library runs its initialisers; plugins are trusted like cores.
        let library = Arc::new(unsafe { libloading::Library::new(path) }.with_context(|| format!("loading the {what} {}", path.display()))?);
        let Ok(entry) = (unsafe { library.get::<DjDecodersFn>(DJ_DECODERS_SYMBOL) }) else { return Ok(vec![]) };
        let mut count = 0usize;
        let tables = unsafe { entry(&mut count) };
        let mut out = vec![];
        for i in 0..if tables.is_null() { 0 } else { count } {
            let t = unsafe { *tables.add(i) };
            let Some(t) = (unsafe { t.as_ref() }) else { bail!("{what} {}: decoder {i} is NULL", path.display()) };
            if t.api_version != DJ_DECODER_API {
                bail!("{what} {}: decoder {i} is for decoder API {}, and this dj reads version {DJ_DECODER_API}; rebuild it", path.display(), t.api_version);
            }
            if (t.size as usize) < DJ_DECODER_V1_SIZE || t.cpu.is_null() || t.decode.is_none() || t.max_length == 0 {
                bail!("{what} {}: decoder {i} is incomplete (size {}, cpu, decode and max_length are required)", path.display(), t.size);
            }
            let cpu = unsafe { std::ffi::CStr::from_ptr(t.cpu) }.to_str().map_err(|_| anyhow!("{what} {}: decoder {i}'s cpu isn't UTF-8", path.display()))?;
            // The table and its strings live as long as the library, which is never unloaded.
            let cpu: &'static str = Box::leak(cpu.to_string().into_boxed_str());
            let table: &'static dj_decoder::ffi::DjDecoder = unsafe { &*(t as *const _) };
            out.push(PluginDecoder { cpu, id: format!("{} sha256 {}", path.display(), &sha[..12.min(sha.len())]), table, max_len: t.max_length as usize, invalid: AtomicUsize::new(0), _library: library.clone() });
        }
        // Keep the library loaded for good: the tables point into it.
        std::mem::forget(library);
        Ok(out)
    }

    /// Answers so far that were out of range.
    pub fn invalid_answers(&self) -> usize {
        self.invalid.load(Ordering::Relaxed)
    }

    fn site(ptr: *const std::ffi::c_char) -> &'static str {
        static SITES: OnceLock<RwLock<HashMap<usize, &'static str>>> = OnceLock::new();
        if ptr.is_null() {
            return "";
        }
        let map = SITES.get_or_init(Default::default);
        if let Some(s) = map.read().unwrap_or_else(|e| e.into_inner()).get(&(ptr as usize)) {
            return s;
        }
        let s: &'static str = Box::leak(unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy().into_owned().into_boxed_str());
        map.write().unwrap_or_else(|e| e.into_inner()).insert(ptr as usize, s);
        s
    }
}

impl Decoder for PluginDecoder {
    fn cpu(&self) -> &'static str {
        self.cpu
    }

    fn id(&self) -> String {
        self.id.clone()
    }

    fn max_len(&self) -> usize {
        self.max_len
    }

    fn fixed_len(&self) -> Option<u8> {
        (self.table.fixed_length != 0).then_some(self.table.fixed_length)
    }

    fn alignment(&self, mode: u32) -> u8 {
        self.table.alignment.map_or(1, |f| unsafe { f(mode) }).max(1)
    }

    fn mode_from_flags(&self, flags: u8) -> Option<u32> {
        let f = self.table.mode_from_flags?;
        let mut mode = 0u32;
        (unsafe { f(flags, &mut mode) } != 0).then_some(mode)
    }

    fn decode(&self, bytes: &[u8], pc: u64, mode: u32) -> Decoded {
        let mut out = dj_decoder::ffi::DjInstruction::default();
        // SAFETY: decode is non-NULL (checked on load); bytes and out are valid for the call.
        unsafe { (self.table.decode.unwrap())(bytes.as_ptr(), bytes.len(), pc, mode, &mut out) };
        match out.to_decoded(Self::site(out.site)) {
            Ok(d) if (d.len as usize) <= self.max_len => d,
            _ => {
                self.invalid.fetch_add(1, Ordering::Relaxed);
                Decoded::next(0)
            }
        }
    }
}

/// The decoders a project uses, by instruction set: its `[[decoders]]` plugins first, then
/// the built-in ones, then any the project's core exports (docs/decoders.md).
pub struct Decoders {
    plugins: Vec<PluginDecoder>,
    /// The core library, whose decoders are loaded the first time a CPU nothing else
    /// decodes is asked for.
    core: Option<PathBuf>,
    from_core: OnceLock<Vec<PluginDecoder>>,
}

impl Decoders {
    /// Only the built-in decoders.
    pub fn built_in() -> Decoders {
        Decoders { plugins: vec![], core: None, from_core: OnceLock::new() }
    }

    /// The project's: its `[[decoders]]`, loaded now (an error names the entry), and its core.
    pub fn load(col: &crate::collection::Collection) -> Result<Decoders> {
        let mut plugins = vec![];
        for d in &col.config.decoders {
            let path = resolve_library(&col.root, &d.path, "decoders");
            if !path.is_file() {
                bail!("[[decoders]] {}: {} doesn't exist", d.path.display(), path.display());
            }
            if !d.sha256.is_empty() {
                let sha = crate::collection::sha256_file(&path)?;
                if sha != d.sha256 {
                    bail!("[[decoders]] {}: {} has sha256 {sha}, but the entry is pinned to {}", d.path.display(), path.display(), d.sha256);
                }
            }
            let found = PluginDecoder::load(&path, "decoder plugin")?;
            if found.is_empty() {
                bail!("[[decoders]] {}: {} exports no decoders (no dj_decoders function)", d.path.display(), path.display());
            }
            plugins.extend(found);
        }
        let core = col.settings().ok().and_then(|s| s.core).map(|p| crate::libretro::library_path(&p)).filter(|p| p.is_file());
        Ok(Decoders { plugins, core, from_core: OnceLock::new() })
    }

    /// The decoder for an instruction set, by the name connectors report.
    pub fn get(&self, cpu: &str) -> Option<&dyn Decoder> {
        if let Some(p) = self.plugins.iter().find(|d| d.cpu == cpu) {
            return Some(p);
        }
        if let Some(b) = decoder_for(cpu) {
            return Some(b);
        }
        let from_core = self.from_core.get_or_init(|| self.core.as_deref().and_then(|p| PluginDecoder::load(p, "core").ok()).unwrap_or_default());
        from_core.iter().find(|d| d.cpu == cpu).map(|d| d as &dyn Decoder)
    }

    /// Answers out of range from the plugin decoding `cpu`, if a plugin does.
    pub fn invalid_answers(&self, cpu: &str) -> usize {
        self.plugins.iter().chain(self.from_core.get().into_iter().flatten()).filter(|d| d.cpu == cpu).map(|d| d.invalid_answers()).sum()
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
