//! ramdiff: plays segment chains on two libretro cores, frame by frame, and reports the
//! first frames where their RAM differs.
//!
//! usage: ramdiff <project> <rom> <core A> <core B> [segment...] [--names ram_map.h]
//!                [--ignore 0100-01FF,...] [--max N] [--context N]
//!
//! With no segment, every leaf segment of the project is played. A segment's chain is
//! played from power-on, so each leaf checks all of its ancestors too; a chain already
//! checked up to a shared ancestor is not reported twice.
//!
//! The stack page ($0100-$01FF) is ignored by default: the port does not reproduce the
//! return addresses and pushed registers the 6502 leaves there.
//!
//! decomp-dj allows one libretro core per process, so each core plays in a child process
//! (this program, run as `ramdiff --dump <project> <rom> <core> <leaf>`), which writes the
//! RAM after every frame of the chain to its standard output.
use anyhow::{bail, Context, Result};
use decomp_dj::collection::Collection;
use decomp_dj::libretro::region_memory_id;
use decomp_dj::player::Session;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Args {
    project: PathBuf,
    rom: PathBuf,
    cores: [PathBuf; 2],
    segments: Vec<String>,
    names: Option<PathBuf>,
    ignore: Vec<(usize, usize)>,
    max: usize,
}

fn parse() -> Result<Args> {
    let mut pos = vec![];
    let mut names = None;
    let mut ignore = vec![(0x100, 0x1FF)];
    let mut max = 1;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--names" => names = Some(PathBuf::from(it.next().context("--names FILE")?)),
            "--max" => max = it.next().context("--max N")?.parse()?,
            "--ignore" => {
                ignore.clear();
                for r in it.next().context("--ignore RANGES")?.split(',').filter(|s| !s.is_empty()) {
                    let (a, b) = r.split_once('-').unwrap_or((r, r));
                    ignore.push((usize::from_str_radix(a, 16)?, usize::from_str_radix(b, 16)?));
                }
            }
            _ => pos.push(a),
        }
    }
    if pos.len() < 4 {
        bail!("usage: ramdiff <project> <rom> <core A> <core B> [segment...] [--names ram_map.h] [--ignore 0100-01FF] [--max N]");
    }
    Ok(Args {
        project: pos[0].clone().into(),
        rom: pos[1].clone().into(),
        cores: [pos[2].clone().into(), pos[3].clone().into()],
        segments: pos[4..].to_vec(),
        names,
        ignore,
        max,
    })
}

/// Names for RAM addresses, from the port's generated ram_map.h (`#define A_name 0x1234`).
fn load_names(p: &Path) -> Result<BTreeMap<usize, String>> {
    let mut m = BTreeMap::new();
    for line in std::fs::read_to_string(p)?.lines() {
        let mut w = line.split_whitespace();
        if let (Some("#define"), Some(n), Some(v)) = (w.next(), w.next(), w.next()) {
            if let (Some(n), Some(v)) = (n.strip_prefix("A_"), v.strip_prefix("0x")) {
                m.entry(usize::from_str_radix(v, 16)?).or_insert_with(|| n.to_string());
            }
        }
    }
    Ok(m)
}

fn name_of(names: &BTreeMap<usize, String>, a: usize) -> String {
    match names.range(..=a).next_back() {
        Some((&b, n)) if a == b => n.clone(),
        Some((&b, n)) if a - b < 0x100 && !names.contains_key(&a) => format!("{n}+{}", a - b),
        _ => String::new(),
    }
}

/// Child mode: plays `leaf`'s chain from power-on on one core and writes the RAM after
/// every frame to stdout, as raw bytes.
fn dump(project: &Path, rom: &Path, core: &Path, leaf: &str) -> Result<()> {
    let col = Collection::open(project)?;
    let mut session = Session::open_with(&col, rom, core, false).with_context(|| format!("opening {}", core.display()))?;
    let ram_id = region_memory_id("RAM").unwrap();
    let chain: Vec<String> = col.chain(leaf)?.into_iter().map(String::from).collect();
    session.start_trunk(&col, &col.replays[&chain[0]])?;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for seg in &chain {
        session.restore_inputs(&col, seg, 0)?;
        for f in 0..col.replays[seg].frames()? {
            session.step(&col, seg, f, false)?;
            let ram = session.core.memory(ram_id).context("the core exposes no RAM")?;
            out.write_all(ram)?;
        }
    }
    out.flush()?;
    Ok(())
}

/// Runs a child `--dump` for one core and returns its frames of RAM, concatenated.
fn play_in_child(args: &Args, core: &Path, leaf: &str) -> Result<Vec<u8>> {
    let out = Command::new(std::env::current_exe()?)
        .arg("--dump")
        .args([args.project.as_os_str(), args.rom.as_os_str(), core.as_os_str()])
        .arg(leaf)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("running the {} child", core.display()))?;
    if !out.status.success() {
        bail!("playing {leaf} on {} failed", core.display());
    }
    Ok(out.stdout)
}

fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("--dump") {
        if argv.len() != 6 {
            bail!("usage: ramdiff --dump <project> <rom> <core> <leaf>");
        }
        return dump(Path::new(&argv[2]), Path::new(&argv[3]), Path::new(&argv[4]), &argv[5]);
    }
    let args = parse()?;
    let col = Collection::open(&args.project)?;
    let names = match &args.names {
        Some(p) => load_names(p)?,
        None => BTreeMap::new(),
    };
    let leaves: Vec<String> = if args.segments.is_empty() {
        col.replays.keys().filter(|id| col.children(Some(id)).is_empty()).cloned().collect()
    } else {
        args.segments.iter().map(|s| col.resolve(s).map(String::from)).collect::<Result<_>>()?
    };
    let ignored = |a: usize| args.ignore.iter().any(|&(lo, hi)| a >= lo && a <= hi);
    let mut checked: HashSet<String> = HashSet::new();
    let mut failures = 0;
    for leaf in &leaves {
        let chain: Vec<String> = col.chain(leaf)?.into_iter().map(String::from).collect();
        let a_all = play_in_child(&args, &args.cores[0], leaf)?;
        let b_all = play_in_child(&args, &args.cores[1], leaf)?;
        let total: u64 = chain.iter().map(|s| col.replays[s].frames()).sum::<Result<u64>>()?;
        if total == 0 {
            continue;
        }
        let size_a = a_all.len() / total as usize;
        let size_b = b_all.len() / total as usize;
        let mut global = 0usize;
        let mut reported = 0;
        'chain: for seg in &chain {
            let title = col.replays[seg].title().to_string();
            let frames = col.replays[seg].frames()?;
            let mut seg_ok = true;
            for f in 0..frames {
                let a = &a_all[global * size_a..(global + 1) * size_a];
                let b = &b_all[global * size_b..(global + 1) * size_b];
                global += 1;
                let diffs: Vec<usize> = (0..a.len().min(b.len())).filter(|&i| a[i] != b[i] && !ignored(i)).collect();
                if !diffs.is_empty() {
                    seg_ok = false;
                    if !checked.contains(seg) {
                        println!("DIFF {} \"{}\" frame {} (from power-on {}): {} bytes", &seg[..8], title, f, global, diffs.len());
                        for &i in diffs.iter().take(24) {
                            println!("  ${i:04X} {:<32} A={:02X} B={:02X}", name_of(&names, i), a[i], b[i]);
                        }
                        reported += 1;
                        failures += 1;
                    }
                    if reported >= args.max {
                        break 'chain;
                    }
                }
            }
            if seg_ok && checked.insert(seg.clone()) {
                println!("ok   {} \"{}\" {} frames", &seg[..8], title, frames);
            }
        }
    }
    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}
