//! Symbol maps and static analysis used to roll byte coverage up into
//! instructions, basic blocks and functions (docs/coverage.md).

use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Function {
    pub name: String,
    pub region: String,
    /// Offset into the region (e.g. PRG ROM offset).
    pub offset: u64,
    pub size: u64,
    /// Address the CPU sees for `offset`, when known (needed to resolve branch targets).
    pub cpu_addr: Option<u64>,
    /// Where the function's bytes lie in the ROM file, for code that runs from a copy in RAM
    /// (N64): the decoder reads them there. None: they're the region's own bytes.
    pub file_offset: Option<u64>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Instruction {
    pub offset: u64,
    pub len: u8,
}

#[derive(Clone, Debug)]
pub struct Block {
    /// Region offset of the first instruction.
    pub offset: u64,
    /// Region offsets of every instruction in the block.
    pub instructions: Vec<u64>,
    pub cpu_addr: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct AnalyzedFunction {
    pub func: Function,
    pub instructions: Vec<Instruction>,
    pub blocks: Vec<Block>,
    /// For each block, the offsets of the blocks control can go to next (inside the
    /// function), as the decoder for the function's instruction set reads them.
    pub successors: Vec<Vec<u64>>,
    /// Direct calls and jumps out of the function's instructions, for callers lists.
    pub calls: Vec<CallSite>,
}

/// A direct call or jump: the instruction's region offset, the CPU address it goes to, and
/// how the instruction set spells it ("JSR", "JAL", ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSite {
    pub at: u64,
    pub target: u64,
    pub kind: &'static str,
}

// ---- symbol maps -------------------------------------------------------------

/// Reads a cc65 linker debug file (`ld65 --dbgfile`). Each `.proc` scope becomes a
/// function. Code labels in code segments that are not inside a `.proc` become
/// functions that run to the next label or proc.
pub fn load_ca65_dbg(path: &Path, rom_header: u64) -> Result<Vec<Function>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    struct Seg {
        name: String,
        start: u64,
        size: u64,
        ooffs: Option<u64>,
    }
    struct Sym {
        name: String,
        val: u64,
        seg: Option<u64>,
        scope: u64,
        is_label: bool,
    }
    struct Scope {
        name: String,
        parent: Option<u64>,
        size: u64,
        sym: Option<u64>,
    }
    let mut segs: HashMap<u64, Seg> = HashMap::new();
    let mut syms: HashMap<u64, Sym> = HashMap::new();
    let mut scopes: HashMap<u64, Scope> = HashMap::new();
    for line in text.lines() {
        let Some((kind, rest)) = line.split_once('\t') else { continue };
        let kv = parse_kv(rest);
        let num = |k: &str| kv.get(k).and_then(|v| parse_dbg_num(v));
        let id = match num("id") {
            Some(i) => i,
            None => continue,
        };
        match kind {
            "seg" => {
                segs.insert(id, Seg { name: kv.get("name").cloned().unwrap_or_default(), start: num("start").unwrap_or(0), size: num("size").unwrap_or(0), ooffs: num("ooffs") });
            }
            "sym" => {
                syms.insert(
                    id,
                    Sym {
                        name: kv.get("name").cloned().unwrap_or_default(),
                        val: num("val").unwrap_or(0),
                        seg: num("seg"),
                        scope: num("scope").unwrap_or(0),
                        is_label: kv.get("type").map(|t| t == "lab").unwrap_or(false),
                    },
                );
            }
            "scope" => {
                if kv.get("type").map(|t| t == "scope").unwrap_or(false) {
                    scopes.insert(id, Scope { name: kv.get("name").cloned().unwrap_or_default(), parent: num("parent"), size: num("size").unwrap_or(0), sym: num("sym") });
                }
            }
            _ => {}
        }
    }
    let is_code_seg = |s: &Seg| s.name.to_ascii_uppercase().contains("CODE") && s.ooffs.is_some();
    let to_offset = |seg: &Seg, val: u64| -> Option<u64> {
        let o = seg.ooffs?;
        if val < seg.start || val >= seg.start + seg.size {
            return None;
        }
        (o + (val - seg.start)).checked_sub(rom_header)
    };
    let full_name = |mut id: u64| -> String {
        let mut parts = vec![];
        while let Some(s) = scopes.get(&id) {
            if !s.name.is_empty() {
                parts.push(s.name.clone());
            }
            match s.parent {
                Some(p) => id = p,
                None => break,
            }
        }
        parts.reverse();
        parts.join("::")
    };

    let mut funcs = vec![];
    let mut covered: Vec<(u64, u64)> = vec![];
    for (sid, sc) in &scopes {
        let Some(sym) = sc.sym.and_then(|s| syms.get(&s)) else { continue };
        let Some(seg) = sym.seg.and_then(|s| segs.get(&s)) else { continue };
        if !is_code_seg(seg) || sc.size == 0 {
            continue;
        }
        let Some(off) = to_offset(seg, sym.val) else { continue };
        funcs.push(Function { name: full_name(*sid), region: "PRG".into(), offset: off, size: sc.size, cpu_addr: Some(sym.val), file_offset: None });
        covered.push((off, off + sc.size));
    }
    // Top-level labels in code segments outside any proc.
    let mut labels: Vec<(u64, u64, String, u64)> = vec![]; // (offset, cpu, name, seg id)
    for s in syms.values() {
        if !s.is_label {
            continue;
        }
        let Some(seg_id) = s.seg else { continue };
        let seg = &segs[&seg_id];
        if !is_code_seg(seg) {
            continue;
        }
        let Some(off) = to_offset(seg, s.val) else { continue };
        if covered.iter().any(|(a, b)| off >= *a && off < *b) {
            continue;
        }
        // Only labels whose scope is a module's top level (no named parent scope).
        let parent_named = scopes.get(&s.scope).map(|sc| !sc.name.is_empty()).unwrap_or(false);
        if parent_named {
            continue;
        }
        labels.push((off, s.val, s.name.clone(), seg_id));
    }
    labels.sort();
    labels.dedup_by_key(|l| l.0);
    let mut starts: BTreeSet<u64> = funcs.iter().map(|f| f.offset).collect();
    starts.extend(labels.iter().map(|l| l.0));
    for (off, cpu, name, seg_id) in &labels {
        let seg = &segs[seg_id];
        let seg_end = to_offset(seg, seg.start).unwrap() + seg.size;
        let next = starts.range(off + 1..).next().copied().unwrap_or(seg_end).min(seg_end);
        if next > *off {
            funcs.push(Function { name: name.clone(), region: "PRG".into(), offset: *off, size: next - off, cpu_addr: Some(*cpu), file_offset: None });
        }
    }
    funcs.sort_by_key(|f| f.offset);
    if funcs.is_empty() {
        bail!("{} has no code symbols", path.display());
    }
    Ok(funcs)
}

fn parse_dbg_num(v: &str) -> Option<u64> {
    if let Some(h) = v.strip_prefix("0x") {
        u64::from_str_radix(h, 16).ok()
    } else {
        v.split('+').next()?.parse().ok()
    }
}

fn parse_kv(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut key = String::new();
    let mut val = String::new();
    let mut in_key = true;
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            '=' if in_key && !quoted => in_key = false,
            ',' if !quoted => {
                out.insert(std::mem::take(&mut key), std::mem::take(&mut val));
                in_key = true;
            }
            _ if in_key => key.push(c),
            _ => val.push(c),
        }
    }
    if !key.is_empty() {
        out.insert(key, val);
    }
    out
}

/// CSV symbol map: `name,region,offset,size[,cpu_addr]`, numbers in hex.
pub fn load_csv(path: &Path) -> Result<Vec<Function>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || (n == 0 && t.starts_with("name,")) {
            continue;
        }
        let f: Vec<&str> = t.split(',').map(|s| s.trim()).collect();
        if f.len() < 4 {
            bail!("{}:{}: expected name,region,offset,size", path.display(), n + 1);
        }
        let hex = |s: &str| u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| anyhow!("{}:{}: bad hex {s}", path.display(), n + 1));
        out.push(Function {
            name: f[0].to_string(),
            region: f[1].to_ascii_uppercase(),
            offset: hex(f[2])?,
            size: hex(f[3])?,
            cpu_addr: f.get(4).filter(|s| !s.is_empty()).map(|s| hex(s)).transpose()?,
            file_offset: f.get(5).filter(|s| !s.is_empty()).map(|s| hex(s)).transpose()?,
        });
    }
    out.sort_by_key(|f| f.offset);
    Ok(out)
}

// ---- loading by format ----------------------------------------------------------

/// The format `dj init` assumes from a symbol file's name.
pub fn guess_format(path: &Path) -> &'static str {
    known_format(path).unwrap_or("csv")
}

/// The symbol map format a file's name says, or None when it says nothing (`guess_format`
/// then assumes csv).
pub fn known_format(path: &Path) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    Some(match ext.as_str() {
        "dbg" => "ca65-dbg",
        "mlb" => "mesen-mlb",
        "map" => "ld-map",
        "elf" | "o" => "elf",
        "csv" => "csv",
        _ if name.contains("symbol_addrs") => "splat",
        _ => return None,
    })
}

/// Bytes before the region's data in `rom` when the config doesn't say: an iNES header is 16.
pub fn default_rom_header(format: &str, rom: Option<&[u8]>) -> u64 {
    match rom {
        Some(r) if r.starts_with(b"NES\x1a") => 16,
        Some(_) => 0,
        // No ROM to look at: cc65 projects are NES projects.
        None => if format == "ca65-dbg" { 16 } else { 0 },
    }
}

/// Loads one `[[symbols]]` entry. `rom` is used only to tell whether it has an iNES header.
pub fn load_symbols(root: &Path, cfg: &crate::collection::SymbolsConfig, rom: Option<&[u8]>) -> Result<Vec<Function>> {
    let path = root.join(&cfg.file);
    let header = cfg.rom_header.unwrap_or_else(|| default_rom_header(&cfg.format, rom));
    let region = cfg.region.clone().unwrap_or_else(|| "PRG".into()).to_ascii_uppercase();
    let mut funcs = match cfg.format.as_str() {
        "ca65-dbg" => load_ca65_dbg(&path, header)?,
        "csv" => load_csv(&path)?,
        "splat" => load_splat(&path, header, &region)?,
        "mesen-mlb" => load_mesen_mlb(&path, &region)?,
        "ld-map" => load_ld_map(&path, header, &region)?,
        "elf" => load_elf(&path, header, &region)?,
        other => bail!("unknown symbols format {other} (expected ca65-dbg, csv, splat, mesen-mlb, ld-map or elf)"),
    };
    if let Some(base) = cfg.cpu_base {
        for f in &mut funcs {
            if f.cpu_addr.is_none() {
                f.cpu_addr = Some(base + f.offset);
            }
        }
    }
    if funcs.is_empty() {
        bail!("{} has no function symbols", path.display());
    }
    Ok(funcs)
}

/// Every function of every `[[symbols]]` map, with annotations.toml's `[[function]]`
/// entries applied, each with the ROM header its map says to skip. Where two maps describe
/// the same bytes of a region, the first one listed wins.
pub fn project_functions(col: &crate::collection::Collection, rom: Option<&[u8]>, ann: &crate::annotations::Annotations) -> Result<Vec<(Function, u64)>> {
    project_functions_with(col, rom, ann, &[])
}

/// `project_functions`, placing each map by where the connector says its region lies in the
/// ROM file (`facts`); `rom_header` in the map's entry overrides that.
pub fn project_functions_with(col: &crate::collection::Collection, rom: Option<&[u8]>, ann: &crate::annotations::Annotations, facts: &[RegionFacts]) -> Result<Vec<(Function, u64)>> {
    let mut funcs: Vec<(Function, u64)> = vec![];
    for s in &col.config.symbols {
        let region = s.region.as_deref().unwrap_or("PRG");
        let placed = facts.iter().find(|f| f.name == region).and_then(|f| f.file_offset);
        let header = s.rom_header.or(placed).unwrap_or_else(|| default_rom_header(&s.format, rom));
        let placed_cfg = crate::collection::SymbolsConfig { rom_header: Some(header), ..s.clone() };
        for f in load_symbols(&col.root, &placed_cfg, rom)? {
            if !funcs.iter().any(|(g, _)| g.region == f.region && f.offset < g.offset + g.size && g.offset < f.offset + f.size) {
                funcs.push((f, header));
            }
        }
    }
    if !ann.entries.iter().any(|e| e.kind == crate::annotations::Kind::Function) {
        return Ok(funcs);
    }
    let default_header = funcs.first().map(|(_, h)| *h).unwrap_or_else(|| default_rom_header("", rom));
    let fixed = rom.and_then(FixedPrg::of_rom);
    let headers: Vec<(String, u64, u64, u64)> = funcs.iter().map(|(f, h)| (f.region.clone(), f.offset, f.offset + f.size, *h)).collect();
    let merged = ann.merge_functions(funcs.into_iter().map(|(f, _)| f).collect(), |region, off| if region == "PRG" { fixed.and_then(|m| m.cpu_of(off)) } else { None })?;
    Ok(merged
        .into_iter()
        .map(|f| {
            let h = headers.iter().find(|(r, a, b, _)| *r == f.region && f.offset >= *a && f.offset < *b).map_or(default_header, |x| x.3);
            (f, h)
        })
        .collect())
}

/// Largest size given to a function whose size isn't known and that has no next symbol.
const LAST_FUNCTION_MAX: u64 = 0x1000;

/// Sorts by offset, drops repeats of an offset (the first one wins), and gives every
/// function without a size (0) the bytes up to the next function, capped by `limit(f)`.
fn finish(mut funcs: Vec<Function>, limit: impl Fn(&Function) -> Option<u64>) -> Vec<Function> {
    funcs.sort_by_key(|f| f.offset);
    funcs.dedup_by_key(|f| (f.region.clone(), f.offset));
    let starts: Vec<u64> = funcs.iter().map(|f| f.offset).collect();
    for (i, f) in funcs.iter_mut().enumerate() {
        if f.size == 0 {
            let next = starts.get(i + 1).copied().unwrap_or(f.offset + LAST_FUNCTION_MAX);
            let end = limit(f).map_or(next, |l| next.min(l));
            f.size = end.saturating_sub(f.offset);
        }
    }
    funcs.retain(|f| f.size > 0);
    funcs
}

fn hex_u64(s: &str) -> Option<u64> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u64::from_str_radix(t, 16).ok()
}

/// splat's `symbol_addrs.txt`: `name = 0x80001234; // type:func rom:0x1234 size:0x40`.
/// Every entry with a `rom:` offset counts, unless it has a `type:` other than `func`
/// or says `ignore:true`. Entries without `size:` run to the next symbol.
pub fn load_splat(path: &Path, rom_header: u64, region: &str) -> Result<Vec<Function>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut out = vec![];
    for line in text.lines() {
        let (code, attrs) = match line.split_once("//") {
            Some((c, a)) => (c, a),
            None => (line, ""),
        };
        let Some((name, val)) = code.split_once('=') else { continue };
        let name = name.trim();
        let Some(vram) = hex_u64(val.trim().trim_end_matches(';')) else { continue };
        if name.is_empty() {
            continue;
        }
        let mut rom = None;
        let mut size = 0;
        let mut skip = false;
        for kv in attrs.split_whitespace() {
            let Some((k, v)) = kv.split_once(':') else { continue };
            match k {
                "rom" => rom = hex_u64(v),
                "size" => size = hex_u64(v).unwrap_or(0),
                "type" => skip |= v != "func",
                "ignore" => skip |= v == "true",
                _ => {}
            }
        }
        let (Some(rom), false) = (rom, skip) else { continue };
        let Some(offset) = rom.checked_sub(rom_header) else { continue };
        out.push(Function { name: name.to_string(), region: region.to_string(), offset, size, cpu_addr: Some(vram), file_offset: None });
    }
    Ok(finish(out, |_| None))
}

/// Mesen label file (`.mlb`): `P:1F3A:name` or `P:1F3A:name:comment`, offsets in hex
/// relative to the start of PRG ROM. Mesen 2 writes `NesPrgRom:1F3A:name` (and
/// `SnesPrgRom`, `GbPrgRom`, ...). Only single-address PRG labels count; ranges
/// (`P:1F3A-1F40:table`) are data. Each runs to the next label.
pub fn load_mesen_mlb(path: &Path, region: &str) -> Result<Vec<Function>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut out = vec![];
    for line in text.lines() {
        let mut parts = line.splitn(4, ':');
        let (Some(kind), Some(addr), Some(name)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let is_prg = kind == "P" || kind.ends_with("PrgRom");
        let name = name.trim();
        if !is_prg || name.is_empty() || addr.contains('-') {
            continue;
        }
        let Some(offset) = hex_u64(addr) else { continue };
        out.push(Function { name: name.to_string(), region: region.to_string(), offset, size: 0, cpu_addr: None, file_offset: None });
    }
    Ok(finish(out, |_| None))
}

/// A GNU ld map file (`-Map=out.map`). Symbols in input sections whose names start
/// with `.text` count as functions. A symbol's ROM offset is its address moved from
/// the output section's address to its load address (`load address 0x...`, which
/// N64 and other decomp linker scripts use for ROM offsets), less `rom_header`.
/// Each runs to the next symbol, within its input section.
pub fn load_ld_map(path: &Path, rom_header: u64, region: &str) -> Result<Vec<Function>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    // Output section: (address, load address).
    let mut out_sec: Option<(u64, u64)> = None;
    // Input section: is code, start and end address.
    let mut in_sec: Option<(bool, u64, u64)> = None;
    let mut pending_name: Option<(bool, String)> = None;
    let mut syms: Vec<(Function, u64)> = vec![];
    let in_map = text.find("Linker script and memory map").map_or(0, |i| i);
    for line in text[in_map..].lines() {
        let indented = line.starts_with(' ');
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        // A section name too long for its column puts the numbers on the next line.
        let (words, top, name): (Vec<&str>, bool, Option<String>) = match pending_name.take() {
            Some((top, name)) if words[0].starts_with("0x") => (words, top, Some(name)),
            _ if words.len() == 1 && words[0].starts_with('.') => {
                pending_name = Some((!indented, words[0].to_string()));
                continue;
            }
            _ if words[0].starts_with('.') || (!indented && !words[0].starts_with("0x")) => {
                let n = words[0].to_string();
                (words[1..].to_vec(), !indented, Some(n))
            }
            _ => (words, false, None),
        };
        match name {
            Some(name) if !words.is_empty() => {
                let (Some(addr), Some(size)) = (words.first().and_then(|w| hex_u64(w)), words.get(1).and_then(|w| hex_u64(w))) else { continue };
                if top {
                    let lma = words.iter().position(|w| *w == "address").and_then(|i| words.get(i + 1)).and_then(|w| hex_u64(w)).unwrap_or(addr);
                    out_sec = Some((addr, lma));
                    in_sec = None;
                } else {
                    in_sec = Some((name.starts_with(".text"), addr, addr + size));
                }
            }
            Some(_) => {}
            None => {
                // `0x80001234  name`: a symbol, unless it is an assignment.
                if words.len() != 2 || words[1].contains('=') || words[1].contains('(') {
                    continue;
                }
                let (Some(addr), Some((true, _, end)), Some((vma, lma))) = (hex_u64(words[0]), in_sec, out_sec) else { continue };
                let Some(offset) = (addr.wrapping_sub(vma).wrapping_add(lma)).checked_sub(rom_header) else { continue };
                let end_off = end.wrapping_sub(vma).wrapping_add(lma).saturating_sub(rom_header);
                syms.push((Function { name: words[1].to_string(), region: region.to_string(), offset, size: 0, cpu_addr: Some(addr), file_offset: None }, end_off));
            }
        }
    }
    let ends: HashMap<u64, u64> = syms.iter().map(|(f, e)| (f.offset, *e)).collect();
    Ok(finish(syms.into_iter().map(|(f, _)| f).collect(), |f| ends.get(&f.offset).copied()))
}

/// An ELF file built from the decomp: its function symbols (`STT_FUNC`), with their sizes.
/// A symbol's ROM offset is its address moved from the program segment's virtual address
/// to its physical (load) address, less `rom_header`; without program headers, its address.
pub fn load_elf(path: &Path, rom_header: u64, region: &str) -> Result<Vec<Function>> {
    use object::read::elf::{ElfFile, FileHeader, ProgramHeader};
    use object::{Object, ObjectSymbol, SymbolKind};
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    fn read<H: FileHeader>(data: &[u8], rom_header: u64, region: &str) -> Result<Vec<Function>> {
        let file = ElfFile::<H>::parse(data)?;
        let e = file.endian();
        let segs: Vec<(u64, u64, u64)> = file
            .elf_program_headers()
            .iter()
            .filter(|p| p.p_memsz(e).into() > 0)
            .map(|p| (p.p_vaddr(e).into(), p.p_paddr(e).into(), p.p_memsz(e).into()))
            .collect();
        let mut out = vec![];
        for sym in file.symbols() {
            if sym.kind() != SymbolKind::Text || !sym.is_definition() {
                continue;
            }
            let Ok(name) = sym.name() else { continue };
            let addr = sym.address();
            let load = segs.iter().find(|(v, _, n)| addr >= *v && addr < v + n).map_or(addr, |(v, p, _)| addr - v + p);
            let Some(offset) = load.checked_sub(rom_header) else { continue };
            out.push(Function { name: name.to_string(), region: region.to_string(), offset, size: sym.size(), cpu_addr: Some(addr), file_offset: None });
        }
        Ok(finish(out, |_| None))
    }
    match object::FileKind::parse(&*data)? {
        object::FileKind::Elf32 => read::<object::elf::FileHeader32<object::Endianness>>(&data, rom_header, region),
        object::FileKind::Elf64 => read::<object::elf::FileHeader64<object::Endianness>>(&data, rom_header, region),
        k => bail!("{} is not an ELF file ({k:?})", path.display()),
    }
}

// ---- 6502 ---------------------------------------------------------------------

/// Instruction length for each 6502 opcode, including the stable undocumented ones.
pub fn op_len_6502(op: u8) -> u8 {
    let row = op >> 4;
    let odd_row = row & 1 == 1;
    match op & 0x0f {
        0x0 => {
            if odd_row {
                2
            } else {
                match op {
                    0x20 => 3,
                    0x00 | 0x40 | 0x60 => 1,
                    _ => 2,
                }
            }
        }
        0x1 | 0x3 | 0x4 | 0x5 | 0x6 | 0x7 => 2,
        0x2 => match op {
            0x82 | 0xA2 | 0xC2 | 0xE2 => 2,
            _ => 1, // KIL / JAM
        },
        0x8 | 0xA => 1,
        0x9 | 0xB => {
            if odd_row {
                3
            } else {
                2
            }
        }
        _ => 3, // 0xC..0xF
    }
}

enum Flow {
    Next,
    Branch(u64),
    Jump(Option<u64>),
    Stop,
}

fn flow_6502(op: u8, bytes: &[u8], cpu: u64) -> Flow {
    match op {
        0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0 => {
            let rel = bytes.get(1).copied().unwrap_or(0) as i8 as i64;
            Flow::Branch(((cpu as i64 + 2 + rel) & 0xffff) as u64)
        }
        0x4C => Flow::Jump(Some(u16::from_le_bytes([bytes.get(1).copied().unwrap_or(0), bytes.get(2).copied().unwrap_or(0)]) as u64)),
        0x6C => Flow::Jump(None),
        0x00 | 0x40 | 0x60 => Flow::Stop,
        op if op & 0x0f == 0x2 && !matches!(op, 0x82 | 0xA2 | 0xC2 | 0xE2) => Flow::Stop,
        _ => Flow::Next,
    }
}

/// Linear-sweep disassembly of each function into instructions and basic blocks.
/// Data embedded in a function's range can throw the sweep off; ca65 code keeps
/// data in RODATA, so this is rarely a problem there.
pub fn analyze_6502(funcs: &[Function], rom: &[u8]) -> Vec<AnalyzedFunction> {
    analyze_6502_with(funcs, rom, &[])
}

/// `analyze_6502`, skipping `data` (inclusive offset ranges, such as jump tables from
/// annotations.toml): the sweep starts again after each one, with a new block there.
pub fn analyze_6502_with(funcs: &[Function], rom: &[u8], data: &[(u64, u64)]) -> Vec<AnalyzedFunction> {
    let mut out = vec![];
    for f in funcs {
        let mut insns = vec![];
        let mut leaders: BTreeSet<u64> = BTreeSet::new();
        leaders.insert(f.offset);
        let end = (f.offset + f.size).min(rom.len() as u64);
        let mut off = f.offset;
        let cpu_of = |o: u64| f.cpu_addr.map(|c| c + (o - f.offset));
        let off_of_cpu = |cpu: u64| -> Option<u64> {
            let base = f.cpu_addr?;
            if cpu >= base && cpu < base + f.size {
                Some(f.offset + (cpu - base))
            } else {
                None
            }
        };
        while off < end {
            if let Some(&(_, e)) = data.iter().find(|(a, b)| off >= *a && off <= *b) {
                off = e + 1;
                leaders.insert(off);
                continue;
            }
            let op = rom[off as usize];
            let len = op_len_6502(op);
            if off + len as u64 > end {
                break;
            }
            // An instruction can't run on into data: the data starts where it would be.
            if let Some(&(a, _)) = data.iter().filter(|(a, _)| *a > off && *a < off + len as u64).min() {
                off = a;
                continue;
            }
            insns.push(Instruction { offset: off, len });
            let bytes = &rom[off as usize..(off + len as u64) as usize];
            let next = off + len as u64;
            match flow_6502(op, bytes, cpu_of(off).unwrap_or(0)) {
                Flow::Next => {}
                Flow::Branch(t) => {
                    if f.cpu_addr.is_some() {
                        if let Some(o) = off_of_cpu(t) {
                            leaders.insert(o);
                        }
                    }
                    leaders.insert(next);
                }
                Flow::Jump(t) => {
                    if let Some(o) = t.and_then(off_of_cpu) {
                        leaders.insert(o);
                    }
                    leaders.insert(next);
                }
                Flow::Stop => {
                    leaders.insert(next);
                }
            }
            off = next;
        }
        let mut blocks: Vec<Block> = vec![];
        for i in &insns {
            if leaders.contains(&i.offset) || blocks.is_empty() {
                blocks.push(Block { offset: i.offset, instructions: vec![], cpu_addr: cpu_of(i.offset) });
            }
            blocks.last_mut().unwrap().instructions.push(i.offset);
        }
        let calls = insns
            .iter()
            .filter_map(|i| {
                let bytes = &rom[i.offset as usize..((i.offset + i.len as u64) as usize).min(rom.len())];
                call_target_6502(bytes).map(|(t, call)| CallSite { at: i.offset, target: t, kind: if call { "JSR" } else { "JMP" } })
            })
            .collect();
        let mut a = AnalyzedFunction { func: f.clone(), instructions: insns, blocks, successors: vec![], calls };
        a.successors = block_successors_6502(&a, rom);
        out.push(a);
    }
    out
}

// ---- MIPS (the N64's VR4300, "mips3") ----------------------------------------------------

/// How a MIPS instruction leaves the straight line. Branches and jumps have a delay slot:
/// the next instruction runs before control moves (for "likely" branches, only when taken).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MipsFlow {
    Next,
    /// A call (JAL, JALR, BAL, BLTZAL ...): control comes back after the delay slot.
    Call(Option<u64>),
    /// A conditional branch to the CPU address.
    Branch(u64),
    /// An unconditional jump: J, B (BEQ $0,$0), or JR other than `jr $ra` (None: indirect).
    Jump(Option<u64>),
    /// JR $ra, ERET.
    Return,
    /// BREAK.
    Stop,
}

/// Reads one instruction word at CPU address `pc`.
fn flow_mips(w: u32, pc: u64) -> MipsFlow {
    let op = w >> 26;
    let (rs, rt, funct) = ((w >> 21) & 31, (w >> 16) & 31, w & 63);
    let branch = pc.wrapping_add(4).wrapping_add(((w as u16 as i16 as i64) << 2) as u64) & 0xFFFF_FFFF;
    let region = (pc.wrapping_add(4) & 0xF000_0000) | (((w & 0x03FF_FFFF) as u64) << 2);
    match op {
        0 => match funct {
            0x08 if rs == 31 => MipsFlow::Return,
            0x08 => MipsFlow::Jump(None),
            0x09 => MipsFlow::Call(None),
            0x0D => MipsFlow::Stop,
            _ => MipsFlow::Next,
        },
        // REGIMM: BLTZ, BGEZ, their likely forms, and the linking ones (BAL is BGEZAL $0).
        1 => match rt {
            0x00..=0x03 => MipsFlow::Branch(branch),
            0x10..=0x13 => MipsFlow::Call(Some(branch)),
            _ => MipsFlow::Next,
        },
        2 => MipsFlow::Jump(Some(region)),
        3 => MipsFlow::Call(Some(region)),
        // BEQ $0,$0 (and BEQL) is an unconditional branch.
        4 | 0x14 if rs == 0 && rt == 0 => MipsFlow::Jump(Some(branch)),
        4..=7 | 0x14..=0x17 => MipsFlow::Branch(branch),
        0x10 if rs == 0x10 && funct == 0x18 => MipsFlow::Return,
        // BC1F, BC1T and their likely forms.
        0x11 if rs == 8 => MipsFlow::Branch(branch),
        _ => MipsFlow::Next,
    }
}

/// Splits each function into instructions and basic blocks, reading its words from `rom`
/// (the ROM file, big-endian .z64 order) at the function's `file_offset`. A block ends after
/// the delay slot of a branch or jump; branch targets inside the function start blocks.
/// Functions without a `file_offset` are skipped (no bytes to read). `data` ranges (region
/// offsets, inclusive) aren't read as code.
pub fn analyze_mips_with(funcs: &[Function], rom: &[u8], data: &[(u64, u64)]) -> Vec<AnalyzedFunction> {
    let mut out = vec![];
    for f in funcs {
        let Some(file) = f.file_offset else { continue };
        let word_at = |o: u64| -> Option<u32> {
            let p = (file + (o - f.offset)) as usize;
            rom.get(p..p + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        let cpu_of = |o: u64| f.cpu_addr.map(|c| c + (o - f.offset));
        let off_of_cpu = |cpu: u64| -> Option<u64> {
            let base = f.cpu_addr?;
            (cpu >= base && cpu < base + f.size).then(|| f.offset + (cpu - base))
        };
        let in_data = |o: u64| data.iter().any(|(a, b)| o + 3 >= *a && o <= *b);
        let end = f.offset + f.size;
        let mut insns = vec![];
        let mut flows = vec![];
        let mut leaders: BTreeSet<u64> = BTreeSet::new();
        leaders.insert(f.offset);
        let mut off = f.offset;
        while off + 4 <= end {
            if in_data(off) {
                off += 4;
                leaders.insert(off);
                continue;
            }
            let Some(w) = word_at(off) else { break };
            let flow = flow_mips(w, cpu_of(off).unwrap_or(0));
            insns.push(Instruction { offset: off, len: 4 });
            flows.push(flow);
            let after_slot = off + 8;
            match flow {
                MipsFlow::Next | MipsFlow::Call(_) => {}
                MipsFlow::Branch(t) => {
                    leaders.extend(off_of_cpu(t));
                    leaders.insert(after_slot);
                }
                MipsFlow::Jump(t) => {
                    leaders.extend(t.and_then(off_of_cpu));
                    leaders.insert(after_slot);
                }
                MipsFlow::Return | MipsFlow::Stop => {
                    leaders.insert(after_slot);
                }
            }
            off += 4;
        }
        let mut blocks: Vec<Block> = vec![];
        for i in &insns {
            if leaders.contains(&i.offset) || blocks.is_empty() {
                blocks.push(Block { offset: i.offset, instructions: vec![], cpu_addr: cpu_of(i.offset) });
            }
            blocks.last_mut().unwrap().instructions.push(i.offset);
        }
        let flow_at = |o: u64| insns.iter().position(|i| i.offset == o).map(|k| flows[k]);
        let successors = blocks
            .iter()
            .map(|b| {
                let last = *b.instructions.last().unwrap();
                // The block's control instruction is the last one, or the one before it when
                // the last is its delay slot.
                let (ctl, flow) = match b.instructions.len() {
                    n if n >= 2 && !matches!(flow_at(b.instructions[n - 2]), Some(MipsFlow::Next | MipsFlow::Call(_))) => (b.instructions[n - 2], flow_at(b.instructions[n - 2]).unwrap()),
                    _ => (last, flow_at(last).unwrap_or(MipsFlow::Next)),
                };
                let next = ctl + 8;
                let mut out = match flow {
                    MipsFlow::Next | MipsFlow::Call(_) => vec![last + 4],
                    MipsFlow::Branch(t) => [Some(next), off_of_cpu(t)].into_iter().flatten().collect(),
                    MipsFlow::Jump(t) => t.and_then(off_of_cpu).into_iter().collect(),
                    MipsFlow::Return | MipsFlow::Stop => vec![],
                };
                out.retain(|o| *o < end && blocks.iter().any(|b| b.offset == *o));
                out.dedup();
                out
            })
            .collect();
        let calls = insns
            .iter()
            .zip(&flows)
            .filter_map(|(i, fl)| match fl {
                MipsFlow::Call(Some(t)) => Some(CallSite { at: i.offset, target: *t, kind: "JAL" }),
                MipsFlow::Jump(Some(t)) if off_of_cpu(*t).is_none() => Some(CallSite { at: i.offset, target: *t, kind: "J" }),
                _ => None,
            })
            .collect();
        out.push(AnalyzedFunction { func: f.clone(), instructions: insns, blocks, successors, calls });
    }
    out
}

/// Where control can go after each block of `f`: (block index, successor offsets in the
/// same region). Falling through and branch targets are included; a jump or branch target
/// outside the function is left out. Used to find covered code that skips an uncovered block.
pub fn block_successors_6502(f: &AnalyzedFunction, prg: &[u8]) -> Vec<Vec<u64>> {
    let off_of_cpu = |cpu: u64| -> Option<u64> {
        let base = f.func.cpu_addr?;
        (cpu >= base && cpu < base + f.func.size).then(|| f.func.offset + (cpu - base))
    };
    let end = f.func.offset + f.func.size;
    f.blocks
        .iter()
        .map(|b| {
            let Some(&last) = b.instructions.last() else { return vec![] };
            let Some(&op) = prg.get(last as usize) else { return vec![] };
            let len = op_len_6502(op) as u64;
            let bytes = &prg[last as usize..((last + len) as usize).min(prg.len())];
            let cpu = f.func.cpu_addr.map(|c| c + (last - f.func.offset)).unwrap_or(0);
            let next = last + len;
            let mut out = vec![];
            match flow_6502(op, bytes, cpu) {
                Flow::Next => out.push(next),
                Flow::Branch(t) => {
                    out.push(next);
                    if let Some(o) = off_of_cpu(t) {
                        out.push(o);
                    }
                }
                Flow::Jump(t) => {
                    if op == 0x20 {
                        out.push(next);
                    } else if let Some(o) = t.and_then(off_of_cpu) {
                        out.push(o);
                    }
                }
                Flow::Stop => {}
            }
            out.retain(|o| *o < end && f.blocks.iter().any(|b| b.offset == *o));
            out.dedup();
            out
        })
        .collect()
}

// ---- 6502 code with no symbol map --------------------------------------------

/// Where PRG sits in CPU space, for a NES ROM with too little PRG to bank: 16 KB (mirrored at
/// $8000 and $C000) or 32 KB at $8000. Larger ROMs give None: an address there means nothing
/// without knowing the bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedPrg {
    size: u64,
}

impl FixedPrg {
    pub fn of_rom(rom: &[u8]) -> Option<FixedPrg> {
        if !rom.starts_with(b"NES\x1a") || rom.len() < 16 {
            return None;
        }
        let size = rom[4] as u64 * 0x4000;
        matches!(size, 0x4000 | 0x8000).then_some(FixedPrg { size })
    }

    pub fn cpu_of(&self, offset: u64) -> Option<u64> {
        (offset < self.size).then(|| 0x10000 - self.size + offset)
    }

    pub fn offset_of(&self, cpu: u64) -> Option<u64> {
        (0x8000..0x10000).contains(&cpu).then(|| (cpu - 0x8000) % self.size)
    }
}

fn ran(flags: &[u8], off: u64) -> bool {
    flags.get(off as usize).is_some_and(|b| b & crate::coverage::FLAG_OPCODE != 0)
}

/// What a connector reports about one of its regions (docs/connectors.md, `dj_region`),
/// recorded per setup in the local database so queries without a core know it too.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegionFacts {
    pub name: String,
    pub size: usize,
    /// The instruction set that runs there ("6502"), if any.
    pub cpu: Option<String>,
    /// Whether the connector hands out the region's bytes.
    pub has_data: bool,
    /// Where offset 0 lies in the ROM file, for regions that come from it.
    pub file_offset: Option<u64>,
}

/// Instruction sets the player can decode (split into instructions and blocks).
pub fn has_decoder(cpu: &str) -> bool {
    matches!(cpu, "6502" | "mips3")
}

/// The region coverage rolls up into, the first of: a region from the ROM file whose
/// instruction set has a decoder (NES: PRG); one from the ROM file that some CPU runs; any
/// region a CPU runs (N64: RDRAM, since code runs from RAM there). "PRG" when the connector
/// says nothing (no facts recorded yet).
pub fn code_region(facts: &[RegionFacts]) -> String {
    let runs = |f: &&RegionFacts| f.cpu.is_some();
    facts
        .iter()
        .find(|f| f.file_offset.is_some() && f.cpu.as_deref().is_some_and(has_decoder))
        .or_else(|| facts.iter().filter(runs).find(|f| f.file_offset.is_some()))
        .or_else(|| facts.iter().find(runs))
        .map(|f| f.name.clone())
        .unwrap_or_else(|| "PRG".into())
}

/// The bytes of every instruction that ran, operands included, and whether they came from
/// the connector's operand flags (true) or, for a core that sets none, from 6502 lengths.
pub fn executed_bytes(prg: &[u8], flags: &[u8]) -> (Vec<bool>, bool) {
    if has_operand_flags(flags) {
        (flags.iter().map(|f| f & (crate::coverage::FLAG_OPCODE | crate::coverage::FLAG_OPERAND) != 0).collect(), true)
    } else {
        (executed_bytes_6502(prg, flags), false)
    }
}

/// Whether the connector set the operand flag anywhere in these flags. A region with code
/// that ran but none set comes from a core that only marks opcodes (or block starts).
pub fn has_operand_flags(flags: &[u8]) -> bool {
    flags.iter().any(|f| f & crate::coverage::FLAG_OPERAND != 0)
}

/// What `dj check-connector` finds: the connector's flags against the decoder, inside the
/// functions of the symbol maps (docs/cli.md).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ConformanceReport {
    /// Whether the region has any operand flags (else only opcodes are checked).
    pub operand_flags: bool,
    /// Instructions the decoder found that ran.
    pub instructions_checked: usize,
    /// An opcode flag on a byte the decoder doesn't start an instruction at.
    pub opcode_not_at_start: Vec<u64>,
    /// An operand flag on a byte that isn't an operand of an instruction that ran.
    pub operand_unexpected: Vec<u64>,
    /// An operand byte of an instruction that ran, without the operand flag.
    pub operand_missing: Vec<u64>,
    /// Opcode bytes that ran in both widths of a mode pair (bits 0x04 and 0x08, or 0x10 and
    /// 0x20): ambiguous lengths. Always 0 on CPUs without mode bits.
    pub both_widths: usize,
}

/// Checks `flags` against the instructions of `funcs`, skipping bytes in `skip` (annotated
/// data and unreachable ranges, inclusive), which the decoder was told not to read.
pub fn check_flags(funcs: &[AnalyzedFunction], flags: &[u8], skip: &[(u64, u64)]) -> ConformanceReport {
    use crate::coverage::{FLAG_OPCODE, FLAG_OPERAND};
    let mut r = ConformanceReport { operand_flags: has_operand_flags(flags), ..Default::default() };
    let at = |o: u64| flags.get(o as usize).copied().unwrap_or(0);
    let skipped = |o: u64| skip.iter().any(|(a, b)| o >= *a && o <= *b);
    for f in funcs {
        let (start, end) = (f.func.offset, f.func.offset + f.func.size);
        // What each byte of the function should carry, from the decoder.
        let mut want = vec![0u8; (end - start) as usize];
        for ins in &f.instructions {
            if at(ins.offset) & FLAG_OPCODE == 0 {
                continue;
            }
            r.instructions_checked += 1;
            for k in 1..ins.len as u64 {
                if let Some(w) = want.get_mut((ins.offset + k - start) as usize) {
                    *w |= FLAG_OPERAND;
                }
            }
        }
        let starts: std::collections::HashSet<u64> = f.instructions.iter().map(|i| i.offset).collect();
        for o in start..end {
            if skipped(o) {
                continue;
            }
            let (have, w) = (at(o), want[(o - start) as usize]);
            if have & FLAG_OPCODE != 0 {
                if !starts.contains(&o) {
                    r.opcode_not_at_start.push(o);
                }
                if (have & 0x0C == 0x0C) || (have & 0x30 == 0x30) {
                    r.both_widths += 1;
                }
            }
            if r.operand_flags {
                if have & FLAG_OPERAND != 0 && w & FLAG_OPERAND == 0 {
                    r.operand_unexpected.push(o);
                }
                if w & FLAG_OPERAND != 0 && have & FLAG_OPERAND == 0 {
                    r.operand_missing.push(o);
                }
            }
        }
    }
    r
}

/// The bytes of every instruction that ran, operands included, from 6502 lengths.
pub fn executed_bytes_6502(prg: &[u8], flags: &[u8]) -> Vec<bool> {
    let n = flags.len().min(prg.len());
    let mut out = vec![false; n];
    for off in 0..n {
        if ran(flags, off as u64) {
            let len = op_len_6502(prg[off]) as usize;
            out[off..(off + len).min(n)].fill(true);
        }
    }
    out
}

/// The maximal runs (`start..end`) of bytes that are not part of any instruction that ran.
pub fn uncovered_ranges(executed: &[bool]) -> Vec<(u64, u64)> {
    let mut out = vec![];
    let mut start = None;
    for (i, e) in executed.iter().enumerate() {
        match (*e, start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                out.push((s as u64, i as u64));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s as u64, executed.len() as u64));
    }
    out
}

/// The CPU address a `JSR abs` or `JMP abs` at the start of `bytes` targets.
pub fn call_target_6502(bytes: &[u8]) -> Option<(u64, bool)> {
    match bytes {
        [0x20, lo, hi, ..] => Some((u16::from_le_bytes([*lo, *hi]) as u64, true)),
        [0x4C, lo, hi, ..] => Some((u16::from_le_bytes([*lo, *hi]) as u64, false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(op_len_6502(0xA9), 2); // LDA #
        assert_eq!(op_len_6502(0xAD), 3); // LDA abs
        assert_eq!(op_len_6502(0xB1), 2); // LDA (zp),Y
        assert_eq!(op_len_6502(0xB9), 3); // LDA abs,Y
        assert_eq!(op_len_6502(0x20), 3); // JSR
        assert_eq!(op_len_6502(0x60), 1); // RTS
        assert_eq!(op_len_6502(0xEA), 1); // NOP
        assert_eq!(op_len_6502(0xD0), 2); // BNE
        assert_eq!(op_len_6502(0x6C), 3); // JMP (ind)
        assert_eq!(op_len_6502(0x0A), 1); // ASL A
        assert_eq!(op_len_6502(0xA2), 2); // LDX #
        assert_eq!(op_len_6502(0x96), 2); // STX zp,Y
        assert_eq!(op_len_6502(0xBE), 3); // LDX abs,Y
    }

    /// $8000: LDA #1; BNE $8006; INC $10; $8006 RTS; data; $8008 DEC $11; RTS; data; $800C JSR $8008; RTS
    const SWEEP: [u8; 16] = [0xA9, 0x01, 0xD0, 0x02, 0xE6, 0x10, 0x60, 0xFF, 0xC6, 0x11, 0x60, 0xFF, 0x20, 0x08, 0x80, 0x60];

    fn ran_at(offsets: &[usize]) -> Vec<u8> {
        let mut f = vec![0; SWEEP.len()];
        for o in offsets {
            f[*o] = crate::coverage::FLAG_OPCODE;
        }
        f
    }

    #[test]
    fn fixed_prg_needs_an_unbankable_nes_rom() {
        let rom = |banks: u8| {
            let mut r = b"NES\x1a".to_vec();
            r.extend([banks, 0, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            r
        };
        let m32 = FixedPrg::of_rom(&rom(2)).unwrap();
        assert_eq!((m32.cpu_of(0x10), m32.offset_of(0x8010), m32.offset_of(0x7FFF)), (Some(0x8010), Some(0x10), None));
        let m16 = FixedPrg::of_rom(&rom(1)).unwrap();
        assert_eq!((m16.cpu_of(0x10), m16.offset_of(0x8010), m16.offset_of(0xC010)), (Some(0xC010), Some(0x10), Some(0x10)));
        assert_eq!(FixedPrg::of_rom(&rom(4)), None);
        assert_eq!(FixedPrg::of_rom(&[0; 32]), None);
    }

    #[test]
    fn executed_bytes_include_operands_and_leave_ranges() {
        let f = ran_at(&[0, 2, 6]);
        let e = executed_bytes_6502(&SWEEP, &f);
        assert_eq!(&e[..5], &[true, true, true, true, false]);
        assert_eq!(uncovered_ranges(&e), vec![(4, 6), (7, 16)]);
        assert_eq!(uncovered_ranges(&[true, true]), vec![]);
    }

    fn facts(name: &str, cpu: Option<&str>, file_offset: Option<u64>) -> RegionFacts {
        RegionFacts { name: name.into(), size: 0x8000, cpu: cpu.map(String::from), has_data: file_offset.is_some(), file_offset }
    }

    #[test]
    fn mips_flow_kinds() {
        let pc = 0x8000_1004;
        assert_eq!(flow_mips(0x27BD_FFE8, pc), MipsFlow::Next); // addiu $sp,$sp,-0x18
        assert_eq!(flow_mips(0x1080_0003, pc), MipsFlow::Branch(0x8000_1014)); // beq $a0,$0,+3
        assert_eq!(flow_mips(0x5080_FFFF, pc), MipsFlow::Branch(0x8000_1004)); // beql $a0,$0,-1
        assert_eq!(flow_mips(0x1000_0002, pc), MipsFlow::Jump(Some(0x8000_1010))); // b +2
        assert_eq!(flow_mips(0x0411_0004, pc), MipsFlow::Call(Some(0x8000_1018))); // bal +4
        assert_eq!(flow_mips(0x0C00_0800, pc), MipsFlow::Call(Some(0x8000_2000))); // jal 0x80002000
        assert_eq!(flow_mips(0x0800_0400, pc), MipsFlow::Jump(Some(0x8000_1000))); // j 0x80001000
        assert_eq!(flow_mips(0x03E0_0008, pc), MipsFlow::Return); // jr $ra
        assert_eq!(flow_mips(0x0100_0008, pc), MipsFlow::Jump(None)); // jr $t0 (a jump table)
        assert_eq!(flow_mips(0x0100_F809, pc), MipsFlow::Call(None)); // jalr $t0
        assert_eq!(flow_mips(0x4200_0018, pc), MipsFlow::Return); // eret
        assert_eq!(flow_mips(0x4501_0003, pc), MipsFlow::Branch(0x8000_1014)); // bc1t +3
        assert_eq!(flow_mips(0x0000_000D, pc), MipsFlow::Stop); // break
    }

    #[test]
    fn mips_blocks_end_after_the_delay_slot() {
        // 80001000 addiu $sp,-0x18; beq $a0,$0,+3; nop; jal 0x80002000; nop; jr $ra; addiu $sp,0x18
        let words: [u32; 7] = [0x27BD_FFE8, 0x1080_0003, 0, 0x0C00_0800, 0, 0x03E0_0008, 0x27BD_0018];
        let mut rom = vec![0xFFu8; 0x10];
        rom.extend(words.iter().flat_map(|w| w.to_be_bytes()));
        let f = Function { name: "f".into(), region: "RDRAM".into(), offset: 0x1000, size: 28, cpu_addr: Some(0x8000_1000), file_offset: Some(0x10) };
        let a = &analyze_mips_with(std::slice::from_ref(&f), &rom, &[])[0];
        assert_eq!(a.instructions.len(), 7);
        let blocks: Vec<(u64, usize)> = a.blocks.iter().map(|b| (b.offset, b.instructions.len())).collect();
        // The branch and its slot; the call, its slot (calls don't end blocks); the return.
        assert_eq!(blocks, vec![(0x1000, 3), (0x100C, 2), (0x1014, 2)]);
        assert_eq!(a.successors, vec![vec![0x100C, 0x1014], vec![0x1014], vec![]]);
        assert_eq!(a.calls, vec![CallSite { at: 0x100C, target: 0x8000_2000, kind: "JAL" }]);
        // Without a file offset there are no bytes to read; data isn't decoded.
        assert!(analyze_mips_with(&[Function { file_offset: None, ..f.clone() }], &rom, &[]).is_empty());
        let a = &analyze_mips_with(std::slice::from_ref(&f), &rom, &[(0x1008, 0x100B)])[0];
        assert_eq!(a.instructions.len(), 6);
    }

    #[test]
    fn csv_with_a_file_offset() {
        let p = std::env::temp_dir().join(format!("dj-csv-{}.csv", std::process::id()));
        std::fs::write(&p, "name,region,offset,size,cpu,file\nf,RDRAM,1000,1C,80001000,10\ng,PRG,0,4,8000\n").unwrap();
        let fs = load_csv(&p).unwrap();
        assert_eq!((fs[1].region.as_str(), fs[1].file_offset, fs[1].cpu_addr), ("RDRAM", Some(0x10), Some(0x8000_1000)));
        assert_eq!(fs[0].file_offset, None);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn code_region_from_connector_facts() {
        // Nothing recorded: the NES default.
        assert_eq!(code_region(&[]), "PRG");
        // Mesen: RAM first, PRG is the region from the file with a decoder.
        let mesen = [facts("RAM", Some("6502"), None), facts("PRG", Some("6502"), Some(16)), facts("WRAM", Some("6502"), None)];
        assert_eq!(code_region(&mesen), "PRG");
        // A region from the file with no decoder yet is still where coverage lies.
        assert_eq!(code_region(&[facts("RDRAM", Some("mips"), None), facts("ROM", Some("mips"), Some(0))]), "ROM");
        // The N64 connector: code runs from RDRAM; the ROM region has no CPU.
        assert_eq!(code_region(&[facts("RDRAM", Some("mips3"), None), facts("ROM", None, Some(0))]), "RDRAM");
        assert!(has_decoder("6502") && !has_decoder("mips") && !has_decoder("65816"));
    }

    #[test]
    fn executed_bytes_prefer_operand_flags() {
        // LDA #1 (2 bytes) ran. Without operand flags the length comes from the opcode;
        // with them, the flags say it.
        let mut f = vec![0u8; SWEEP.len()];
        f[0] = 0x01;
        let (e, from_flags) = executed_bytes(&SWEEP, &f);
        assert!(!from_flags);
        assert_eq!(&e[..3], &[true, true, false]);
        f[1] = 0x02;
        let (e, from_flags) = executed_bytes(&SWEEP, &f);
        assert!(from_flags);
        assert_eq!(&e[..3], &[true, true, false]);
    }

    #[test]
    fn check_flags_finds_disagreements() {
        // C000 LDA #1; BNE +2; INC $10; RTS.
        let rom = [0xA9, 0x01, 0xD0, 0x02, 0xE6, 0x10, 0x60];
        let f = Function { name: "f".into(), region: "PRG".into(), offset: 0, size: rom.len() as u64, cpu_addr: Some(0xC000), file_offset: None };
        let a = analyze_6502(&[f], &rom);
        // LDA and BNE ran, flagged as the decoder reads them.
        let mut flags = vec![0x01, 0x02, 0x01, 0x02, 0, 0, 0];
        let r = check_flags(&a, &flags, &[]);
        assert_eq!((r.operand_flags, r.instructions_checked), (true, 2));
        assert!(r.opcode_not_at_start.is_empty() && r.operand_unexpected.is_empty() && r.operand_missing.is_empty());
        // An opcode inside the BNE's operand, a missing operand flag, a stray one.
        flags[3] = 0x01;
        flags[5] = 0x02;
        let r = check_flags(&a, &flags, &[]);
        assert_eq!((r.opcode_not_at_start.clone(), r.operand_missing.clone(), r.operand_unexpected.clone()), (vec![3], vec![3], vec![5]));
        // Skipped bytes aren't checked; both widths of a mode pair are counted.
        let r = check_flags(&a, &flags, &[(3, 5)]);
        assert!(r.opcode_not_at_start.is_empty() && r.operand_missing.is_empty() && r.operand_unexpected.is_empty());
        flags[0] |= 0x04 | 0x08;
        assert_eq!(check_flags(&a, &flags, &[(3, 5)]).both_widths, 1);
        // A core with no operand flags: only opcodes are checked.
        let r = check_flags(&a, &[0x01, 0, 0x01, 0, 0, 0, 0], &[]);
        assert!(!r.operand_flags && r.operand_missing.is_empty());
    }

    #[test]
    fn blocks_split_at_branches() {
        // C000 LDA $10; CMP #5; BNE C00A; INC $11; RTS; C00A DEC $11; RTS
        let rom = [0xA5, 0x10, 0xC9, 0x05, 0xD0, 0x04, 0xE6, 0x11, 0x60, 0x00, 0xC6, 0x11, 0x60];
        // Note: byte 9 (0x00) is a 1-byte BRK placeholder so the target lands at C00A.
        let f = Function { name: "f".into(), region: "PRG".into(), offset: 0, size: rom.len() as u64, cpu_addr: Some(0xC000), file_offset: None };
        let a = &analyze_6502(&[f], &rom)[0];
        let starts: Vec<u64> = a.blocks.iter().map(|b| b.offset).collect();
        assert_eq!(starts, vec![0, 6, 9, 10]);
    }

    #[test]
    fn decoding_skips_data() {
        // C000 JSR $AC82; .word C009, C00B; C007 (two stray bytes) ; C009 INC $11; RTS; C00C DEC $11; RTS
        // Read as code, the table's $09 $C0 would be ORA #$C0 and swallow the real code's start.
        let rom = [0x20, 0x82, 0xAC, 0x09, 0xC0, 0x0C, 0xC0, 0x00, 0x00, 0xE6, 0x11, 0x60, 0xC6, 0x11, 0x60];
        let f = Function { name: "f".into(), region: "PRG".into(), offset: 0, size: rom.len() as u64, cpu_addr: Some(0xC000), file_offset: None };
        let a = &analyze_6502_with(std::slice::from_ref(&f), &rom, &[(3, 8)])[0];
        let insns: Vec<u64> = a.instructions.iter().map(|i| i.offset).collect();
        assert_eq!(insns, vec![0, 9, 11, 12, 14]);
        let starts: Vec<u64> = a.blocks.iter().map(|b| b.offset).collect();
        assert_eq!(starts, vec![0, 9, 12]);
        // Data starting inside an instruction cuts it off there.
        let a = &analyze_6502_with(std::slice::from_ref(&f), &rom, &[(1, 8)])[0];
        assert_eq!(a.instructions.iter().map(|i| i.offset).collect::<Vec<_>>(), vec![9, 11, 12, 14]);
        // Without the annotation the sweep reads the table as code.
        let a = &analyze_6502(&[f], &rom)[0];
        assert!(a.instructions.iter().any(|i| i.offset == 3));
    }

    fn temp(name: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dj-sym-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    fn summary(f: &[Function]) -> Vec<(String, u64, u64, Option<u64>)> {
        f.iter().map(|f| (f.name.clone(), f.offset, f.size, f.cpu_addr)).collect()
    }

    #[test]
    fn splat_symbol_addrs() {
        let p = temp(
            "symbol_addrs.txt",
            "// comment line\n\
             func_80000400 = 0x80000400; // type:func rom:0x1000 size:0x20\n\
             main_loop = 0x80000420; // rom:0x1020\n\
             D_80002000 = 0x80002000; // type:data rom:0x3000\n\
             no_rom = 0x80000500; // type:func\n\
             skipped = 0x80000460; // type:func rom:0x1060 ignore:true\n\
             caller = 0x80000480; // type:func rom:0x1080 size:0x10\n",
        );
        let f = load_splat(&p, 0, "ROM").unwrap();
        assert_eq!(
            summary(&f),
            vec![("func_80000400".into(), 0x1000, 0x20, Some(0x80000400)), ("main_loop".into(), 0x1020, 0x60, Some(0x80000420)), ("caller".into(), 0x1080, 0x10, Some(0x80000480))]
        );
        assert_eq!(f[0].region, "ROM");
    }

    #[test]
    fn mesen_labels() {
        let p = temp("game.mlb", "P:0197:doStateAction\nP:0100:reset:Power-on\\nentry\nR:0021:frameCount\nP:0200-020F:table\nNesPrgRom:0300:nmi\nP:0250:\n");
        let f = load_mesen_mlb(&p, "PRG").unwrap();
        assert_eq!(summary(&f), vec![("reset".into(), 0x100, 0x97, None), ("doStateAction".into(), 0x197, 0x169, None), ("nmi".into(), 0x300, LAST_FUNCTION_MAX, None)]);
    }

    #[test]
    fn ld_map() {
        // As GNU ld 2.42 writes it, including a name too long for its column.
        let map = r#"Memory Configuration

Name             Origin             Length             Attributes
*default*        0x0000000000000000 0xffffffffffffffff

Linker script and memory map


.main           0x0000000080000400       0x3c load address 0x0000000000001000
 *(.text)
 .text          0x0000000080000400        0x0 a.o
 *(.text.*)
 .text.helper   0x0000000080000400        0x9 a.o
                0x0000000080000400                helper
 .text.main_loop
                0x0000000080000409       0x29 a.o
                0x0000000080000409                main_loop
 .text.caller   0x0000000080000432        0xa a.o
                0x0000000080000432                caller
                0x0000000080000440                . = ALIGN (0x10)

.data           0x0000000080002000        0x8 load address 0x0000000000003000
 .data          0x0000000080002000        0x8 a.o
                0x0000000080002000                some_table

/DISCARD/
 *(*)
LOAD a.o
OUTPUT(a.elf elf64-x86-64)
"#;
        let p = temp("a.map", map);
        let f = load_ld_map(&p, 0, "ROM").unwrap();
        assert_eq!(summary(&f), vec![("helper".into(), 0x1000, 9, Some(0x80000400)), ("main_loop".into(), 0x1009, 0x29, Some(0x80000409)), ("caller".into(), 0x1032, 0xa, Some(0x80000432))]);
    }

    #[test]
    fn formats_from_names() {
        assert_eq!(guess_format(Path::new("x/thwaite.dbg")), "ca65-dbg");
        assert_eq!(guess_format(Path::new("symbol_addrs.txt")), "splat");
        assert_eq!(guess_format(Path::new("game.mlb")), "mesen-mlb");
        assert_eq!(guess_format(Path::new("build/us.map")), "ld-map");
        assert_eq!(guess_format(Path::new("build/us.elf")), "elf");
        assert_eq!(guess_format(Path::new("funcs.csv")), "csv");
        assert_eq!(known_format(Path::new("funcs.csv")), Some("csv"));
        assert_eq!(known_format(Path::new("notes.txt")), None);
        assert_eq!(guess_format(Path::new("notes.txt")), "csv");
    }
}
