//! `dj init`: on a terminal, asks for what a project needs one thing at a time, each with
//! a default, then shows the project and makes it. Elsewhere, or with --yes, the flags and
//! the defaults are the whole answer.

use crate::*;
use dj_core::scaffold::{self, FirstSegment, InitPlan, InitRequest};
use std::io::{BufRead, IsTerminal, Write};

/// What the flags said.
#[derive(Clone, Default)]
pub(crate) struct InitArgs {
    pub(crate) dir: Option<PathBuf>,
    pub(crate) name: Option<String>,
    pub(crate) rom: Option<PathBuf>,
    pub(crate) core: Option<PathBuf>,
    pub(crate) core_build: Option<String>,
    pub(crate) options: Vec<String>,
    pub(crate) symbols: Option<PathBuf>,
    pub(crate) no_segment: bool,
    pub(crate) yes: bool,
}

/// What the flags and the answers come to.
#[derive(Clone, Debug, PartialEq)]
struct Answers {
    rom: PathBuf,
    dir: PathBuf,
    name: Option<String>,
    core: Option<PathBuf>,
    symbols: Option<PathBuf>,
    segment: bool,
}

/// Questions on one stream, answers a line each from another.
struct Asker<R, W> {
    input: R,
    out: W,
}

impl<R: BufRead, W: Write> Asker<R, W> {
    fn say(&mut self, text: &str) -> Result<()> {
        writeln!(self.out, "{text}")?;
        Ok(())
    }

    /// One answer, trimmed: the default for an empty one. None at the end of input.
    fn ask(&mut self, question: &str, default: Option<&str>) -> Result<Option<String>> {
        match default {
            Some(d) => write!(self.out, "{question} [{d}]: ")?,
            None => write!(self.out, "{question}: ")?,
        }
        self.out.flush()?;
        let mut line = String::new();
        if self.input.read_line(&mut line)? == 0 {
            writeln!(self.out)?;
            return Ok(None);
        }
        let line = line.trim();
        Ok(Some(if line.is_empty() { default.unwrap_or("").to_string() } else { line.to_string() }))
    }

    /// A yes or a no, asked again until it is one.
    fn yes_no(&mut self, question: &str, default: bool) -> Result<Option<bool>> {
        loop {
            let Some(a) = self.ask(question, Some(if default { "Y/n" } else { "y/N" }))? else { return Ok(None) };
            match a.to_ascii_lowercase().as_str() {
                "y/n" => return Ok(Some(default)),
                "y" | "yes" => return Ok(Some(true)),
                "n" | "no" => return Ok(Some(false)),
                _ => self.say("  Answer y or n.")?,
            }
        }
    }
}

/// A path as someone types or drops it into a terminal: quotes around it, spaces escaped
/// with a backslash (Unix), `~/` for the home folder.
fn typed_path(text: &str) -> PathBuf {
    let t = text.trim();
    let quoted = t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')));
    let mut s = if quoted { t[1..t.len() - 1].to_string() } else { t.to_string() };
    if cfg!(unix) && !quoted {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            out.push(if c == '\\' { chars.next().unwrap_or('\\') } else { c });
        }
        s = out;
    }
    if let (Some(rest), Some(home)) = (s.strip_prefix("~/"), std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))) {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(s)
}

use dj_core::scaffold::default_name;

/// The flags and the defaults, with nothing asked.
fn defaults(args: &InitArgs) -> Result<Answers> {
    let rom = args.rom.clone().ok_or_else(|| anyhow!("dj init needs the game's ROM: pass --rom FILE (it asks for it only on a terminal, and not with --yes)"))?;
    let dir = args.dir.clone().unwrap_or_else(|| PathBuf::from(crate::PROJECT_SUBDIR));
    let name = args.name.clone().or_else(|| default_name(&dir, &rom));
    Ok(Answers { rom, dir, name, core: args.core.clone(), symbols: args.symbols.clone(), segment: !args.no_segment })
}

/// Asks for each thing the flags didn't say. None when the input ends first.
fn questions<R: BufRead, W: Write>(io: &mut Asker<R, W>, args: &InitArgs) -> Result<Option<Answers>> {
    io.say("A new dj project. Press Enter to take the answer in [brackets].\n")?;

    // The ROM, which the other defaults come from.
    let rom = match &args.rom {
        Some(r) => r.clone(),
        None => loop {
            let Some(a) = io.ask("ROM file", None)? else { return Ok(None) };
            if a.is_empty() {
                io.say("  dj pins a project to the game's ROM, so it needs the file.")?;
                continue;
            }
            let p = typed_path(&a);
            if p.is_file() {
                break p;
            }
            io.say(&format!("  There's no file at {}.", p.display()))?;
        },
    };
    if !rom.is_file() {
        bail!("the ROM {} doesn't exist", rom.display());
    }

    let dir = match &args.dir {
        Some(d) => d.clone(),
        None => loop {
            let Some(a) = io.ask("Project folder", Some(crate::PROJECT_SUBDIR))? else { return Ok(None) };
            let p = typed_path(&a);
            if scaffold::has_project(&p) {
                io.say(&format!("  {} already has a project.", p.display()))?;
                continue;
            }
            break p;
        },
    };

    let name = match &args.name {
        Some(n) => Some(n.clone()),
        None => {
            let default = default_name(&dir, &rom);
            loop {
                let Some(a) = io.ask("Project name", default.as_deref())? else { return Ok(None) };
                if !a.is_empty() {
                    break Some(a);
                }
            }
        }
    };

    let core = match &args.core {
        Some(c) => Some(c.clone()),
        None => {
            let from_env = std::env::var_os(dj_core::collection::CORE_ENV).filter(|c| !c.is_empty()).map(PathBuf::from);
            let s = scaffold::suggest_core(&rom)?;
            let rom_file = rom.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let default = match (&from_env, s.choice, &s.found) {
                (Some(env), _, _) => {
                    io.say(&format!("  ${} names a core.", dj_core::collection::CORE_ENV))?;
                    Some(env.clone())
                }
                (None, Some(choice), Some(found)) => {
                    io.say(&format!("  {rom_file} is {} ROM. dj suggests the {} core, which has a coverage connector.", s.a_console().unwrap_or_default(), choice.title))?;
                    Some(found.clone())
                }
                (None, Some(choice), None) => {
                    io.say(&format!(
                        "  {rom_file} is {} ROM. dj suggests the {} core, but {} isn't in any folder dj looks for cores in:\n{}\n  Build it with {} and run dj init again, or give another core's file.",
                        s.a_console().unwrap_or_default(),
                        choice.title,
                        dj_core::libretro::library_path(Path::new(choice.library)).display(),
                        s.looked(),
                        choice.build
                    ))?;
                    None
                }
                (None, None, _) => {
                    io.say("  dj can't tell which console this ROM is for (it knows NES, SNES, GBA and N64 ROMs), so it has no core to suggest. Any libretro core plays; one without a connector records no coverage.")?;
                    None
                }
            };
            let default_text = default.as_ref().map(|d| d.display().to_string());
            loop {
                let Some(a) = io.ask(if default.is_some() { "Core" } else { "Core library file" }, default_text.as_deref())? else { return Ok(None) };
                if a.is_empty() {
                    io.say("  A project needs a core to play the ROM.")?;
                    continue;
                }
                match scaffold::resolve_core(&typed_path(&a)) {
                    Ok(found) => break Some(found),
                    Err(e) => io.say(&format!("  {e:#}"))?,
                }
            }
        }
    };

    let symbols = match &args.symbols {
        Some(s) => Some(s.clone()),
        None => loop {
            let Some(a) = io.ask("Symbol map, if you have one (.dbg, .mlb, .map, ELF or CSV)", Some("none"))? else { return Ok(None) };
            if a.eq_ignore_ascii_case("none") || a.eq_ignore_ascii_case("n") || a.eq_ignore_ascii_case("no") {
                break None;
            }
            let p = typed_path(&a);
            if p.is_file() {
                break Some(p);
            }
            io.say(&format!("  There's no file at {}. Enter takes none: a map can be added to config.toml later.", p.display()))?;
        },
    };

    let segment = if args.no_segment {
        false
    } else {
        let Some(a) = io.yes_no("Make a first segment to record into (60 frames from power-on)?", true)? else { return Ok(None) };
        a
    };

    Ok(Some(Answers { rom, dir, name, core, symbols, segment }))
}

pub(crate) fn init(args: InitArgs) -> Result<i32> {
    let mut overrides = BTreeMap::new();
    for o in &args.options {
        let (k, v) = o.split_once('=').ok_or_else(|| anyhow!("--option takes KEY=VALUE"))?;
        overrides.insert(k.to_string(), v.to_string());
    }
    // Asked only of a person: with either end not a terminal (a script, CI, an agent) the
    // flags are the whole answer.
    let asking = !args.yes && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut io = Asker { input: std::io::stdin().lock(), out: std::io::stdout() };
    let a = if asking {
        match questions(&mut io, &args)? {
            Some(a) => a,
            None => return nothing_made(),
        }
    } else {
        defaults(&args)?
    };
    if asking {
        println!("\nLoading the core with the ROM...");
    }
    let plan = scaffold::propose(&InitRequest {
        dir: Some(a.dir.clone()),
        name: a.name.clone(),
        rom: a.rom.clone(),
        core: a.core.clone(),
        core_build: args.core_build.clone(),
        options: overrides,
        symbols: a.symbols.clone(),
        segment: a.segment.then(|| FirstSegment { title: "Power-on".into(), frames: 60 }),
    })?;
    if let Some(console) = plan.core_chosen_for {
        println!("core: {} (found for {} {console} ROM)", plan.core.display(), if console == "GBA" { "a" } else { "an" });
    }
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
                eprintln!("warning: the connector's {} bytes don't match {} from byte {}", f.name, a.rom.display(), f.file_offset.unwrap_or(0));
            }
        }
    }
    match plan.console {
        Some(c) => println!("console: {c} (its defaults for the {} core: docs/config.md#console-defaults)", plan.config.core.name),
        None => println!("console: no defaults for the {} core, so one joypad on port 1 with every control the core names, and RAM to hash (docs/config.md)", plan.config.core.name),
    }
    for n in &plan.notes {
        println!("  {n}");
    }
    print!("{}", init_summary(&plan));
    if asking && io.yes_no("Create it?", true)? != Some(true) {
        return nothing_made();
    }
    let made = plan.write()?;
    println!("created {}", made.config.display());
    if let (Some((_, file)), Some(first)) = (&made.segment, &plan.segment) {
        println!("created {} (a root segment from power-on, {} frames, \"{}\")", file.display(), first.frames, first.title);
    }
    if !made.ignored.is_empty() {
        println!("added to {}: {}", plan.dir.join(".gitignore").display(), made.ignored.join(", "));
    }
    let d = plan.dir.display();
    // dj finds a project in .dj from the folder it is in, and one made in `.` from there.
    let here = plan.dir == Path::new(crate::PROJECT_SUBDIR) || plan.dir == Path::new(".");
    let dj = if here { "dj".to_string() } else { format!("dj -C {d}") };
    match &made.segment {
        Some(_) => println!("next: record inputs in the GUI (dj-gui {d}), or edit the segment's file by hand (docs/segments.md); {dj} status shows the tree"),
        None => println!("next: make a root segment ({dj} new --title \"Boot\"), then record inputs in the GUI (dj-gui {d})"),
    }
    Ok(0)
}

/// The end of a `dj init` that was stopped at a question: status 1, so a command chained
/// after it doesn't run.
fn nothing_made() -> Result<i32> {
    println!("nothing was made");
    Ok(1)
}

/// The project `dj init` is about to make, a line per part.
fn init_summary(plan: &InitPlan) -> String {
    let c = &plan.config;
    let short = |s: &str| s.chars().take(12).collect::<String>();
    let mut rows: Vec<(&str, String)> = vec![
        ("folder", format!("{}{}", plan.dir.display(), if plan.dir.exists() { "" } else { " (new)" })),
        ("name", c.name.clone()),
        ("ROM", format!("{}, sha256 {}...", plan.rom.display(), short(&c.rom.sha256))),
        ("core", format!("{}, build {}, {} option{} recorded", plan.core.display(), c.core.build, c.core.options.len(), if c.core.options.len() == 1 { "" } else { "s" })),
        ("hash", c.hash.regions.join(", ")),
        ("cache", c.cache.as_ref().and_then(|c| c.preset.clone()).map_or_else(|| "the default preset".to_string(), |p| format!("preset {p}"))),
    ];
    rows.push(("input", plan.controllers.text()));
    for s in &c.symbols {
        rows.push(("symbols", format!("{} ({})", s.file, s.format)));
    }
    rows.push(("segment", match &plan.segment {
        Some(f) => format!("\"{}\", {} frames from power-on", f.title, f.frames),
        None => "none".into(),
    }));
    let mut out = String::from("\nThe project:\n");
    for (k, v) in rows {
        out += &format!("  {k:<8} {v}\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder with an NES ROM, a symbol map, and a cores folder holding a file under the
    /// NES core's name (the questions only look for the file; nothing loads it).
    fn setup(what: &str) -> (PathBuf, PathBuf, PathBuf) {
        let tmp = std::env::temp_dir().join(format!("dj-init-ask-{what}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("cores")).unwrap();
        let rom = tmp.join("My Game.nes");
        std::fs::write(&rom, b"NES\x1A\x02\x01").unwrap();
        std::fs::write(tmp.join("game.mlb"), "").unwrap();
        let core = dj_core::libretro::library_path(&tmp.join("cores").join("mesen_libretro"));
        std::fs::write(&core, "").unwrap();
        (tmp, rom, core)
    }

    fn run(args: &InitArgs, typed: &str) -> (Option<Answers>, String) {
        let mut io = Asker { input: std::io::Cursor::new(typed.as_bytes().to_vec()), out: Vec::new() };
        let a = questions(&mut io, args).unwrap();
        (a, String::from_utf8(io.out).unwrap())
    }

    /// One test: they share $DJ_CORES and $DJ_CORE.
    #[test]
    fn the_questions() {
        let (tmp, rom, core) = setup("q");
        std::env::set_var(dj_core::platform::CORES_ENV, tmp.join("cores"));
        std::env::remove_var(dj_core::collection::CORE_ENV);

        // Enter at every question: the defaults, with the core suggested for the console.
        let (a, out) = run(&InitArgs { rom: Some(rom.clone()), ..Default::default() }, "\n\n\n\n\n");
        assert_eq!(a, Some(Answers { rom: rom.clone(), dir: PathBuf::from(".dj"), name: Some("My Game".into()), core: Some(core.clone()), symbols: None, segment: true }));
        for part in ["Project folder [.dj]: ", "Project name [My Game]: ", "an NES ROM. dj suggests the Mesen core", &format!("Core [{}]: ", core.display()), "[none]: ", "[Y/n]: "] {
            assert!(out.contains(part), "{part:?} not in:\n{out}");
        }
        assert!(!out.contains("ROM file:"), "the ROM was given, so it isn't asked for:\n{out}");

        // Everything typed, with answers that are asked for again: no ROM, a ROM that isn't
        // there, a symbol map that isn't there, a yes that isn't one.
        let folder = tmp.join("my proj");
        let typed = format!("\nnope.nes\n{}\n{}\nA Game\nmesen_libretro\nnope.mlb\n{}\nmaybe\nn\n", rom.display().to_string().replace(' ', "\\ "), folder.display().to_string().replace(' ', "\\ "), tmp.join("game.mlb").display());
        let (a, out) = run(&InitArgs::default(), &typed);
        assert_eq!(a, Some(Answers { rom: rom.clone(), dir: folder.clone(), name: Some("A Game".into()), core: Some(core.clone()), symbols: Some(tmp.join("game.mlb")), segment: false }));
        for part in ["it needs the file", "There's no file at nope.nes", "Project name [my proj]: ", "There's no file at nope.mlb", "Answer y or n"] {
            assert!(out.contains(part), "{part:?} not in:\n{out}");
        }

        // What a flag says isn't asked: here only the folder and the first segment are.
        let args = InitArgs { rom: Some(rom.clone()), name: Some("N".into()), core: Some(core.clone()), symbols: Some(tmp.join("game.mlb")), ..Default::default() };
        let (a, out) = run(&args, "\nno\n");
        assert_eq!(a.map(|a| (a.dir, a.segment)), Some((PathBuf::from(".dj"), false)));
        assert!(!out.contains("Project name") && !out.contains("Core") && !out.contains("Symbol map"), "{out}");

        // A core that isn't on this machine: said, with the script that builds it, and asked for.
        std::env::set_var(dj_core::platform::CORES_ENV, tmp.join("nowhere"));
        let n64 = tmp.join("game.z64");
        std::fs::write(&n64, [0x80, 0x37, 0x12, 0x40]).unwrap();
        let (a, out) = run(&InitArgs { rom: Some(n64), ..Default::default() }, &format!("\n\n\n{}\n\n\n", core.display()));
        assert_eq!(a.and_then(|a| a.core), Some(core.clone()));
        assert!(out.contains("an N64 ROM") && out.contains("connectors/mupen64plus/build.sh") && out.contains("Core library file: ") && out.contains("needs a core"), "{out}");

        // A ROM dj doesn't know the console of.
        let other = tmp.join("game.bin");
        std::fs::write(&other, b"rom").unwrap();
        let (_, out) = run(&InitArgs { rom: Some(other), ..Default::default() }, &format!("\n\n{}\n\n\n", core.display()));
        assert!(out.contains("can't tell which console"), "{out}");

        // The input ends at a question: nothing is answered.
        let (a, _) = run(&InitArgs { rom: Some(rom.clone()), ..Default::default() }, "\n\n");
        assert_eq!(a, None);

        // A folder that has a project is asked for again.
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("config.toml"), "").unwrap();
        std::env::set_var(dj_core::platform::CORES_ENV, tmp.join("cores"));
        let (a, out) = run(&InitArgs { rom: Some(rom.clone()), ..Default::default() }, &format!("\"{}\"\nelsewhere\n\n\n\n\n", folder.display()));
        assert_eq!(a.map(|a| (a.dir, a.name)), Some((PathBuf::from("elsewhere"), Some("elsewhere".into()))));
        assert!(out.contains("already has a project"), "{out}");

        std::env::remove_var(dj_core::platform::CORES_ENV);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn with_nothing_asked() {
        // No ROM is an error that names the flag.
        let e = defaults(&InitArgs::default()).unwrap_err().to_string();
        assert!(e.contains("--rom"), "{e}");
        let rom = PathBuf::from("roms/My Game.nes");
        // .dj, named after the ROM; a folder given names the project.
        let a = defaults(&InitArgs { rom: Some(rom.clone()), ..Default::default() }).unwrap();
        assert_eq!((a.dir, a.name, a.segment), (PathBuf::from(".dj"), Some("My Game".into()), true));
        let a = defaults(&InitArgs { rom: Some(rom.clone()), dir: Some("port".into()), no_segment: true, ..Default::default() }).unwrap();
        assert_eq!((a.dir, a.name, a.segment), (PathBuf::from("port"), Some("port".into()), false));
        // `.` is the folder that is.
        let here = std::env::current_dir().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        let a = defaults(&InitArgs { rom: Some(rom), dir: Some(".".into()), ..Default::default() }).unwrap();
        assert_eq!(a.name, Some(here));
    }

    #[test]
    fn paths_as_typed() {
        assert_eq!(typed_path("  game.nes "), PathBuf::from("game.nes"));
        assert_eq!(typed_path("\"My Game.nes\""), PathBuf::from("My Game.nes"));
        assert_eq!(typed_path("'roms/My Game.nes'"), PathBuf::from("roms/My Game.nes"));
        if cfg!(unix) {
            // As a terminal writes a dropped file.
            assert_eq!(typed_path("/roms/My\\ Game\\ \\(USA\\).nes"), PathBuf::from("/roms/My Game (USA).nes"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(typed_path("~/roms/a.nes"), PathBuf::from(home).join("roms/a.nes"));
        }
    }
}
