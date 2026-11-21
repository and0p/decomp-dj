//! Importing and exporting TAS movies.

use crate::*;

/// The project's ROM, for a command that reads it but runs no core: `--rom`, else the one in
/// the settings. None when neither is set.
pub(crate) fn tas_rom(col: &Collection, local: &Local) -> Result<Option<Vec<u8>>> {
    match local.rom.clone().or(col.settings()?.rom) {
        Some(p) => Ok(Some(std::fs::read(&p).with_context(|| format!("reading the ROM {}", p.display()))?)),
        None => Ok(None),
    }
}

pub(crate) fn import_tas(dir: &Path, file: &Path, title: Option<&str>, strict: bool, local: &Local) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let rom = tas_rom(&col, local)?;
    let ctx = dj_core::tas::Ctx { col: &col, rom: rom.as_deref() };
    let imp = dj_core::tas::import(&ctx, file, title, strict)?;
    for w in &imp.warnings {
        eprintln!("warning: {w}");
    }
    let id = imp.segment.id()?.to_string();
    let frames = imp.segment.frames()?;
    let lines = imp.segment.lines.len();
    col.segments.insert(id.clone(), imp.segment);
    col.save_segment(&id)?;
    eprintln!(
        "imported {} frames ({lines} lines, {} subtitles) from {} as a root segment; clean it (dj clean {}) to record its hashes, and watch it in dj-gui to see how far it stays in sync",
        frames,
        imp.movie.markers.len(),
        imp.movie.meta.emulator,
        &id[..8]
    );
    println!("{id}");
    Ok(())
}

pub(crate) fn export_tas(dir: &Path, segment: &str, out: &Path, through: Option<u64>, author: Option<&str>, strict: bool, local: &Local) -> Result<()> {
    let col = Collection::open(dir)?;
    dj_core::tas::check_ext(out)?;
    let id = col.resolve(segment)?.to_string();
    let rom = tas_rom(&col, local)?;
    let ctx = dj_core::tas::Ctx { col: &col, rom: rom.as_deref() };
    let ex = dj_core::tas::export(&ctx, dj_core::tas::ExportEnd { segment: &id, through }, strict, author)?;
    for w in &ex.warnings {
        eprintln!("warning: {w}");
    }
    std::fs::write(out, &ex.bytes).with_context(|| format!("writing {}", out.display()))?;
    eprintln!("wrote {} frames ({} segment{}) to {}", ex.frames, ex.segments, if ex.segments == 1 { "" } else { "s" }, out.display());
    Ok(())
}
