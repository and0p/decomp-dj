//! The `dj cache` commands: the local database's size, clean-up, export and seeding.

use crate::*;

pub(crate) fn human(bytes: i64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.1} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

pub(crate) fn cache_stats(dir: &Path) -> Result<()> {
    let col = Project::open(dir)?;
    let conn = cache::Cache::open_existing(&col)?;
    let size = std::fs::metadata(cache::cache_path(&col)).map(|m| m.len()).unwrap_or(0);
    println!("{} ({})", cache::cache_path(&col).display(), human(size as i64));
    let settings = cache::CacheSettings::resolve(col.config.cache.as_ref(), col.settings()?.cache.as_ref())?;
    println!(
        "settings: preset {}, pages {}, a snapshot every {} frames{}, previews {:?}",
        settings.preset,
        settings.page_bytes.map(|b| format!("{} KB", b / 1024)).unwrap_or_else(|| "1:1".into()),
        settings.diff_every,
        if settings.diff_at_markers { " and at markers" } else { "" },
        settings.previews
    );
    for (name, q, bytes_q) in [
        ("contents", "SELECT count(*) FROM contents", None),
        ("snapshots", "SELECT count(*) FROM snapshots", Some("SELECT coalesce(sum(size),0) FROM snapshots")),
        ("pages", "SELECT count(*) FROM pages", Some("SELECT coalesce(sum(length(data)),0) FROM pages")),
        ("stored frames", "SELECT count(*) FROM frames", None),
        ("previews", "SELECT count(*) FROM images", Some("SELECT coalesce(sum(length(png)),0) FROM images")),
        ("coverage rows", "SELECT count(*) FROM coverage", Some("SELECT coalesce(sum(length(flags)),0) FROM coverage")),
        ("first hits", "SELECT count(*) FROM first_hits", None),
    ] {
        let n: i64 = conn.query_row(q, [], |r| r.get(0))?;
        let b = match bytes_q {
            Some(bq) => format!("  {}", human(conn.query_row(bq, [], |r| r.get(0))?)),
            None => String::new(),
        };
        println!("  {name:<14} {n:>8}{b}");
    }
    let raw: i64 = conn.query_row("SELECT coalesce(sum(size),0) FROM snapshots", [], |r| r.get(0))?;
    let stored: i64 = conn.query_row("SELECT coalesce(sum(length(data)),0) FROM pages", [], |r| r.get(0))?;
    if stored > 0 {
        println!("  snapshots take {} as pages, {:.1}x smaller than {} raw", human(stored), raw as f64 / stored as f64, human(raw));
    }
    if let Some(b) = settings.budget_bytes {
        if size > b {
            println!("  over the {} budget; run `dj cache gc`", human(b as i64));
        }
    }
    Ok(())
}

pub(crate) fn cache_gc(dir: &Path, local: &Local, force: bool) -> Result<()> {
    let col = Project::open(dir)?;
    if !force && dj_core::cache::gui_open(&col.root) {
        bail!("a dj-gui has this project open, and gc would drop the snapshots of its unsaved edits; close it first, or pass --force");
    }
    let mut s = open_session(&col, local)?;
    let c = s.cache.as_mut().ok_or_else(|| anyhow!("gc needs the local database"))?;
    let before = c.file_size();
    let (contents, snaps, pages) = c.gc(&col)?;
    println!("removed {contents} contents, {snaps} snapshots and {pages} pages; {} -> {}", human(before as i64), human(c.file_size() as i64));
    Ok(())
}

/// `dj cache export`: the start states that checking the chosen segments needs.
pub(crate) fn cache_export(dir: &Path, local: &Local, pick: parity_cmd::Pick, compare: Option<&str>, out: &Path) -> Result<()> {
    let col = Project::open(dir)?;
    let ids = pick.resolve(&col)?;
    let preroll = match compare {
        Some(name) => {
            let name = dj_core::parity::compare_name(&col, Some(name))?;
            col.config.compare.iter().find(|c| c.name == name).map(|c| c.preroll).unwrap_or(0)
        }
        None => col.config.compare.iter().map(|c| c.preroll).max().unwrap_or(0),
    };
    // The segments whose start state is wanted: a root starts at power-on, or from its own
    // snapshot blob, so needs none.
    let mut wanted: Vec<String> = vec![];
    let mut want = |x: &str| {
        if col.segments[x].parent().is_some() && !wanted.iter().any(|w| w == x) {
            wanted.push(x.to_string());
        }
    };
    for id in &ids {
        want(id);
        if preroll > 0 {
            if let Some((first, _, _)) = dj_core::parity::preroll_route(&col, id, 0, preroll)?.first() {
                want(first);
            }
        }
    }
    let unclean: Vec<&String> = wanted.iter().filter(|x| col.segments[x.as_str()].parent().is_some_and(|p| col.tree_status(p) != Status::Clean)).collect();
    if !unclean.is_empty() {
        bail!(
            "the start of {} isn't known: the segment above isn't clean (dj clean)",
            unclean.iter().map(|x| format!("{} {}", &x[..8.min(x.len())], col.segments[x.as_str()].title())).collect::<Vec<_>>().join(", ")
        );
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut s = open_session(&col, local)?;
    let mut bytes = 0;
    for x in &wanted {
        let parent = col.segments[x.as_str()].parent().unwrap_or_default().to_string();
        let expected = col.segments[&parent].get("end-hash").unwrap_or_default().to_string();
        s.enter_segment(&col, x)?;
        let h = s.state_hash(&col)?;
        if h != expected {
            bail!("{}: the state it starts from has hash {h}, but {} records {expected}: clean the project (dj clean)", &x[..8.min(x.len())], &parent[..8.min(parent.len())]);
        }
        let state = s.state_to_store()?;
        let p = out.join(format!("{h}.state"));
        std::fs::write(&p, &state).with_context(|| format!("writing {}", p.display()))?;
        bytes += state.len();
        println!("{h}  start of {} {}", &x[..8.min(x.len())], col.segments[x.as_str()].title());
    }
    println!("{} state(s), {} into {}{}", wanted.len(), human(bytes as i64), out.display(), if preroll > 0 { format!(" (with {preroll} frames of pre-roll)") } else { String::new() });
    Ok(())
}

/// `dj cache seed`: stores the states given that are a clean segment's recorded end.
pub(crate) fn cache_seed(dir: &Path, local: &Local, paths: &[PathBuf]) -> Result<i32> {
    let col = Project::open(dir)?;
    let mut files = vec![];
    for p in paths {
        if p.is_dir() {
            let mut inside: Vec<PathBuf> = std::fs::read_dir(p).with_context(|| format!("reading {}", p.display()))?.filter_map(|e| e.ok().map(|e| e.path())).filter(|f| f.is_file()).collect();
            inside.sort();
            files.extend(inside);
        } else {
            files.push(p.clone());
        }
    }
    // Where each recorded end hash is: what `dj parity check`, `dj clean` and the rest load
    // a child's start from.
    let mut ends: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, r) in &col.segments {
        if let (Some(h), Status::Clean) = (r.get("end-hash"), col.tree_status(id)) {
            ends.entry(h.to_string()).or_default().push(id.clone());
        }
    }
    let mut s = open_session(&col, local)?;
    if s.cache.is_none() {
        bail!("seeding fills the local database, which is turned off in .local/local.toml");
    }
    let (mut stored, mut refused) = (0, vec![]);
    for f in &files {
        let data = std::fs::read(f).with_context(|| format!("reading {}", f.display()))?;
        if let Err(e) = s.load_state(&data) {
            refused.push(format!("{}: the core can't load it ({e:#}): another core build, or not a state", f.display()));
            continue;
        }
        let h = s.state_hash(&col)?;
        let Some(ids) = ends.get(&h) else {
            refused.push(format!("{}: its state hash {h} is no clean segment's end: from an older project, or another core build that plays differently", f.display()));
            continue;
        };
        let state = s.state_to_store()?;
        s.cache.as_mut().unwrap().put_snapshot(&h, &state)?;
        stored += 1;
        let names: Vec<String> = ids.iter().map(|id| format!("{} {}", &id[..8.min(id.len())], col.segments[id].title())).collect();
        println!("{}  {h}  end of {}", dj_core::style::out("stored ", dj_core::style::Tone::Good), names.join(", "));
    }
    for r in &refused {
        println!("{} {r}", dj_core::style::verdict("refused", 7));
    }
    println!("{stored} stored, {} refused", refused.len());
    if !refused.is_empty() {
        return found(format!("{} state(s) refused", refused.len()));
    }
    Ok(0)
}
