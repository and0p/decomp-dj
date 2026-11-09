//! Scratch tool for .agents/CONNECTOR_PLAN.md, step 3: is a core deterministic from power-on,
//! and does a save state round-trip (save at K, run on, load, run again) reproduce the same
//! RAM hashes? Hashes libretro's SYSTEM_RAM with blake3 after every frame, as dj does.
//!
//! Usage:
//!   n64_roundtrip <core> <rom> <frames> <k,k,...> <m> [KEY=VALUE ...]
//!     runs from power-on twice, then for each K: save at K, run M frames, load, run M again.
//!   n64_roundtrip <core> <rom> save <k> <state-file> <hash-file> <m> [KEY=VALUE ...]
//!     runs to K, writes the state, then the hashes of K+1..=K+M (one per line).
//!   n64_roundtrip <core> <rom> load <state-file> <m> [KEY=VALUE ...]
//!     loads the state into a fresh process and prints the hashes of the next M frames.
use decomp_dj::libretro::{Core, MEMORY_SYSTEM_RAM};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

fn hash(core: &mut Core) -> String {
    let ram = core.memory(MEMORY_SYSTEM_RAM).expect("the core exposes no SYSTEM_RAM");
    blake3::hash(ram).to_hex()[..16].to_string()
}

fn run(core: &mut Core, n: u64) -> Vec<String> {
    (0..n)
        .map(|_| {
            core.run_frame(false);
            hash(core)
        })
        .collect()
}

fn first_diff(a: &[String], b: &[String]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

fn open(core: &Path, rom: &Path, options: &BTreeMap<String, String>) -> anyhow::Result<Core> {
    let work = std::env::temp_dir().join(format!("dj-n64-roundtrip-{}", std::process::id()));
    std::fs::create_dir_all(&work)?;
    let verbose = std::env::var_os("DJ_VERBOSE").is_some();
    if verbose { eprintln!("loading {}", core.display()); }
    let mut c = Core::load(core, &work, options)?;
    if verbose { eprintln!("loading the game"); }
    c.load_game(rom)?;
    if verbose { eprintln!("loaded"); }
    Ok(c)
}

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (core_path, rom) = (Path::new(&a[1]), Path::new(&a[2]));
    let opts = |from: usize| -> BTreeMap<String, String> {
        a[from..].iter().filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
    };
    match a[3].as_str() {
        // exp <k> <m> <j,j,...> [opts]: save at K; reference = the M frames run straight after.
        // For each J: a new instance runs to K, saves, runs J more frames (J = "twice": none,
        // and loads twice), loads, runs M; prints the first frame that differs and the
        // RDRAM ranges that differ there, with WWF No Mercy's layout (aki_n64_decomp README).
        "exp" => {
            let k: u64 = a[4].parse()?;
            let m: u64 = a[5].parse()?;
            let options = opts(7);
            let to_save = |extra: u64, twice: bool, frames: u64| -> anyhow::Result<(Vec<String>, Vec<u8>)> {
                let mut c = open(core_path, rom, &options)?;
                run(&mut c, k);
                let state = c.serialize()?;
                if extra > 0 || twice {
                    run(&mut c, extra);
                    c.unserialize(&state)?;
                    if twice {
                        c.unserialize(&state)?;
                    }
                }
                let h = run(&mut c, frames);
                Ok((h, c.memory(MEMORY_SYSTEM_RAM).unwrap().to_vec()))
            };
            let (reference, _) = to_save(0, false, m)?;
            for j in a[6].split(',') {
                let (extra, twice) = if j == "twice" { (0, true) } else { (j.parse::<u64>()?, false) };
                if extra == 0 && !twice {
                    // Load straight after saving (no frames between).
                    let mut c = open(core_path, rom, &options)?;
                    run(&mut c, k);
                    let state = c.serialize()?;
                    c.unserialize(&state)?;
                    let h = run(&mut c, m);
                    println!("J=0 (load right after saving): {}", first_diff(&reference, &h).map_or("identical".into(), |i| format!("FIRST DIFFERENCE at frame {} after the load", i + 1)));
                    continue;
                }
                let (h, _) = to_save(extra, twice, m)?;
                match first_diff(&reference, &h) {
                    None => println!("J={j}: identical for {m} frames"),
                    Some(d) => {
                        let (_, want) = to_save(0, false, d as u64 + 1)?;
                        let (_, got) = to_save(extra, twice, d as u64 + 1)?;
                        let mut ranges: Vec<(usize, usize)> = vec![];
                        for (i, (x, y)) in want.iter().zip(&got).enumerate() {
                            if x != y {
                                match ranges.last_mut() {
                                    Some(r) if i <= r.1 + 16 => r.1 = i,
                                    _ => ranges.push((i, i)),
                                }
                            }
                        }
                        let bytes: usize = want.iter().zip(&got).filter(|(x, y)| x != y).count();
                        println!("J={j}: FIRST DIFFERENCE at frame {} after the load; {bytes} bytes differ in {} ranges", d + 1, ranges.len());
                        let mut blocks: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
                        for (i, (x, y)) in want.iter().zip(&got).enumerate() {
                            if x != y {
                                *blocks.entry(i >> 16).or_default() += 1;
                            }
                        }
                        println!("    by 64 KB block: {}", blocks.iter().map(|(b, n)| format!("0x{:08X}:{n}", 0x8000_0000u64 + ((*b as u64) << 16))).collect::<Vec<_>>().join(" "));
                        for (s, e) in ranges.iter().take(4) {
                            let v = 0x8000_0000u64 + *s as u64;
                            let seg = match v {
                                0x8000_0450..0x8004_0380 => "main text",
                                0x8004_0380..0x8005_6670 => "main data",
                                0x8005_6670..0x800B_D960 => "main bss",
                                0x800D_9960..0x8014_4670 => "overlay text",
                                0x8014_4670..0x8015_1ED0 => "overlay data",
                                0x8015_1ED0..0x8016_B9F0 => "overlay bss",
                                _ => "outside the known segments",
                            };
                            println!("    0x{v:08X}-0x{:08X} ({} bytes, {seg})", 0x8000_0000u64 + *e as u64, e - s + 1);
                        }
                    }
                }
            }
        }
        "save" => {
            let k: u64 = a[4].parse()?;
            let m: u64 = a[7].parse()?;
            let mut c = open(core_path, rom, &opts(8))?;
            run(&mut c, k);
            std::fs::write(&a[5], c.serialize()?)?;
            std::fs::write(&a[6], run(&mut c, m).join("\n") + "\n")?;
        }
        "load" => {
            let m: u64 = a[5].parse()?;
            let mut c = open(core_path, rom, &opts(6))?;
            // A frame first, as dj does before restoring: some cores only take a state once
            // they're running.
            c.run_frame(false);
            c.unserialize(&std::fs::read(&a[4])?)?;
            println!("{}", run(&mut c, m).join("\n"));
        }
        _ => {
            let n: u64 = a[3].parse()?;
            let ks: Vec<u64> = a[4].split(',').map(|s| s.parse().unwrap()).collect();
            let m: u64 = a[5].parse()?;
            let options = opts(6);
            let mut c = open(core_path, rom, &options)?;
            let t = Instant::now();
            let first = run(&mut c, n);
            let fps = n as f64 / t.elapsed().as_secs_f64();
            drop(c);
            let mut c = open(core_path, rom, &options)?;
            let second = run(&mut c, n);
            let distinct = first.iter().collect::<std::collections::BTreeSet<_>>().len();
            println!("power-on x2, {n} frames, {fps:.0} fps, {distinct} distinct RAM hashes: {}", match first_diff(&first, &second) {
                None => "identical".to_string(),
                Some(i) => format!("FIRST DIFFERENCE at frame {}", i + 1),
            });
            for k in ks {
                drop(c);
                c = open(core_path, rom, &options)?;
                run(&mut c, k);
                let state = c.serialize()?;
                let after = run(&mut c, m);
                c.unserialize(&state)?;
                let again = run(&mut c, m);
                let straight = &first[k as usize..(k + m) as usize];
                // A save or load that runs the core on by whole frames shows as a shift.
                let shift = |xs: &[String]| (0..4usize).find(|d| xs.iter().take(m as usize - 4).enumerate().all(|(i, h)| first.get(k as usize + i + d) == Some(h)));
                eprintln!("  K={k}: run on matches the power-on run shifted by {:?} frames; after load by {:?}", shift(&after), shift(&again));
                println!(
                    "K={k}: state {} bytes; run on vs power-on run: {}; after load vs run on: {}",
                    state.len(),
                    first_diff(&after, straight).map_or("identical".into(), |i| format!("FIRST DIFFERENCE at K+{}", i + 1)),
                    first_diff(&again, &after).map_or("identical".into(), |i| format!("FIRST DIFFERENCE at K+{}", i + 1)),
                );
            }
        }
    }
    Ok(())
}
