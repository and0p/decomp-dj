//! annotations.toml: what a person knows about the ROM that no symbol map says
//! (docs/config.md). Three kinds of entry:
//!
//! ```toml
//! [[unreachable]]          # code no input can reach: left out of the totals
//! range = "PRG:0x018C-0x0197"
//! why = "two-player mode"
//!
//! [[data]]                 # bytes that are not code: never disassembled or counted
//! range = "PRG:0x0050-0x0059"
//! why = "jump table after jsr switch_s_plus_2a"
//!
//! [[function]]             # names a function, or adds one the maps don't have
//! at = "PRG:0x0161"
//! name = "branchOnGameMode"
//! size = 0x13              # optional
//! ```
//!
//! Ranges are inclusive region offsets (`0x8000-0x80FF`, or one byte `0x8123`); the region
//! is PRG unless given.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::analysis::Function;

pub const FILE: &str = "annotations.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Unreachable,
    Data,
    Function,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Unreachable => "unreachable",
            Kind::Data => "data",
            Kind::Function => "function",
        }
    }
}

/// One entry. `start..=end` are region offsets; a function's `end` is its last byte once
/// its size is known (`size` given, or worked out by `merge_functions`), else `start`.
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub kind: Kind,
    /// 1-based position among the entries of its kind, as error messages count them.
    pub index: usize,
    pub region: String,
    pub start: u64,
    pub end: u64,
    pub why: String,
    pub name: Option<String>,
    pub size: Option<u64>,
}

impl Entry {
    /// "data entry 3", for messages.
    pub fn label(&self) -> String {
        format!("{} entry {}", self.kind.name(), self.index)
    }

    pub fn range_text(&self) -> String {
        if self.start == self.end {
            format!("{}:{:#06X}", self.region, self.start)
        } else {
            format!("{}:{:#06X}-{:#06X}", self.region, self.start, self.end)
        }
    }

    pub fn contains(&self, region: &str, off: u64) -> bool {
        self.region == region && off >= self.start && off <= self.end
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Annotations {
    pub entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    unreachable: Vec<RangeEntry>,
    #[serde(default)]
    data: Vec<RangeEntry>,
    #[serde(default)]
    function: Vec<FunctionEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeEntry {
    range: String,
    #[serde(default)]
    why: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FunctionEntry {
    at: String,
    name: String,
    size: Option<u64>,
    #[serde(default)]
    why: String,
}

fn hex(s: &str) -> Option<u64> {
    let s = s.trim();
    let t = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).or_else(|| s.strip_prefix('$')).unwrap_or(s);
    if t.is_empty() {
        return None;
    }
    u64::from_str_radix(t, 16).ok()
}

/// `PRG:0x10-0x1F`, `0x10-0x1F` or `0x10` -> (region, start, end).
fn parse_range(s: &str) -> std::result::Result<(String, u64, u64), String> {
    let (region, rest) = match s.split_once(':') {
        Some((r, rest)) => (r.trim().to_ascii_uppercase(), rest),
        None => ("PRG".to_string(), s),
    };
    if region.is_empty() || !region.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{s:?}: {region:?} is not a region name"));
    }
    let (a, b) = match rest.split_once('-') {
        Some((a, b)) => (a, Some(b)),
        None => (rest, None),
    };
    let start = hex(a).ok_or_else(|| format!("{s:?}: {:?} is not a hex offset", a.trim()))?;
    let end = match b {
        Some(b) => hex(b).ok_or_else(|| format!("{s:?}: {:?} is not a hex offset", b.trim()))?,
        None => start,
    };
    if end < start {
        return Err(format!("{s:?}: the range ends before it starts"));
    }
    Ok((region, start, end))
}

impl Annotations {
    /// Reads `annotations.toml` from a project folder; none there is no annotations.
    pub fn load(root: &Path) -> Result<Annotations> {
        let path = root.join(FILE);
        if !path.exists() {
            // The file this replaced: better to stop than to count what it left out.
            if root.join("unreachable.txt").exists() {
                bail!("unreachable.txt is no longer read: move its ranges into {FILE} as [[unreachable]] entries (docs/config.md)");
            }
            return Ok(Annotations::default());
        }
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        Annotations::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Annotations> {
        let file: File = toml::from_str(text).map_err(|e| anyhow!("{FILE}: {}", e.message().trim()))?;
        let mut entries = vec![];
        for (kind, list) in [(Kind::Unreachable, &file.unreachable), (Kind::Data, &file.data)] {
            for (i, e) in list.iter().enumerate() {
                let (region, start, end) = parse_range(&e.range).map_err(|m| anyhow!("{FILE}, {} entry {}: range {m}", kind.name(), i + 1))?;
                entries.push(Entry { kind, index: i + 1, region, start, end, why: e.why.trim().to_string(), name: None, size: None });
            }
        }
        for (i, e) in file.function.iter().enumerate() {
            let what = || format!("{FILE}, function entry {}", i + 1);
            let (region, start, end) = parse_range(&e.at).map_err(|m| anyhow!("{}: at {m}", what()))?;
            if end != start {
                bail!("{}: `at` is one offset, not a range ({:?}); give the length as `size`", what(), e.at);
            }
            let name = e.name.trim();
            if name.is_empty() {
                bail!("{}: the name is empty", what());
            }
            if e.size == Some(0) {
                bail!("{}: size 0", what());
            }
            let end = e.size.map_or(start, |s| start + s - 1);
            entries.push(Entry { kind: Kind::Function, index: i + 1, region, start, end, why: e.why.trim().to_string(), name: Some(name.to_string()), size: e.size });
        }
        // Two functions at one place, or one name twice, is a mistake.
        let funcs: Vec<&Entry> = entries.iter().filter(|e| e.kind == Kind::Function).collect();
        for (i, a) in funcs.iter().enumerate() {
            for b in &funcs[..i] {
                if a.region == b.region && a.start == b.start {
                    bail!("{FILE}, {}: {} already has a function ({})", a.label(), a.range_text(), b.label());
                }
                if a.name == b.name {
                    bail!("{FILE}, {}: the name {} is already used by {}", a.label(), a.name.as_deref().unwrap_or(""), b.label());
                }
                if a.region == b.region && a.start <= b.end && b.start <= a.end && (a.size.is_some() && b.size.is_some()) {
                    bail!("{FILE}, {}: overlaps {}", a.label(), b.label());
                }
            }
        }
        Ok(Annotations { entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.kind == kind)
    }

    /// Inclusive ranges of one kind in one region.
    pub fn ranges(&self, kind: Kind, region: &str) -> Vec<(u64, u64)> {
        self.of_kind(kind).filter(|e| e.region == region).map(|e| (e.start, e.end)).collect()
    }

    /// The data or unreachable entry covering `off`, data first (a jump table inside dead
    /// code is still data).
    pub fn excluded_at(&self, region: &str, off: u64) -> Option<&Entry> {
        self.of_kind(Kind::Data).find(|e| e.contains(region, off)).or_else(|| self.of_kind(Kind::Unreachable).find(|e| e.contains(region, off)))
    }

    /// The symbol maps' functions with the `[[function]]` entries applied. An entry at the
    /// offset of a map's function renames it (and resizes it, given a size). Elsewhere it adds
    /// a function: given no size, it runs to the end of the map's function it falls inside
    /// (which then ends where it starts), or else to the next function. Map functions it
    /// overlaps give way: one that starts before it is cut short, one inside it is dropped,
    /// and one that runs past its end starts again after it. `cpu_of(region, offset)` gives
    /// CPU addresses for added functions that no map function places.
    pub fn merge_functions(&self, mut funcs: Vec<Function>, cpu_of: impl Fn(&str, u64) -> Option<u64>) -> Result<Vec<Function>> {
        let mut added: Vec<Function> = vec![];
        for e in self.of_kind(Kind::Function) {
            let name = e.name.clone().unwrap_or_default();
            let cpu_at = |fs: &[Function], off: u64| fs.iter().find(|f| f.region == e.region && off >= f.offset && off < f.offset + f.size).and_then(|f| f.cpu_addr.map(|c| c + (off - f.offset)));
            if let Some(f) = funcs.iter_mut().find(|f| f.region == e.region && f.offset == e.start) {
                f.name = name.clone();
                if let Some(s) = e.size {
                    f.size = s;
                }
                continue;
            }
            let size = match e.size {
                Some(s) => s,
                None => {
                    let inside = funcs.iter().find(|f| f.region == e.region && e.start > f.offset && e.start < f.offset + f.size);
                    let next = funcs.iter().chain(&added).filter(|f| f.region == e.region && f.offset > e.start).map(|f| f.offset).min();
                    let other = self.of_kind(Kind::Function).filter(|o| o.region == e.region && o.start > e.start).map(|o| o.start).min();
                    let end = match inside {
                        Some(f) => Some(f.offset + f.size),
                        None => next,
                    };
                    let end = match (end, other) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    match end {
                        Some(end) => end - e.start,
                        None => bail!("{FILE}, {}: nothing after {} to say where it ends; give a size", e.label(), e.range_text()),
                    }
                }
            };
            let cpu_addr = cpu_at(&funcs, e.start).or_else(|| cpu_of(&e.region, e.start));
            added.push(Function { name, region: e.region.clone(), offset: e.start, size, cpu_addr });
        }
        // Map functions give way to added ones (renamed ones are already in place).
        for a in &added {
            let (s, end) = (a.offset, a.offset + a.size);
            let mut tails = vec![];
            funcs.retain_mut(|f| {
                if f.region != a.region || f.offset + f.size <= s || f.offset >= end {
                    return true;
                }
                let f_end = f.offset + f.size;
                if f_end > end {
                    let mut t = f.clone();
                    t.offset = end;
                    t.size = f_end - end;
                    t.cpu_addr = f.cpu_addr.map(|c| c + (end - f.offset));
                    if f.offset < s {
                        t.name = format!("{}+{:#X}", f.name, end - f.offset);
                    }
                    tails.push(t);
                }
                if f.offset < s {
                    f.size = s - f.offset;
                    true
                } else {
                    false
                }
            });
            funcs.extend(tails);
        }
        funcs.extend(added);
        funcs.sort_by_key(|f| (f.region.clone(), f.offset));
        Ok(funcs)
    }

    /// Lines for a cache table: (kind, region, start, end, name, why).
    pub fn rows(&self) -> impl Iterator<Item = (&'static str, &str, u64, u64, Option<&str>, &str)> {
        self.entries.iter().map(|e| (e.kind.name(), e.region.as_str(), e.start, e.end, e.name.as_deref(), e.why.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn func(name: &str, offset: u64, size: u64) -> Function {
        Function { name: name.into(), region: "PRG".into(), offset, size, cpu_addr: Some(0x8000 + offset) }
    }

    #[test]
    fn parses_entries() {
        let a = Annotations::parse(
            r#"
            [[unreachable]]
            range = "PRG:0x018C-0x0197"
            why = "two players"
            [[data]]
            range = "0x50-0x59"
            [[data]]
            range = "chr:0x10"
            [[function]]
            at = "PRG:0x0161"
            name = "branchOnGameMode"
            size = 0x13
            "#,
        )
        .unwrap();
        let k: Vec<(Kind, &str, u64, u64)> = a.entries.iter().map(|e| (e.kind, e.region.as_str(), e.start, e.end)).collect();
        assert_eq!(k, vec![(Kind::Unreachable, "PRG", 0x18C, 0x197), (Kind::Data, "PRG", 0x50, 0x59), (Kind::Data, "CHR", 0x10, 0x10), (Kind::Function, "PRG", 0x161, 0x173)]);
        assert_eq!(a.entries[0].why, "two players");
        assert_eq!(a.entries[2].label(), "data entry 2");
        assert_eq!(a.excluded_at("PRG", 0x55).unwrap().label(), "data entry 1");
        assert!(a.excluded_at("PRG", 0x5A).is_none());
        assert!(Annotations::parse("").unwrap().is_empty());
    }

    #[test]
    fn errors_name_the_entry() {
        let err = |t: &str| Annotations::parse(t).unwrap_err().to_string();
        assert_eq!(err("[[data]]\nrange = \"0x10\"\n[[data]]\nrange = \"0x20-0x1F\""), "annotations.toml, data entry 2: range \"0x20-0x1F\": the range ends before it starts");
        assert!(err("[[unreachable]]\nrange = \"PRG:zz\"").starts_with("annotations.toml, unreachable entry 1: range"), "{}", err("[[unreachable]]\nrange = \"PRG:zz\""));
        assert!(err("[[function]]\nat = \"0x10-0x20\"\nname = \"f\"").contains("function entry 1: `at` is one offset"));
        assert!(err("[[function]]\nat = \"0x10\"\nname = \"f\"\n[[function]]\nat = \"0x20\"\nname = \"f\"").contains("function entry 2: the name f is already used by function entry 1"));
        assert!(err("[[data]]\nrange = \"0x10\"\nwhy = \"x\"\ncolour = 1").starts_with("annotations.toml: unknown field `colour`"));
        assert!(err("[[code]]\nrange = \"0x10\"").starts_with("annotations.toml: unknown field `code`"));
    }

    #[test]
    fn merges_functions() {
        let maps = vec![func("a", 0x00, 0x40), func("b", 0x40, 0x40), func("c", 0x80, 0x20)];
        let a = Annotations::parse(
            r#"
            [[function]]
            at = "0x40"
            name = "renamed_b"
            [[function]]
            at = "0x10"
            name = "inside_a"
            [[function]]
            at = "0x70"
            name = "spans_b_c"
            size = 0x18
            [[function]]
            at = "0x200"
            name = "loose"
            size = 4
            "#,
        )
        .unwrap();
        let merged = a.merge_functions(maps, |_, o| Some(0xC000 + o)).unwrap();
        let got: Vec<(&str, u64, u64, Option<u64>)> = merged.iter().map(|f| (f.name.as_str(), f.offset, f.size, f.cpu_addr)).collect();
        assert_eq!(
            got,
            vec![
                ("a", 0x00, 0x10, Some(0x8000)),
                ("inside_a", 0x10, 0x30, Some(0x8010)),
                ("renamed_b", 0x40, 0x30, Some(0x8040)),
                ("spans_b_c", 0x70, 0x18, Some(0x8070)),
                ("c", 0x88, 0x18, Some(0x8088)),
                ("loose", 0x200, 4, Some(0xC200)),
            ]
        );
        // With nothing after it and no size, an added function has no end.
        let a = Annotations::parse("[[function]]\nat = \"0x300\"\nname = \"x\"").unwrap();
        assert!(a.merge_functions(vec![func("a", 0, 0x10)], |_, _| None).unwrap_err().to_string().contains("function entry 1: nothing after"));
    }
}
