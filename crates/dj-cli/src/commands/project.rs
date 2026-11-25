//! Commands on the project itself: init, new, status, annotations, repin, upgrade, snapshot, sql.

use crate::*;

pub(crate) fn init(dir: Option<&Path>, name: Option<&str>, rom: &Path, core_path: &Path, core_build: Option<String>, options: &[String], symbols: Option<&Path>) -> Result<()> {
    let mut overrides = BTreeMap::new();
    for o in options {
        let (k, v) = o.split_once('=').ok_or_else(|| anyhow!("--option takes KEY=VALUE"))?;
        overrides.insert(k.to_string(), v.to_string());
    }
    let plan = dj_core::scaffold::propose(&dj_core::scaffold::InitRequest {
        dir: dir.map(Path::to_path_buf),
        name: name.map(String::from),
        rom: rom.to_path_buf(),
        core: core_path.to_path_buf(),
        core_build,
        options: overrides,
        symbols: symbols.map(Path::to_path_buf),
    })?;
    if !plan.connector {
        eprintln!("note: this core has no coverage connector; segments will play but record no coverage");
    } else {
        println!("connector regions:");
        for r in &plan.regions {
            let f = &r.facts;
            let place = f.file_offset.map(|o| format!("ROM file from byte {o}")).unwrap_or_else(|| "not from the ROM file".into());
            let decoder = match f.cpu.as_deref() {
                Some(c) if r.decoder => c.to_string(),
                Some(c) => format!("{c} (no built-in decoder: a plugin or the core can add one, docs/decoders.md)"),
                None => "no instruction set".into(),
            };
            let mark = if r.rolls_up { "  <- coverage rolls up here" } else { "" };
            println!("  {:<6} {:>8} bytes  {decoder}, {place}{mark}", f.name, f.size);
            if r.differs_from_rom {
                eprintln!("warning: the connector's {} bytes don't match {} from byte {}", f.name, rom.display(), f.file_offset.unwrap_or(0));
            }
        }
    }
    let path = plan.write()?;
    println!("created {}", path.display());
    Ok(())
}

pub(crate) fn new_segment(dir: &Path, parent: Option<String>, title: &str, frames: u64) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let parent = match parent {
        Some(p) => Some(col.resolve(&p)?.to_string()),
        None => None,
    };
    let id = uuid::Uuid::new_v4().to_string();
    let r = Segment::new(&id, parent.as_deref(), title, frames);
    col.segments.insert(id.clone(), r);
    col.save_segment(&id)?;
    println!("{}", col.paths[&id].display());
    Ok(())
}

/// `dj new --snapshot`, `--sram` and `--from`: a root segment that starts from a save
/// state, a save file, or a frame of another segment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_root(dir: &Path, title: &str, frames: u64, snapshot: Option<&Path>, sram: Option<&Path>, start_frame: u64, from: Option<(&str, u64)>, blob_name: Option<&str>, local: &Local) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let mut s = open_session(&col, local)?;
    let read = |p: Option<&Path>| p.map(|p| std::fs::read(p).with_context(|| format!("reading {}", p.display()))).transpose();
    let mut new = dj_core::player::NewRoot { title: title.to_string(), frames, snapshot: read(snapshot)?, sram: read(sram)?, start_frame, lines: vec![], blob_name: String::new() };
    // The blob's name: the one asked for, else the file's own, else (--from) where it was cut.
    let stem = snapshot.or(sram).and_then(|p| p.file_stem()).map(|n| dj_core::collection::blob_name(&n.to_string_lossy()));
    new.blob_name = blob_name.map(String::from).or(stem).unwrap_or_default();
    if let Some((segment, frame)) = from {
        let src = col.resolve(segment)?.to_string();
        let (state, at) = s.state_at(&col, &src, frame)?;
        new.snapshot = Some(state);
        new.start_frame = at;
        new.lines = dj_core::edit::carried_lines(&col, &src, frame)?;
        if title.is_empty() {
            new.title = format!("{} from frame {frame}", col.segments[&src].title());
        }
        if blob_name.is_none() {
            new.blob_name = dj_core::collection::blob_name(&format!("{} frame {frame}", col.segments[&src].title()));
        }
    }
    let id = s.add_root(&mut col, &new)?;
    if new.snapshot.is_some() && col.warns_about_snapshots() {
        eprintln!("note: this copies a snapshot into the project's blobs folder. {}", dj_core::collection::SNAPSHOT_WARNING);
    }
    let r = &col.segments[&id];
    if let Some(b) = r.get("snapshot") {
        eprintln!("snapshot {} ({} bytes), {} frames after power-on{}", blob_file(&col, b, "snapshot"), new.snapshot.as_ref().map_or(0, |d| d.len()), new.start_frame, match new.lines.len() {
            0 => String::new(),
            1 => ", 1 line carried onto frame 0".to_string(),
            n => format!(", {n} lines carried onto frame 0"),
        });
    }
    if let Some(b) = r.get("sram") {
        eprintln!("save file {} ({} bytes)", blob_file(&col, b, "sram"), new.sram.as_ref().map_or(0, |d| d.len()));
    }
    println!("{}", col.paths[&id].display());
    Ok(())
}

pub(crate) fn status(dir: &Path) -> Result<()> {
    let col = Collection::open(dir)?;
    let mut counts = BTreeMap::new();
    for (id, depth) in col.tree_order() {
        let r = &col.segments[id];
        let st = col.tree_status(id);
        *counts.entry(st.label()).or_insert(0) += 1;
        let mark = match st {
            Status::Clean => " ",
            Status::Dirty if r.get("end-hash").is_none() => "?",
            Status::Dirty => "~",
        };
        println!("{mark} {}{} {:>6}f  {}{}", "  ".repeat(depth), &id[..8], r.frames()?, r.title(), if r.is_poked() { "  [poked]" } else { "" });
    }
    println!("{} segments: {:?}  (? unrecorded: no end hash yet; ~ stale: starts where its parent no longer ends)", col.segments.len(), counts);
    Ok(())
}

pub(crate) fn annotations(dir: &Path, local: &Local) -> Result<i32> {
    use dj_core::annotations::{Annotations, Kind};
    let col = Collection::open(dir)?;
    let ann = Annotations::load(&col.root)?;
    if ann.is_empty() {
        println!("no annotations ({} not found or empty)", col.root.join(dj_core::annotations::FILE).display());
        return Ok(0);
    }
    let rom = tas_rom(&col, local)?;
    // What the maps alone say, to tell a rename from a new function.
    let maps = dj_core::analysis::project_functions(&col, rom.as_deref(), &Annotations::default())?;
    let merged = dj_core::analysis::project_functions(&col, rom.as_deref(), &ann)?;
    let rep = match cache::Cache::open_existing(&col) {
        Ok(conn) => match coverage::build_report(&col, &conn, rom.as_deref(), None) {
            Ok(r) => Some(r),
            Err(e) => {
                println!("(not checked against coverage: {e:#})");
                None
            }
        },
        Err(e) => {
            println!("(not checked against coverage: {e:#})");
            None
        }
    };
    let mut problems = 0;
    for kind in [Kind::Unreachable, Kind::Data, Kind::Function] {
        for e in ann.of_kind(kind) {
            let mut line = format!("{:<20} {:<22}", e.label(), e.range_text());
            match kind {
                Kind::Function => {
                    let name = e.name.as_deref().unwrap_or("");
                    let was = maps.iter().find(|(f, _)| f.region == e.region && f.offset == e.start);
                    let now = merged.iter().find(|(f, _)| f.region == e.region && f.offset == e.start).map(|(f, _)| f.size).unwrap_or(0);
                    match was {
                        Some((f, _)) if f.name == name && f.size == now => line += &format!(" {name}: the map says the same"),
                        Some((f, _)) => line += &format!(" {name}: was {} ({:#X} bytes), now {now:#X} bytes", f.name, f.size),
                        None => line += &format!(" {name}: new, {now:#X} bytes"),
                    }
                }
                _ => {
                    let size = e.end - e.start + 1;
                    line += &format!(" {size:>5} bytes");
                    if let Some(w) = rep.as_ref().and_then(|r| r.annotation_warnings.iter().find(|w| w.entry == e.label())) {
                        line += &format!("  RAN: {}", w.text(&col));
                        problems += 1;
                    }
                }
            }
            if !e.why.is_empty() {
                line += &format!("  # {}", e.why);
            }
            println!("{line}");
        }
    }
    if let Some(r) = &rep {
        let t = &r.totals;
        println!("left out of coverage: {} unreachable {}, {} data bytes", t.unreachable_instructions, if r.symbols { "instructions" } else { "bytes" }, t.data_bytes);
    }
    if problems > 0 {
        return found(format!("{problems} annotated range(s) ran as code"));
    }
    Ok(0)
}

/// `dj repin`: checks the whole tree with the core as it is now, then records its build.
pub(crate) fn repin(dir: &Path, local: &Local) -> Result<i32> {
    let bad = check_tree(dir, local, &parity_cmd::Pick::default(), true, None, vec![])?.differ;
    if bad > 0 {
        return found(format!(
            "not repinned: {bad} segment(s) don't reproduce their hashes with this core. If the new hashes are the right ones, record them with `dj clean --force --accept`, then run `dj repin` again"
        ));
    }
    let mut col = Collection::open(dir)?;
    let (_, core) = resolve_local(&col, local)?;
    let build = open_session(&col, local)?.build.clone();
    let sha = sha256_file(&libretro::library_path(&core))?;
    if build == col.config.core.build {
        println!("every segment matches; [core] build is already \"{build}\"");
        return Ok(0);
    }
    let was = col.config.core.build.clone();
    col.set_core_build(&build, Some(&sha))?;
    println!("every segment matches: [core] build \"{was}\" -> \"{build}\" in config.toml{}", if col.config.core.sha256.is_empty() { "" } else { " (and its sha256)" });
    Ok(0)
}

/// `dj upgrade`: every segment file and config.toml in this dj's format.
pub(crate) fn upgrade(dir: &Path) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let ids: Vec<String> = col.tree_order().into_iter().map(|(id, _)| id.to_string()).filter(|id| !col.is_draft(id)).collect();
    let mut rewritten = 0;
    for id in &ids {
        let Some(path) = col.paths.get(id) else { continue };
        let disk = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if col.segments[id].to_text() != disk {
            col.save_segment(id)?;
            rewritten += 1;
        }
    }
    col.note_writer()?;
    println!(
        "{rewritten} of {} segment files rewritten (segment format {}); config.toml: format {}, dj {}",
        ids.len(),
        dj_core::format::SEGMENT_FORMAT,
        dj_core::collection::PROJECT_FORMAT,
        col.config.dj.as_deref().unwrap_or("?")
    );
    Ok(())
}

pub(crate) fn snapshot_cmd(dir: &Path, segment: &str, frame: u64, out: &Path, local: &Local) -> Result<()> {
    let col = Collection::open(dir)?;
    let id = col.resolve(segment)?.to_string();
    let mut s = open_session(&col, local)?;
    let (state, at) = s.state_at(&col, &id, frame)?;
    std::fs::write(out, &state)?;
    println!("wrote {} ({} bytes; frame {frame} of {}, {at} frames after power-on)", out.display(), state.len(), &id[..8.min(id.len())]);
    Ok(())
}

pub(crate) fn sram_cmd(dir: &Path, segment: &str, frame: u64, out: &Path, local: &Local) -> Result<()> {
    let col = Collection::open(dir)?;
    let id = col.resolve(segment)?.to_string();
    let mut s = open_session(&col, local)?;
    let data = s.sram_at(&col, &id, frame)?;
    std::fs::write(out, &data)?;
    println!("wrote {} ({} bytes; the save RAM at frame {frame} of {})", out.display(), data.len(), &id[..8.min(id.len())]);
    Ok(())
}

/// A blob's file name in blobs/, for a message.
fn blob_file(col: &Collection, blob: &str, field: &str) -> String {
    col.blob_path(blob, field).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| blob.to_string())
}

pub(crate) fn blob_list(dir: &Path) -> Result<()> {
    let col = Collection::open(dir)?;
    let blobs = col.blobs();
    if blobs.is_empty() {
        println!("no snapshots or save files in {}", col.root.join("blobs").display());
    }
    for b in blobs {
        let users: Vec<String> = b.users.iter().map(|id| format!("{} {}", &id[..8.min(id.len())], col.segments[id].title())).collect();
        let kind = if b.field == "sram" { "save file" } else { "snapshot" };
        println!("{}  {kind}, {} bytes, hash {}, {}", b.stem, b.size, b.hash, if users.is_empty() { "not used".to_string() } else { format!("used by {}", users.join("; ")) });
    }
    Ok(())
}

pub(crate) fn blob_rename(dir: &Path, blob: &str, name: &str) -> Result<()> {
    let col = Collection::open(dir)?;
    // The file's name (with or without its extension), or the start of exactly one name or hash.
    let all = col.blobs();
    let want = blob.strip_suffix(".srm").or(blob.strip_suffix(".state")).unwrap_or(blob);
    let hits: Vec<&dj_core::collection::BlobFile> = match all.iter().filter(|b| b.stem == want || b.hash == want).collect::<Vec<_>>() {
        exact if !exact.is_empty() => exact,
        _ => all.iter().filter(|b| b.stem.starts_with(want) || b.hash.starts_with(want)).collect(),
    };
    let b = match hits[..] {
        [one] => one,
        [] => anyhow::bail!("no blob called {blob} in blobs/ (`dj blob list` shows them)"),
        _ => anyhow::bail!("{blob} could be more than one blob: {}", hits.iter().map(|b| format!("{}.{}", b.stem, dj_core::collection::blob_ext(b.field))).collect::<Vec<_>>().join(", ")),
    };
    let new = col.rename_blob(&b.hash, b.field, name)?;
    println!("{} is now {new} ({} it; none needed changing)", b.stem, match b.users.len() {
        1 => "1 segment starts from".to_string(),
        n => format!("{n} segments start from"),
    });
    Ok(())
}

pub(crate) fn sql(dir: &Path, query: &str) -> Result<()> {
    let col = Collection::open(dir)?;
    let mut conn = cache::Cache::open_existing(&col)?;
    // Bring segments_now (and the v_ views on it) up to date with the segment files, for the
    // setup played last. A database nothing has been played into yet has no setup to use.
    let setup: Option<String> = rusqlite::OptionalExtension::optional(conn.query_row("SELECT setup FROM contents ORDER BY played_at DESC LIMIT 1", [], |r| r.get(0)))?;
    if let Some(setup) = setup {
        if let Err(e) = cache::refresh_segments_now(&mut conn, &col, &setup) {
            eprintln!("warning: couldn't bring segments_now up to date ({e:#}); the views show the segments as of the last play");
        }
    }
    let mut st = conn.prepare(query)?;
    let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
    println!("{}", names.join("\t"));
    let n = names.len();
    let mut rows = st.query([])?;
    while let Some(row) = rows.next()? {
        let mut cells = vec![];
        for i in 0..n {
            use rusqlite::types::ValueRef;
            cells.push(match row.get_ref(i)? {
                ValueRef::Null => "NULL".to_string(),
                ValueRef::Integer(v) => v.to_string(),
                ValueRef::Real(v) => v.to_string(),
                ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                ValueRef::Blob(b) => format!("<{} bytes>", b.len()),
            });
        }
        println!("{}", cells.join("\t"));
    }
    Ok(())
}

/// `dj controls`: what each port's inputs are called.
pub(crate) fn controls(dir: &Path, local: &Local) -> Result<()> {
    use dj_core::controls::{control_from_name, control_name, Controls};
    use dj_core::format::JOYPAD;
    let col = Collection::open(dir)?;
    let s = open_session(&col, local)?;
    let names = s.core.input_names();
    let c = Controls::new(&names, &col.config.ports);
    if names.is_empty() {
        println!("The core doesn't name its inputs: the GUI shows the usual joypad buttons, and any [[ports]] labels.");
    }
    let mut ports: std::collections::BTreeSet<u32> = names.iter().map(|(k, _)| k.port).collect();
    ports.extend(col.config.ports.iter().filter(|p| p.port > 0).map(|p| p.port - 1));
    let sticks = ["LEFT X", "LEFT X-", "LEFT X+", "LEFT Y", "LEFT Y-", "LEFT Y+", "RIGHT X", "RIGHT X-", "RIGHT X+", "RIGHT Y", "RIGHT Y-", "RIGHT Y+"];
    let all: Vec<&str> = JOYPAD.iter().copied().chain(sticks).collect();
    for port in ports {
        println!("P{}", port + 1);
        println!("  {:<9} {:<28} {:<20} {}", "control", "core's name", "label", "shown");
        for name in &all {
            let key = control_from_name(port, name).expect("a control's name");
            let core = names.iter().find(|(k, _)| *k == key).map(|(_, n)| n.as_str());
            let label = c.label(key).filter(|l| Some(*l) != core);
            if core.is_none() && label.is_none() && !c.hidden(key) {
                continue;
            }
            let shown = if c.hidden(key) {
                "no (hide)"
            } else if c.directions(key) {
                "as buttons"
            } else {
                "yes"
            };
            println!("  {:<9} {:<28} {:<20} {shown}", name, core.unwrap_or("-"), label.unwrap_or("-"));
        }
        // Inputs the core names that [[ports]] can't: listed so nothing it says is lost.
        for (k, n) in names.iter().filter(|(k, _)| k.port == port && control_name(*k).is_none()) {
            println!("  {:<9} {n}", format!("{} {} {}", k.device, k.index, k.id));
        }
    }
    Ok(())
}
