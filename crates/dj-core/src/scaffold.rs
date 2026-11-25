//! Making a new project (`dj init`): `propose` works out what the project would be from a
//! ROM and a core and writes nothing into the project's folder; `InitPlan::write` makes it.
//! Front ends show the plan in between.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::analysis::RegionFacts;
use crate::collection::{sha256_file, Collection, Config, CoreConfig, HashConfig, LocalConfig, PortConfig, RomConfig, SymbolsConfig, DJ_VERSION, PROJECT_FORMAT};
use crate::libretro;
use crate::platform::ConsoleProfile;

/// What the user gave. Everything but the ROM and the core has a default.
#[derive(Clone, Debug, Default)]
pub struct InitRequest {
    /// The folder to make the project in (default: one named after the ROM file, without
    /// its extension, in the current folder).
    pub dir: Option<PathBuf>,
    /// The project's name (default: the folder's name when `dir` is given, `.` being the
    /// folder it is; else the ROM file's name without its extension).
    pub name: Option<String>,
    pub rom: PathBuf,
    /// The core library, with or without its suffix (`libretro::library_path`). A bare
    /// name (`mesen_libretro`) that isn't a file here is looked for in `platform::core_dirs`.
    /// None: `$DJ_CORE`, else the core for the ROM's console (`platform::detect`), looked
    /// for in the same folders.
    pub core: Option<PathBuf>,
    /// The id of the core's build to record (default: what the core reports).
    pub core_build: Option<String>,
    /// Core options to set before the core's own values are recorded.
    pub options: BTreeMap<String, String>,
    /// A symbol map to copy into `symbols/`.
    pub symbols: Option<PathBuf>,
    /// A first root segment, from power-on.
    pub segment: Option<FirstSegment>,
}

/// The root segment a new project starts with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirstSegment {
    pub title: String,
    pub frames: u64,
}

/// What `InitPlan::write` made.
#[derive(Clone, Debug)]
pub struct Written {
    pub config: PathBuf,
    /// The first segment's id and file.
    pub segment: Option<(String, PathBuf)>,
    /// The lines added to the project's `.gitignore`.
    pub ignored: Vec<String>,
}

/// One of the connector's regions, as `dj init` reports it.
#[derive(Clone, Debug)]
pub struct RegionNote {
    pub facts: RegionFacts,
    /// Whether dj decodes the region's instruction set itself.
    pub decoder: bool,
    /// Whether coverage rolls up into this region (`analysis::code_region`).
    pub rolls_up: bool,
    /// The region says it comes from the ROM file, and its bytes aren't the file's there.
    pub differs_from_rom: bool,
}

/// A project that `write` would make.
#[derive(Clone, Debug)]
pub struct InitPlan {
    pub dir: PathBuf,
    pub rom: PathBuf,
    /// The core library's file.
    pub core: PathBuf,
    /// The ROM's console, when the core was chosen for it (no core was asked for).
    pub core_chosen_for: Option<&'static str>,
    /// The first root segment to make.
    pub segment: Option<FirstSegment>,
    /// The symbol map to copy, which `config.symbols` already names.
    pub symbols: Option<PathBuf>,
    pub config: Config,
    /// Whether the core has a coverage connector. Without one, segments play but record
    /// no coverage.
    pub connector: bool,
    /// The connector's regions (none without a connector).
    pub regions: Vec<RegionNote>,
    /// The console dj has defaults for with this core (`platform::for_core`), if any.
    /// Without one the project gets a bare joypad on port 1 and nothing console-specific.
    pub console: Option<&'static str>,
    /// What the console's defaults chose that someone making the project should know.
    pub notes: Vec<String>,
    /// The controllers the project starts with, in a few words.
    pub controllers: Controllers,
}

/// A project's controllers, counted: for "2 joypads, 8 buttons each".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controllers {
    /// The joypads in `[[ports]]`.
    pub joypads: usize,
    /// The buttons one of them shows: those the core names that the project doesn't hide,
    /// and each direction of a stick it names as a button. None when the core names none.
    pub buttons: Option<usize>,
    /// Its sticks that aren't shown as buttons.
    pub sticks: usize,
}

impl Controllers {
    fn count(ports: &[PortConfig], inputs: &[(crate::format::InputKey, String)]) -> Controllers {
        use crate::controls::{control_from_name, control_name, is_axis};
        let joypads = ports.iter().filter(|p| p.device == "JOYPAD").count();
        let Some(first) = ports.first() else { return Controllers::default() };
        let port = first.port.saturating_sub(1);
        let named: Vec<crate::format::InputKey> = inputs.iter().filter(|(k, n)| k.port == port && !n.trim().is_empty()).map(|(k, _)| *k).collect();
        let hidden: Vec<crate::format::InputKey> = first.hide.iter().filter_map(|n| control_from_name(port, n)).collect();
        // A label on one direction of an axis makes that direction a button.
        let directions: Vec<&String> = first.labels.keys().filter(|n| n.ends_with(['+', '-'])).collect();
        let as_buttons = |stick: u32| directions.iter().any(|n| control_from_name(port, n.trim_end_matches(['+', '-'])).is_some_and(|k| k.index == stick));
        let pressed = named.iter().filter(|k| k.device == crate::format::DEVICE_JOYPAD && control_name(**k).is_some() && !hidden.contains(k)).count();
        let sticks = (0..2).filter(|&stick| named.iter().any(|k| is_axis(*k) && k.index == stick && !hidden.contains(k)) && !as_buttons(stick)).count();
        Controllers { joypads, buttons: (!named.is_empty()).then_some(pressed + directions.len()), sticks }
    }

    /// "2 joypads, 8 buttons each", "1 joypad, 14 buttons and a stick".
    pub fn text(&self) -> String {
        let mut s = format!("{} joypad{}", self.joypads, if self.joypads == 1 { "" } else { "s" });
        if let Some(b) = self.buttons {
            s += &format!(", {b} button{}", if b == 1 { "" } else { "s" });
            match self.sticks {
                0 => {}
                1 => s += " and a stick",
                n => s += &format!(" and {n} sticks"),
            }
            if self.joypads > 1 {
                s += " each";
            }
        }
        s
    }
}

/// The folder a project is made in when none is named: inside the folder of what it is
/// about (a decomp's repository, or beside the ROM), where dj finds it from there and below.
pub const PROJECT_SUBDIR: &str = ".dj";

/// The name a project gets when none is given: the folder's (for `.`, the folder that
/// is), but the ROM's for a project in `.dj`.
pub fn default_name(dir: &Path, rom: &Path) -> Option<String> {
    let folder = dir.file_name().map(|n| n.to_os_string()).or_else(|| std::fs::canonicalize(dir).ok()?.file_name().map(|n| n.to_os_string()));
    folder.map(|n| n.to_string_lossy().into_owned()).filter(|n| n != PROJECT_SUBDIR).or_else(|| stem(rom))
}

/// Whether a folder already holds a project.
pub fn has_project(dir: &Path) -> bool {
    dir.join("config.toml").exists() || dir.join("collection.toml").exists()
}

fn stem(path: &Path) -> Option<String> {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty())
}

/// Loads the core with the ROM to learn what the project pins (the core's build and every
/// option, the regions to hash, the connector's regions) and returns the project it would
/// make. The core runs in a folder of its own under the system's temporary folder, so
/// nothing is written into the project's.
pub fn propose(req: &InitRequest) -> Result<InitPlan> {
    propose_with(req, crate::platform::for_core)
}

/// Looks up a core's console defaults from its name and the ROM's bytes.
pub type ProfileLookup = fn(&str, Option<&[u8]>) -> Option<&'static ConsoleProfile>;

/// `propose` with another table of console defaults (tests give the test core one).
pub fn propose_with(req: &InitRequest, profiles: ProfileLookup) -> Result<InitPlan> {
    if !req.rom.is_file() {
        bail!("the ROM {} doesn't exist", req.rom.display());
    }
    let (core_path, core_chosen_for) = find_core(req)?;
    let rom_stem = stem(&req.rom);
    let dir = match &req.dir {
        Some(d) => d.clone(),
        None => PathBuf::from(rom_stem.clone().ok_or_else(|| anyhow!("can't name a folder after {}: say which folder to make the project in", req.rom.display()))?),
    };
    if has_project(&dir) {
        bail!("{} already has a project", dir.display());
    }
    let name = match &req.name {
        Some(n) => n.clone(),
        // `.` and `..` have no name of their own: the folder they are, if it's there.
        None => req.dir.as_deref().and_then(|d| d.file_name().map(|n| n.to_os_string()).or_else(|| std::fs::canonicalize(d).ok()?.file_name().map(|n| n.to_os_string()))).map(|n| n.to_string_lossy().into_owned()).or(rom_stem).ok_or_else(|| anyhow!("can't name the project after {}: pass a name", req.rom.display()))?,
    };
    let mut syms = vec![];
    if let Some(s) = &req.symbols {
        if !s.is_file() {
            bail!("the symbol map {} doesn't exist", s.display());
        }
        let fname = s.file_name().ok_or_else(|| anyhow!("bad symbols path"))?;
        let format = crate::analysis::guess_format(s);
        syms.push(SymbolsConfig { file: format!("symbols/{}", fname.to_string_lossy()), format: format.into(), rom_header: None, region: None, cpu_base: None });
    }

    let run = |options: &BTreeMap<String, String>| {
        let work = std::env::temp_dir().join(format!("dj-init-{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()));
        let probed = probe(&core_path, &req.rom, &work, options, profiles);
        let _ = std::fs::remove_dir_all(&work);
        probed
    };
    let mut p = run(&req.options)?;
    let mut notes = vec![];
    if let Some(profile) = p.profile {
        // The console's option defaults, where the core has the option, it isn't already
        // at that value and the request doesn't set it. The core is only known once it
        // has been loaded, so they take a second load.
        let wanted: Vec<(&str, &str)> = profile.options.iter().copied().filter(|(k, v)| !req.options.contains_key(*k) && p.options.get(*k).is_some_and(|have| have != v)).collect();
        if !wanted.is_empty() {
            let mut options = req.options.clone();
            options.extend(wanted.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            p = run(&options)?;
            // A core can refuse a value (or force another): say only what took.
            let took: Vec<String> = wanted.iter().filter(|(k, v)| p.options.get(*k).is_some_and(|have| have == v)).map(|(k, v)| format!("{k} = \"{v}\"")).collect();
            if !took.is_empty() {
                notes.push(format!("{}: {}", took.join(", "), profile.options_note));
            }
        }
        if p.bus_hashed {
            notes.push(profile.hash_note.to_string());
        }
        if !profile.advice.is_empty() {
            notes.push(profile.advice.to_string());
        }
    }
    let profile = p.profile;
    // A symbol map's code is in the region coverage rolls up into (when that isn't PRG,
    // which an entry without a region means): ROM on the SNES.
    if let Some(r) = p.regions.iter().find(|r| r.rolls_up && r.facts.name != "PRG") {
        for s in syms.iter_mut().filter(|s| s.region.is_none() && s.format == "ca65-dbg") {
            s.region = Some(r.facts.name.clone());
        }
    }
    // With a console's defaults, a joypad for every port the core names inputs on (the
    // NES's two, the N64's four), so each is trimmed and named alike; else port 1 alone.
    let mut numbers: Vec<u32> = match profile {
        Some(_) => p.inputs.iter().map(|(k, _)| k.port + 1).collect::<std::collections::BTreeSet<u32>>().into_iter().collect(),
        None => vec![],
    };
    if !numbers.contains(&1) {
        numbers.insert(0, 1);
    }
    let ports: Vec<PortConfig> = numbers
        .into_iter()
        .map(|port| PortConfig {
            port,
            device: "JOYPAD".into(),
            labels: profile.map(|c| c.labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()).unwrap_or_default(),
            hide: profile.map(|c| c.hide.iter().map(|h| h.to_string()).collect()).unwrap_or_default(),
        })
        .collect();
    let controllers = Controllers::count(&ports, &p.inputs);
    let cache = profile.and_then(|c| c.cache_preset).map(|name| crate::cache::CacheConfig { preset: Some(name.to_string()), ..Default::default() });

    let config = Config {
        decoders: vec![],
        dj: Some(DJ_VERSION.to_string()),
        format: PROJECT_FORMAT,
        name,
        rom: RomConfig { sha256: sha256_file(&req.rom)?, file: req.rom.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(), shareable: None },
        core: CoreConfig { name: p.library_name.to_lowercase(), build: req.core_build.clone().unwrap_or(p.build_id), sha256: String::new(), options: p.options },
        ports,
        hash: HashConfig { algorithm: "blake3".into(), regions: p.hash_regions, exclude: vec![] },
        tick: None,
        symbols: syms,
        cache,
        compare: vec![],
    };
    Ok(InitPlan { dir, rom: req.rom.clone(), core: core_path, core_chosen_for, segment: req.segment.clone(), symbols: req.symbols.clone(), config, connector: p.connector, regions: p.regions, console: profile.map(|c| c.console), notes, controllers })
}

/// The core dj would make a ROM's project with, and whether it is on this machine.
#[derive(Clone, Debug)]
pub struct CoreSuggestion {
    /// The ROM's console (`platform::detect`), if dj can tell.
    pub console: Option<&'static str>,
    /// The core for that console.
    pub choice: Option<crate::platform::CoreChoice>,
    /// That core's file, in the first of `dirs` that has it.
    pub found: Option<PathBuf>,
    /// The folders dj looks for cores in (`platform::core_dirs`).
    pub dirs: Vec<PathBuf>,
}

impl CoreSuggestion {
    /// "an NES", "a GBA": the console with its article.
    pub fn a_console(&self) -> Option<String> {
        self.console.map(|c| format!("{} {c}", if c == "GBA" { "a" } else { "an" }))
    }

    /// The folders looked in, a line each.
    pub fn looked(&self) -> String {
        self.dirs.iter().map(|d| format!("  {}", d.display())).collect::<Vec<_>>().join("\n")
    }
}

/// The core for a ROM file, from its header or extension.
pub fn suggest_core(rom: &Path) -> Result<CoreSuggestion> {
    let mut head = vec![];
    std::io::Read::read_to_end(&mut std::io::Read::take(std::fs::File::open(rom).with_context(|| format!("reading {}", rom.display()))?, 0x10200), &mut head)?;
    let dirs = crate::platform::core_dirs();
    let console = crate::platform::detect(&head, rom);
    let choice = console.and_then(crate::platform::core_for);
    let found = choice.and_then(|c| crate::platform::find_core(c.library, &dirs));
    Ok(CoreSuggestion { console, choice, found, dirs })
}

/// The file of a core that was asked for: the path (with or without its suffix), or for a
/// name with no folder, the core of that name in one of the folders dj looks in.
pub fn resolve_core(core: &Path) -> Result<PathBuf> {
    let path = libretro::library_path(core);
    if path.is_file() {
        return Ok(path);
    }
    if core.components().count() != 1 {
        bail!("the core library {} doesn't exist", path.display());
    }
    let dirs = crate::platform::core_dirs();
    match core.to_str().and_then(|name| crate::platform::find_core(name, &dirs)) {
        Some(found) => Ok(found),
        None => bail!("the core library {} doesn't exist here, nor in any of the folders dj looks for cores in:\n{}", path.display(), dirs.iter().map(|d| format!("  {}", d.display())).collect::<Vec<_>>().join("\n")),
    }
}

/// The core's file: the one asked for, or the one for the ROM's console.
fn find_core(req: &InitRequest) -> Result<(PathBuf, Option<&'static str>)> {
    let asked = req.core.clone().or_else(|| std::env::var_os(crate::collection::CORE_ENV).filter(|c| !c.is_empty()).map(PathBuf::from));
    if let Some(core) = asked {
        return Ok((resolve_core(&core)?, None));
    }
    let s = suggest_core(&req.rom)?;
    let (Some(console), Some(choice)) = (s.console, s.choice) else {
        bail!("can't tell which console {} is for (dj knows NES, SNES, GBA and N64 ROMs by their header or extension), so it can't pick a core: name one with --core", req.rom.display());
    };
    match &s.found {
        Some(found) => Ok((found.clone(), Some(console))),
        None => bail!(
            "{} is {} ROM, which dj plays with the core {}, and that isn't in any of the folders dj looks for cores in:\n{}\nBuild it with {} (docs/install.md), or name a core's file with --core",
            req.rom.display(),
            s.a_console().unwrap_or_default(),
            libretro::library_path(Path::new(choice.library)).display(),
            s.looked(),
            choice.build
        ),
    }
}

/// A path as a `.gitignore` line that matches only it, from the folder the file is in.
fn gitignore_line(rel: &Path) -> String {
    let mut line = String::from("/");
    for c in rel.to_string_lossy().replace('\\', "/").chars() {
        if matches!(c, '*' | '?' | '[' | '\\') {
            line.push('\\');
        }
        line.push(c);
    }
    // A space at the end of a line is dropped unless it's escaped.
    if line.ends_with(' ') {
        line.insert(line.len() - 1, '\\');
    }
    line
}

/// Adds the lines `.gitignore` in `dir` doesn't have yet, keeping what is there, and
/// returns the ones it added.
fn add_to_gitignore(dir: &Path, lines: &[String]) -> Result<Vec<String>> {
    let path = dir.join(".gitignore");
    let mut text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let added: Vec<String> = lines.iter().filter(|l| !text.lines().any(|have| have.trim_end() == l.as_str())).cloned().collect();
    if added.is_empty() {
        return Ok(added);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for l in &added {
        text.push_str(l);
        text.push('\n');
    }
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(added)
}

struct Probed {
    library_name: String,
    build_id: String,
    options: BTreeMap<String, String>,
    hash_regions: Vec<String>,
    connector: bool,
    regions: Vec<RegionNote>,
    profile: Option<&'static ConsoleProfile>,
    /// What the core calls its inputs.
    inputs: Vec<(crate::format::InputKey, String)>,
    /// The profile's bus ranges are what `hash_regions` has in place of `RAM`.
    bus_hashed: bool,
}

fn probe(core_path: &Path, rom: &Path, work: &Path, options: &BTreeMap<String, String>, profiles: ProfileLookup) -> Result<Probed> {
    let mut core = libretro::Core::load(core_path, work, options)?;
    core.load_game(rom)?;
    // Some cores set up their memory on the first frame (Mupen64Plus-Next): run one so
    // the RAM regions to hash exist when they're probed.
    core.run_frame(false);
    let mut hash_regions = vec![];
    for r in ["RAM", "SRAM"] {
        if core.memory_view(libretro::region_memory_id(r).unwrap()).is_some() {
            hash_regions.push(r.to_string());
        }
    }
    let file = std::fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    let profile = profiles(&core.library_name.to_lowercase(), Some(&file));
    // The console's memory through the bus, where the core's RAM region isn't all of it
    // and the core maps both ends of every range.
    let bus: &[(u64, u64)] = profile.map_or(&[], |c| c.hash_bus);
    let bus_hashed = !bus.is_empty() && bus.iter().all(|&(a, b)| core.bus_byte(a as usize).is_some() && core.bus_byte(b as usize).is_some());
    if bus_hashed {
        hash_regions.retain(|r| r != "RAM");
        hash_regions.splice(0..0, bus.iter().map(|(a, b)| format!("BUS:0x{a:08X}-0x{b:08X}")));
    }
    let connector = core.has_connector();
    let mut regions = vec![];
    if connector {
        let facts = core.region_facts();
        let code = crate::analysis::code_region(&facts);
        for (i, f) in facts.into_iter().enumerate() {
            let differs_from_rom = match (f.file_offset, core.region_data(i)) {
                (Some(off), Some(data)) => usize::try_from(off).ok().and_then(|o| file.get(o..o.checked_add(data.len())?)) != Some(data),
                _ => false,
            };
            regions.push(RegionNote { decoder: f.cpu.as_deref().is_some_and(crate::analysis::has_decoder), rolls_up: f.name == code, differs_from_rom, facts: f });
        }
    }
    Ok(Probed { library_name: core.library_name.clone(), build_id: core.build_id(), options: core.effective_options(), hash_regions, connector, regions, profile, inputs: core.input_names(), bus_hashed })
}

impl InitPlan {
    /// Makes the project: its folders, `config.toml`, the symbol map's copy,
    /// `.local/local.toml` with where the ROM and the core are on this machine, a
    /// `.gitignore` that leaves out `.local/` and the ROM if it is in the project's folder
    /// (lines are added to one that is there, never taken away), and the first segment.
    pub fn write(&self) -> Result<Written> {
        let dir = &self.dir;
        if has_project(dir) {
            bail!("{} already has a project", dir.display());
        }
        std::fs::create_dir_all(dir.join("segments"))?;
        std::fs::create_dir_all(dir.join("blobs"))?;
        if let Some(s) = &self.symbols {
            let fname = s.file_name().ok_or_else(|| anyhow!("bad symbols path"))?;
            std::fs::create_dir_all(dir.join("symbols"))?;
            std::fs::copy(s, dir.join("symbols").join(fname)).with_context(|| format!("copying {}", s.display()))?;
        }
        let path = dir.join("config.toml");
        std::fs::write(&path, toml::to_string_pretty(&self.config)?)?;
        let mut col = Collection::open(dir)?;
        let rom = std::fs::canonicalize(&self.rom)?;
        let mut ignore = vec![".local/".to_string()];
        if let Ok(rel) = rom.strip_prefix(std::fs::canonicalize(dir)?) {
            ignore.push(gitignore_line(rel));
        }
        let ignored = add_to_gitignore(dir, &ignore)?;
        col.save_local(&LocalConfig { rom: Some(rom), core: Some(std::fs::canonicalize(&self.core)?), ..Default::default() })?;
        let segment = match &self.segment {
            Some(first) => {
                let id = uuid::Uuid::new_v4().to_string();
                col.segments.insert(id.clone(), crate::format::Segment::new(&id, None, &first.title, first.frames));
                col.save_segment(&id)?;
                // As the folder was given, not as the project resolved it.
                let file = col.paths[&id].file_name().map(|n| dir.join("segments").join(n)).unwrap_or_else(|| col.paths[&id].clone());
                Some((id, file))
            }
            None => None,
        };
        Ok(Written { config: path, segment, ignored })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::control_from_name;

    fn port(port: u32, hide: &[&str], labels: &[(&str, &str)]) -> PortConfig {
        PortConfig { port, device: "JOYPAD".into(), hide: hide.iter().map(|h| h.to_string()).collect(), labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn named(port: u32, controls: &[&str]) -> Vec<(crate::format::InputKey, String)> {
        controls.iter().map(|c| (control_from_name(port, c).unwrap(), c.to_string())).collect()
    }

    #[test]
    fn controllers_counted() {
        // Two pads, the core naming ten buttons on the first, two of them hidden.
        let buttons = ["B", "Y", "SELECT", "START", "UP", "DOWN", "LEFT", "RIGHT", "A", "X"];
        let mut inputs = named(0, &buttons);
        inputs.extend(named(1, &buttons));
        let c = Controllers::count(&[port(1, &["Y", "X", "L3"], &[]), port(2, &["Y", "X"], &[])], &inputs);
        assert_eq!(c, Controllers { joypads: 2, buttons: Some(8), sticks: 0 });
        assert_eq!(c.text(), "2 joypads, 8 buttons each");

        // One pad with a stick, and a second stick whose directions are buttons.
        let inputs = named(0, &["B", "Y", "START", "A", "LEFT X", "LEFT Y", "RIGHT X", "RIGHT Y"]);
        let labels = [("LEFT X", "Stick X"), ("RIGHT X-", "C Left"), ("RIGHT X+", "C Right"), ("RIGHT Y-", "C Up"), ("RIGHT Y+", "C Down")];
        let c = Controllers::count(&[port(1, &["A"], &labels)], &inputs);
        assert_eq!(c, Controllers { joypads: 1, buttons: Some(7), sticks: 1 });
        assert_eq!(c.text(), "1 joypad, 7 buttons and a stick");

        // A core that names nothing: the pad alone.
        assert_eq!(Controllers::count(&[port(1, &[], &[])], &[]).text(), "1 joypad");
        assert_eq!(Controllers { joypads: 2, buttons: Some(1), sticks: 2 }.text(), "2 joypads, 1 button and 2 sticks each");
    }
}
