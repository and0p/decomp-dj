//! Commands on the project itself: init, new, status, annotations, repin, upgrade, snapshot, sql.

use crate::*;

pub(crate) fn init(dir: &Path, name: &str, rom: &Path, core_path: &Path, core_build: Option<String>, options: &[String], symbols: Option<&Path>) -> Result<()> {
    if dir.join("config.toml").exists() || dir.join("collection.toml").exists() {
        bail!("{} already has a project", dir.display());
    }
    let core_path = &libretro::library_path(core_path);
    if !core_path.is_file() {
        bail!("the core library {} doesn't exist", core_path.display());
    }
    let mut overrides = BTreeMap::new();
    for o in options {
        let (k, v) = o.split_once('=').ok_or_else(|| anyhow!("--option takes KEY=VALUE"))?;
        overrides.insert(k.to_string(), v.to_string());
    }
    std::fs::create_dir_all(dir.join("segments"))?;
    std::fs::create_dir_all(dir.join("blobs"))?;
    let work = dir.join(".local");
    let (lib_name, build_id, effective, regions) = {
        let mut core = libretro::Core::load(core_path, &work, &overrides)?;
        core.load_game(rom)?;
        // Some cores set up their memory on the first frame (Mupen64Plus-Next): run one so
        // the RAM regions to hash exist when they're probed.
        core.run_frame(false);
        let mut regions = vec![];
        for r in ["RAM", "SRAM"] {
            if core.memory_view(libretro::region_memory_id(r).unwrap()).is_some() {
                regions.push(r.to_string());
            }
        }
        if !core.has_connector() {
            eprintln!("note: this core has no coverage connector; segments will play but record no coverage");
        } else {
            print_connector_regions(&mut core, rom)?;
        }
        (core.library_name.clone(), core.build_id(), core.effective_options(), regions)
    };
    let mut syms = vec![];
    if let Some(s) = symbols {
        std::fs::create_dir_all(dir.join("symbols"))?;
        let fname = s.file_name().ok_or_else(|| anyhow!("bad symbols path"))?;
        std::fs::copy(s, dir.join("symbols").join(fname))?;
        let format = dj_core::analysis::guess_format(s);
        syms.push(SymbolsConfig { file: format!("symbols/{}", fname.to_string_lossy()), format: format.into(), rom_header: None, region: None, cpu_base: None });
    }
    let config = Config {
        decoders: vec![],
        dj: Some(dj_core::collection::DJ_VERSION.to_string()),
        format: dj_core::collection::PROJECT_FORMAT,
        name: name.to_string(),
        rom: RomConfig { sha256: sha256_file(rom)?, file: rom.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default() },
        core: CoreConfig { name: lib_name.to_lowercase(), build: core_build.unwrap_or(build_id), sha256: String::new(), options: effective },
        ports: vec![PortConfig { port: 1, device: "JOYPAD".into() }],
        hash: HashConfig { algorithm: "blake3".into(), regions, exclude: vec![] },
        tick: None,
        symbols: syms,
        cache: None,
        compare: vec![],
    };
    std::fs::write(dir.join("config.toml"), toml::to_string_pretty(&config)?)?;
    let col = Collection::open(dir)?;
    col.save_local(&LocalConfig { rom: Some(std::fs::canonicalize(rom)?), core: Some(std::fs::canonicalize(core_path)?), ..Default::default() })?;
    println!("created {}", dir.join("config.toml").display());
    Ok(())
}

pub(crate) fn new_replay(dir: &Path, parent: Option<String>, title: &str, frames: u64) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let parent = match parent {
        Some(p) => Some(col.resolve(&p)?.to_string()),
        None => None,
    };
    let id = uuid::Uuid::new_v4().to_string();
    let r = Replay::new(&id, parent.as_deref(), title, frames);
    col.replays.insert(id.clone(), r);
    col.save_replay(&id)?;
    println!("{}", col.paths[&id].display());
    Ok(())
}

pub(crate) fn status(dir: &Path) -> Result<()> {
    let col = Collection::open(dir)?;
    let mut counts = BTreeMap::new();
    for (id, depth) in col.tree_order() {
        let r = &col.replays[id];
        let st = col.tree_status(id);
        *counts.entry(st.label()).or_insert(0) += 1;
        let mark = match st {
            Status::Clean => " ",
            Status::Dirty if r.get("end-hash").is_none() => "?",
            Status::Dirty => "~",
        };
        println!("{mark} {}{} {:>6}f  {}{}", "  ".repeat(depth), &id[..8], r.frames()?, r.title(), if r.is_poked() { "  [poked]" } else { "" });
    }
    println!("{} segments: {:?}  (? unrecorded: no end hash yet; ~ stale: starts where its parent no longer ends)", col.replays.len(), counts);
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
        if col.replays[id].to_text() != disk {
            col.save_replay(id)?;
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
    let frames = col.replays[&id].frames()?;
    if frame > frames {
        bail!("{} has {frames} frames", &id[..8.min(id.len())]);
    }
    let mut s = open_session(&col, local)?;
    s.enter_replay(&col, &id)?;
    let start = s.state_hash(&col)?;
    let mut at = 0;
    if let Some((f, state)) = s.nearest_stored(&col, &id, &start, frame)? {
        if f > 0 {
            s.load_state(&state)?;
            at = f;
        }
    }
    s.restore_inputs(&col, &id, at)?;
    for f in at..frame {
        s.step(&col, &id, f, false)?;
    }
    let state = s.snapshot()?;
    std::fs::write(out, &state)?;
    println!("wrote {} ({} bytes; frame {frame} of {}, played {} frames from {})", out.display(), state.len(), &id[..8.min(id.len())], frame - at, if at == 0 { "the segment's start".to_string() } else { format!("the snapshot at frame {at}") });
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
