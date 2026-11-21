//! Scratch tool: plays a node's chain from power-on and prints chosen RAM bytes per frame.
//! Usage: ramtrace <collection> <id> <hexaddr,...> [every]
use dj_core::collection::Collection;
use dj_core::libretro::region_memory_id;
use dj_core::player::Session;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let col = Collection::open(Path::new(&a[1]))?;
    let id = col.resolve(&a[2])?.to_string();
    let addrs: Vec<usize> = a[3].split(',').map(|s| usize::from_str_radix(s, 16).unwrap()).collect();
    let every: u64 = a.get(4).map_or(1, |s| s.parse().unwrap());
    let local = col.load_local()?;
    let mut s = Session::open(&col, local.rom.as_ref().unwrap(), local.core.as_ref().unwrap())?;
    let chain: Vec<String> = col.chain(&id)?.into_iter().map(String::from).collect();
    s.start_trunk(&col, &col.replays[&chain[0]])?;
    let mut g = 0u64;
    let ram = region_memory_id("RAM").unwrap();
    for c in &chain {
        s.restore_inputs(&col, c, 0)?;
        let n = col.replays[c].frames()?;
        for f in 0..n {
            s.step(&col, c, f, false)?;
            g += 1;
            if g % every == 0 || f + 1 == n {
                let m = s.core.memory(ram).unwrap();
                let v: Vec<String> = addrs.iter().map(|x| format!("{:02X}", m[*x])).collect();
                println!("{g:6} {:<28} {:4} {}", &col.replays[c].title().chars().take(28).collect::<String>(), f, v.join(" "));
            }
        }
    }
    Ok(())
}
