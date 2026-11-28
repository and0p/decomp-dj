//! Scratch tool for the Thwaite decomp (.agents/_archive/THWAITE_DECOMP_PLAN.md): plays a segment with
//! the decomp beside the original, through dj's own `Checker` as dj-gui does, and saves both
//! pictures side by side (original left, decomp right) at the frames asked for. The decomp
//! draws with OpenGL from inside the baseline's frame, so this also checks nested contexts.
//!
//! Usage: thwaite_pictures <project> <rom> <baseline core> <segment> <frame,frame,...> <out dir> [<frame,...>]
//!
//! The last list, if given: frames before which the decomp copies the original again, as
//! after a rewind (Sync now in dj-gui).
//!
//! The decomp draws its flat picture here, whatever the project sets, so the pictures can be
//! compared pixel for pixel. THW_OPTIONS="key=value,..." sets its core options instead (for
//! example thwaite_decomp_render=voxel).
use anyhow::{anyhow, Result};
use dj_core::project::Project;
use dj_core::compare::{Comparison, OpenSpec};
use dj_core::libretro::{write_png, Frame};
use dj_core::parity::{Checked, Checker};
use dj_core::player::Session;
use std::path::Path;

fn side_by_side(a: &Frame, b: &Frame) -> Frame {
    let (w, h) = (a.width + b.width, a.height.max(b.height));
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    for (f, x0) in [(a, 0), (b, a.width)] {
        for y in 0..f.height {
            let src = &f.rgb[(y * f.width * 3) as usize..((y + 1) * f.width * 3) as usize];
            let at = ((y * w + x0) * 3) as usize;
            rgb[at..at + src.len()].copy_from_slice(src);
        }
    }
    Frame { width: w, height: h, rgb }
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 7 && a.len() != 8 {
        return Err(anyhow!("usage: thwaite_pictures <project> <rom> <baseline core> <segment> <frame,...> <out dir> [<copy at frame,...>]"));
    }
    let col = Project::open(Path::new(&a[1]))?;
    let id = col.resolve(&a[4])?.to_string();
    let shots: Vec<u64> = a[5].split(',').map(|s| s.parse()).collect::<std::result::Result<_, _>>()?;
    let copies: Vec<u64> = a.get(7).map(|c| c.split(',').map(|s| s.parse()).collect()).transpose()?.unwrap_or_default();
    let out = Path::new(&a[6]);
    std::fs::create_dir_all(out)?;
    let n = col.segments[&id].frames()?;
    let mut s = Session::open_with(&col, Path::new(&a[2]), Path::new(&a[3]), false)?;
    let mut spec = OpenSpec::resolve(&col, "decomp", Path::new(&a[3]), Some(Path::new(&a[2])))?;
    spec.options.insert("thwaite_decomp_render".into(), "flat".into());
    for kv in std::env::var("THW_OPTIONS").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').ok_or_else(|| anyhow!("THW_OPTIONS: {kv} isn't key=value"))?;
        spec.options.insert(k.into(), v.into());
    }
    let mut cmp = Comparison::open_spec(&spec)?;
    let power_on = s.enter_segment(&col, &id)?;
    let mut ck = Checker::default();
    let mut last_decomp: Option<Frame> = None;
    for k in 0..n {
        if !ck.in_step(&id, k) {
            ck.sync(&mut cmp, &mut s.core, &col, &id, k, power_on + k).map_err(|e| anyhow!(e))?;
        }
        if copies.contains(&k) {
            ck.copy_again();
        }
        let want = shots.contains(&k);
        ck.begin(&mut cmp, &mut s.core, &col, &id, k, true, true);
        let base = s.step(&col, &id, k, want)?;
        let st = ck.step(&mut cmp, &mut s.core, &col, &id, k, power_on + k, true, true).ok_or_else(|| anyhow!("out of step at frame {k}"))?;
        if let Some(p) = st.picture {
            last_decomp = Some(p);
        }
        match &st.checked {
            Checked::Failed(why) => println!("frame {k}: FAILED {why}"),
            Checked::Drifting(why) => println!("frame {k}: drifting {why}"),
            _ => {}
        }
        if let (true, Some(b), Some(d)) = (want, base, last_decomp.as_ref()) {
            let path = out.join(format!("{}-{k:06}.png", &id[..8]));
            write_png(&side_by_side(&b, d), &path)?;
            println!("frame {k}: {}", path.display());
        }
    }
    Ok(())
}
