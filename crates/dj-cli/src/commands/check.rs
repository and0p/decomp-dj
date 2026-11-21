//! Commands that play segments and judge them: check, fill, clean.

use crate::*;

/// What a play of a segment produced, compared with the hashes its file records: ok, new
/// (none recorded yet), dirty (it started somewhere else) or changed.
pub(crate) fn judge(col: &Collection, res: &ReplayResult) -> &'static str {
    dj_core::hydrate::judge_hashes(col, &res.id, &res.start_hash, &res.end_hash, Some(&res.checkpoints))
}

pub(crate) fn save_shots(res: &ReplayResult, dir: &Path) -> Result<()> {
    for (f, frame) in &res.screenshots {
        let p = dir.join(format!("{}-{:06}.png", &res.id[..8], f));
        libretro::write_png(frame, &p)?;
        println!("  screenshot {}", p.display());
    }
    Ok(())
}

/// What `check_tree` found: segments whose hashes differ (dirty or changed), and segments
/// with none recorded ("new").
#[derive(Default)]
pub(crate) struct Checked {
    pub(crate) differ: usize,
    pub(crate) new: usize,
    /// One per segment, for a JUnit report.
    pub(crate) cases: Vec<junit::Case>,
}

/// Plays every segment, or those `pick` chooses, and compares each with its recorded hashes,
/// writing no file. A chosen segment plays from the nearest stored state of a segment above
/// it (unless `from_power_on`); every one plays from its parent's end, reached in this run.
pub(crate) fn check_tree(dir: &Path, local: &Local, pick: &parity_cmd::Pick, from_power_on: bool, shots: Option<&Path>, extra_shots: Vec<u64>) -> Result<Checked> {
    let col = Collection::open(dir)?;
    let chosen = if pick.segments.is_empty() && pick.tags.is_empty() && !pick.children { None } else { Some(pick.resolve(&col)?) };
    let started = std::time::Instant::now();
    let (mut total_frames, mut found) = (0u64, Checked::default());
    let mut s = open_session(&col, local)?;
    if !s.core.has_connector() {
        eprintln!("note: this core has no coverage connector; checking hashes only");
    }
    let opts = PlayOptions { end_screenshot: shots.is_some(), extra_shots, record: true };
    let mut last = std::time::Instant::now();
    let mut report = |res: &ReplayResult| -> Result<()> {
        let outcome = judge(&col, res);
        let seconds = std::mem::replace(&mut last, std::time::Instant::now()).elapsed().as_secs_f64();
        let o = match outcome {
            "ok" => junit::Outcome::Pass,
            "new" => junit::Outcome::Skip { message: "no recorded hashes: nothing to compare with (dj clean records them)".into() },
            _ => junit::Outcome::Fail {
                message: format!("{outcome}: it doesn't reproduce its recorded hashes"),
                body: format!("start {}\nend {}\nrecorded start {}\nrecorded end {}", res.start_hash, res.end_hash, col.replays[&res.id].get("start-hash").unwrap_or("-"), col.replays[&res.id].get("end-hash").unwrap_or("-")),
            },
        };
        found.cases.push(junit::Case::segment(&col, &res.id, seconds, o));
        let depth = col.chain(&res.id)?.len() - 1;
        println!("{} {}{} {}  {}f{}", dj_core::style::verdict(outcome, 8), "  ".repeat(depth), &res.id[..8], col.replays[&res.id].title(), res.frames, res.ticks.map(|t| format!(", {t} ticks")).unwrap_or_default());
        if let Some(d) = shots {
            save_shots(res, d)?;
        }
        total_frames += res.frames;
        found.differ += matches!(outcome, "dirty" | "changed") as usize;
        found.new += (outcome == "new") as usize;
        Ok(())
    };
    match &chosen {
        None => s.play_all(&col, &opts, from_power_on, |_, res| report(res))?,
        Some(ids) => {
            for id in ids {
                if dj_core::isolate::batch_skipped(&col, id).is_some() {
                    continue;
                }
                if let Some(res) = s.play_chain(&col, id, &opts, !from_power_on)?.last() {
                    report(res)?;
                }
            }
        }
    }
    // Left out after crashing or hanging the core in an earlier batch child (docs/cli.md).
    for (id, _) in col.tree_order().into_iter().filter(|(id, _)| chosen.as_ref().is_none_or(|c| c.iter().any(|x| x == id))) {
        if let Some(why) = dj_core::isolate::batch_skipped(&col, id) {
            found.cases.push(junit::Case::segment(&col, id, 0.0, junit::Outcome::Fail { message: format!("{why}: the core crashed or hung here, or above"), body: String::new() }));
        }
    }
    let secs = started.elapsed().as_secs_f64();
    println!("played {total_frames} frames in {secs:.2}s ({:.0} frames/s)", total_frames as f64 / secs.max(1e-9));
    print_inside_frame_summary();
    Ok(found)
}

/// `dj check`: exit 1 when a segment doesn't reproduce its recorded hashes, or with
/// `strict` has none recorded.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check(dir: &Path, local: &Local, pick: parity_cmd::Pick, from_power_on: bool, shots: Option<&Path>, extra_shots: Vec<u64>, strict: bool, junit_to: Option<&Path>) -> Result<i32> {
    if let Some(d) = shots {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    let mut c = check_tree(dir, local, &pick, from_power_on, shots, extra_shots)?;
    if let Some(path) = junit::target(junit_to, "dj-check.xml") {
        if strict {
            for case in &mut c.cases {
                if let junit::Outcome::Skip { message } = &case.outcome {
                    case.outcome = junit::Outcome::Fail { message: message.clone(), body: "--strict: a segment with no recorded hashes fails".into() };
                }
            }
        }
        junit::write(&path, "dj check", &c.cases)?;
    }
    if c.differ > 0 {
        return found(format!("{} segment(s) differ from their recorded hashes (`dj clean --accept` records the new ones)", c.differ));
    }
    if c.new > 0 {
        let what = format!("{} segment(s) have no recorded hashes, so nothing was checked for them (`dj clean` records them)", c.new);
        if strict {
            return found(what);
        }
        eprintln!("note: {what}");
    }
    Ok(0)
}

/// `dj cache fill`: fills the local database with what isn't stored yet. Unlike a
/// clean it records no hashes, and so plays dirty segments too, from wherever they start now.
pub(crate) fn fill(dir: &Path, local: &Local, segment: Option<String>, children: bool, force: bool) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let seg = match &segment {
        Some(s) => Some(col.resolve(s)?.to_string()),
        None => None,
    };
    let mut h = Hydrator::new(&col, seg.as_deref().map(|s| (s, children)), force)?;
    let mut s = open_session(&col, local)?;
    let started = std::time::Instant::now();
    while !h.done() {
        for ev in h.step(&mut s, &mut col, std::time::Duration::from_secs(3600), 0)? {
            let depth = col.chain(&ev.id)?.len() - 1;
            println!("{:<7} {} {}{} {}", ev.action, dj_core::style::verdict(ev.verdict, 9), "  ".repeat(depth), &ev.id[..8.min(ev.id.len())], col.replays[&ev.id].title());
        }
    }
    let played = h.events.iter().filter(|e| e.action == "played").count();
    println!("{played} played ({} frames in {:.1}s), {} already stored", h.frames_played, started.elapsed().as_secs_f64(), h.events.len() - played);
    print_inside_frame_summary();
    let problems = h.problems();
    if !problems.is_empty() {
        println!("{} segments don't match their recorded hashes:", problems.len());
        for p in problems {
            println!("  {} {}: {}", &p.id[..8.min(p.id.len())], col.replays[&p.id].title(), p.verdict);
        }
    }
    Ok(())
}

/// `dj clean`: what Clean all segments does in the GUI, saving each segment it records.
pub(crate) fn clean(dir: &Path, local: &Local, segment: Option<String>, children: bool, accept: bool, force: bool) -> Result<i32> {
    let mut col = Collection::open(dir)?;
    let seg = match &segment {
        Some(s) => Some(col.resolve(s)?.to_string()),
        None => None,
    };
    let mut h = Hydrator::verify(&col, seg.as_deref().map(|s| (s, children)), false, force)?;
    let mut s = open_session(&col, local)?;
    let started = std::time::Instant::now();
    while !h.done() {
        for ev in h.step(&mut s, &mut col, std::time::Duration::from_secs(3600), 0)? {
            if matches!(ev.verdict, "recorded" | "cleaned") || (accept && ev.verdict == "changed") {
                col.save_replay(&ev.id)?;
            } else if ev.verdict == "changed" {
                // Not written: put the file's version back, so the segments below wait for it.
                let p = col.paths[&ev.id].clone();
                col.replays.insert(ev.id.clone(), Replay::load(&p)?);
            }
            let depth = col.chain(&ev.id)?.len() - 1;
            println!("{:<7} {} {}{} {}", ev.action, dj_core::style::verdict(ev.verdict, 9), "  ".repeat(depth), &ev.id[..8.min(ev.id.len())], col.replays[&ev.id].title());
        }
    }
    let n = |v: &str| h.events.iter().filter(|e| e.verdict == v).count();
    println!(
        "{} recorded, {} cleaned, {} with a changed ending, {} blocked, {} already clean ({} frames in {:.1}s)",
        n("recorded"),
        n("cleaned"),
        n("changed"),
        n("blocked"),
        n("ok"),
        h.frames_played,
        started.elapsed().as_secs_f64()
    );
    print_inside_frame_summary();
    let changed: Vec<&str> = h.events.iter().filter(|e| e.verdict == "changed").map(|e| e.id.as_str()).collect();
    if !changed.is_empty() {
        println!("{}", if accept { "endings changed (written; check them, e.g. with git diff):" } else { "endings changed (not written; check them, then run again with --accept):" });
        for id in &changed {
            println!("  {} {}", &id[..8.min(id.len())], col.replays[*id].title());
        }
    }
    if n("blocked") > 0 {
        println!("blocked: segments below one that couldn't be cleaned (a segment above is still dirty)");
    }
    if !accept && !changed.is_empty() {
        return found(format!("{} segment(s) end differently from their files", changed.len()));
    }
    Ok(0)
}
