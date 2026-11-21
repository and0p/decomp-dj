mod cli;
mod commands;
mod mcp;
mod junit;
mod parity_cmd;
mod queries;
mod trace;
use dj_core::{cache, coverage, libretro};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dj_core::collection::{sha256_file, Collection, Config, CoreConfig, HashConfig, LocalConfig, PortConfig, RomConfig, Status, SymbolsConfig};
use dj_core::format::Replay;
use dj_core::hydrate::Hydrator;
use dj_core::player::{PlayOptions, ReplayResult, Session};

use cli::*;
use commands::{cache::*, check::*, project::*, tas::*};

fn main() {
    // Started as a worker for code dj doesn't trust (docs/decoders.md, docs/cli.md)?
    dj_core::isolate::run_if_worker();
    if let Err(e) = real_main() {
        eprintln!("{} {e:#}", dj_core::style::err("error:", dj_core::style::Tone::Bad));
        std::process::exit(2);
    }
}

/// Ends the process with a command's exit status when it isn't 0. Every command uses the
/// same three: 0 all is well, 1 the command ran and found something wrong (hashes that
/// differ, a parity failure, an annotation that ran), 2 it couldn't do its job (an error).
fn exit_with(code: i32) -> Result<()> {
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// The end of a command that ran and found something wrong: says so on stderr, and gives
/// exit status 1 (an error, which can't do its job, is 2).
fn found(what: String) -> Result<i32> {
    eprintln!("{} {what}", dj_core::style::err("failed:", dj_core::style::Tone::Bad));
    Ok(1)
}

/// For the end of a batch command: one line on stderr saying how many of those there were,
/// if any. The warnings above it name each one.
pub(crate) fn print_inside_frame_summary() {
    let (writes, capped) = dj_core::console::inside_frame_counts();
    let mut parts = vec![];
    if writes > 0 {
        parts.push(format!("{writes} [WRITE] line{} at an instruction not made", if writes == 1 { "" } else { "s" }));
    }
    if capped > 0 {
        parts.push(format!("a breakpoint switched off for the rest of its frame {capped} time{}", if capped == 1 { "" } else { "s" }));
    }
    if !parts.is_empty() {
        eprintln!("{} {} (the warnings above name each)", dj_core::style::err("warning:", dj_core::style::Tone::Note), parts.join("; "));
    }
}

fn real_main() -> Result<()> {
    if let Some(d) = old_form(std::env::args_os().skip(1)) {
        eprintln!("{} {d} is a project folder, which dj no longer takes as an argument: run dj inside it, or pass -C {d} (as in dj -C {d} check)", dj_core::style::err("error:", dj_core::style::Tone::Bad));
        std::process::exit(2);
    }
    let cli = Cli::parse();
    if let Cmd::Init { dir, name, rom, core, core_build, options, symbols } = &cli.cmd {
        if cli.project.is_some() {
            bail!("dj init takes the folder to create as its argument, not -C");
        }
        return init(dir, name, rom, core, core_build.clone(), options, symbols.as_deref());
    }
    let dir = project_dir(cli.project.as_deref())?;
    // Batch commands play in a child dj, so a core that crashes or hangs costs a segment,
    // not the command (docs/cli.md). The parity commands that run no core don't need one.
    let batch = matches!(
        &cli.cmd,
        Cmd::Check { .. } | Cmd::Clean { .. } | Cmd::Cache { cmd: CacheCmd::Fill { .. } } | Cmd::Parity { cmd: ParityCmd::Check { .. } | ParityCmd::Rerun { .. } | ParityCmd::Explain { .. } }
    );
    if batch {
        if let Some(code) = dj_core::isolate::supervise(&dir)? {
            std::process::exit(code);
        }
    }
    // A project a newer dj wrote, or settings this dj doesn't know: said once, here (in the
    // batch child when supervised, so not twice).
    if let Ok(t) = dj_core::collection::layered_table(&dir) {
        for w in dj_core::collection::check_versions(&t).unwrap_or_default() {
            eprintln!("warning: {w}");
        }
    }
    match cli.cmd {
        Cmd::Init { .. } => unreachable!("dj init is handled above"),
        Cmd::New { parent, title, frames } => new_replay(&dir, parent, &title, frames),
        Cmd::Status => status(&dir),
        Cmd::ImportTas { file, title, strict, rom } => import_tas(&dir, &file, title.as_deref(), strict, &rom.local()),
        Cmd::ExportTas { segment, out, through, author, strict, rom } => export_tas(&dir, &segment, &out, through, author.as_deref(), strict, &rom.local()),
        Cmd::Check { core, pick, no_db, from_power_on, screenshots, shot, strict, junit } => exit_with(check(&dir, &core.local(no_db), pick.into(), from_power_on, screenshots.as_deref(), shot, strict, junit.as_deref())?),
        Cmd::Annotations { rom } => exit_with(annotations(&dir, &rom.local())?),
        Cmd::Clean { core, segment, children, accept, force } => exit_with(clean(&dir, &core.local(false), segment, children, accept, force)?),
        Cmd::Parity { cmd } => exit_with(match cmd {
            ParityCmd::Check { core, pick, compare, again, dirty, json, events, junit } => parity_cmd::check(&dir, &core.local(false), parity_cmd::CheckArgs { pick: pick.into(), compare, again, dirty, json, events, junit })?,
            ParityCmd::Report { rom, compare, limit, json } => parity_cmd::report(&dir, &rom.local(), compare.as_deref(), limit, json)?,
            ParityCmd::Rerun { ids, core, compare, json, events } => parity_cmd::rerun(&dir, &core.local(false), compare.as_deref(), &ids, json, events)?,
            ParityCmd::Clear { ids, compare } => parity_cmd::clear(&dir, compare.as_deref(), &ids)?,
            ParityCmd::Explain { id, core, compare, frames_back, json } => parity_cmd::explain(&dir, &core.local(false), compare.as_deref(), id, frames_back, json)?,
            ParityCmd::Core { core, compare, json } => parity_cmd::core(&dir, &core.local(true), compare.as_deref(), json)?,
        }),
        Cmd::Xref { what, query } => queries::xref(&dir, &what, &query),
        Cmd::Summary { segments, query } => queries::summary(&dir, segments, &query),
        Cmd::Gaps { sort, only, csv, query } => queries::gaps(&dir, sort, only, csv.as_deref(), &query),
        Cmd::Function { what, query } => queries::function(&dir, &what, &query),
        Cmd::Frontier { what, query } => queries::frontier(&dir, what.as_deref(), &query),
        Cmd::ConnectorCheck { query } => exit_with(queries::connector_check(&dir, &query)?),
        Cmd::Snapshot { segment, frame, out, core, no_db } => snapshot_cmd(&dir, &segment, frame, &out, &core.local(no_db)),
        Cmd::Sql { query } => sql(&dir, &query),
        Cmd::Worker => mcp::worker::run(dir.clone()),
        Cmd::Repin { core } => exit_with(repin(&dir, &core.local(false))?),
        Cmd::Upgrade => upgrade(&dir),
        Cmd::Mcp { allow } => mcp::serve(dir.clone(), mcp::tools::Options { allow }),
        Cmd::Cache { cmd: CacheCmd::Stats } => cache_stats(&dir),
        Cmd::Cache { cmd: CacheCmd::Gc { core, force } } => cache_gc(&dir, &core.local(false), force),
        Cmd::Cache { cmd: CacheCmd::Fill { core, segment, children, force } } => fill(&dir, &core.local(false), segment, children, force),
        Cmd::Cache { cmd: CacheCmd::Export { core, pick, compare, out } } => cache_export(&dir, &core.local(false), pick.into(), compare.as_deref(), &out),
        Cmd::Cache { cmd: CacheCmd::Seed { files, core } } => exit_with(cache_seed(&dir, &core.local(false), &files)?),
    }
}

/// Whether `d` holds a dj project: a config.toml with dj's `[rom]` and `[core]` tables
/// (another tool's config.toml is not one), or an older project's collection.toml.
fn is_project(d: &Path) -> bool {
    if d.join("collection.toml").is_file() {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(d.join("config.toml")) else { return false };
    let Ok(t) = text.parse::<toml::Table>() else { return false };
    t.get("rom").is_some_and(|v| v.is_table()) && t.get("core").is_some_and(|v| v.is_table())
}

/// The project a command works on: `-C DIR`, else the nearest folder with a project, from
/// the current one up. Other relative paths stay relative to the current folder.
fn project_dir(given: Option<&Path>) -> Result<PathBuf> {
    if let Some(d) = given {
        if !is_project(d) {
            bail!("{} isn't a dj project (no config.toml with [rom] and [core])", d.display());
        }
        return Ok(d.to_path_buf());
    }
    let here = std::env::current_dir().context("finding the current folder")?;
    for d in here.ancestors() {
        if is_project(d) {
            return Ok(if d == here { PathBuf::from(".") } else { d.to_path_buf() });
        }
    }
    bail!("not in a dj project (no config.toml here or in a folder above); run dj inside one, or pass -C DIR")
}

/// The old way of naming the project, `dj check examples/thwaite`: the first argument that
/// isn't a flag's value and is a project folder. dj init is left alone.
fn old_form(args: impl Iterator<Item = std::ffi::OsString>) -> Option<String> {
    let mut skip = false;
    for a in args {
        let a = a.to_string_lossy().into_owned();
        if std::mem::take(&mut skip) {
            continue;
        }
        if a == "-C" || a == "--project" {
            skip = true;
            continue;
        }
        if a == "init" {
            return None;
        }
        if !a.starts_with('-') && is_project(Path::new(&a)) {
            return Some(a);
        }
    }
    None
}

fn resolve_local(col: &Collection, local: &Local) -> Result<(PathBuf, PathBuf)> {
    let saved = col.settings()?;
    let rom = local.rom.clone().or(saved.rom).ok_or_else(|| anyhow!("no ROM given; pass --rom (it is then remembered) or set DJ_ROM"))?;
    let core = local.core.clone().or(saved.core).ok_or_else(|| anyhow!("no core given; pass --core (it is then remembered) or set DJ_CORE"))?;
    if local.rom.is_some() || local.core.is_some() {
        let mut saved = col.load_local()?;
        saved.rom = Some(std::fs::canonicalize(&rom).with_context(|| format!("the ROM {}", rom.display()))?);
        let lib = libretro::library_path(&core);
        saved.core = Some(std::fs::canonicalize(&lib).with_context(|| format!("the core library {}", lib.display()))?);
        col.save_local(&saved)?;
    }
    Ok((rom, core))
}

fn open_session(col: &Collection, local: &Local) -> Result<Session> {
    let (rom, core) = resolve_local(col, local)?;
    let use_db = !local.no_db && col.settings()?.database.unwrap_or(true);
    let s = Session::open_with(col, &rom, &core, use_db)?;
    if let Some(note) = &s.build_note {
        eprintln!("warning: {note}");
    }
    Ok(s)
}

/// Prints what the connector reports about its regions (docs/connectors.md), and checks
/// that the bytes it hands out match the ROM file where it says they lie.
fn print_connector_regions(core: &mut libretro::Core, rom: &Path) -> Result<()> {
    let file = std::fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    let facts = core.region_facts();
    let code = dj_core::analysis::code_region(&facts);
    println!("connector regions:");
    for (i, f) in facts.iter().enumerate() {
        let place = f.file_offset.map(|o| format!("ROM file from byte {o}")).unwrap_or_else(|| "not from the ROM file".into());
        let decoder = match f.cpu.as_deref() {
            Some(c) if dj_core::analysis::has_decoder(c) => format!("{c}"),
            Some(c) => format!("{c} (no built-in decoder: a plugin or the core can add one, docs/decoders.md)"),
            None => "no instruction set".into(),
        };
        let mark = if f.name == code { "  <- coverage rolls up here" } else { "" };
        println!("  {:<6} {:>8} bytes  {decoder}, {place}{mark}", f.name, f.size);
        if let (Some(off), Some(data)) = (f.file_offset, core.region_data(i)) {
            let off = off as usize;
            if file.get(off..off + data.len()) != Some(data) {
                eprintln!("warning: the connector's {} bytes don't match {} from byte {off}", f.name, rom.display());
            }
        }
    }
    Ok(())
}
